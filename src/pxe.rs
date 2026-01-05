use crate::binary_cache::{self, BinaryCache};
use crate::config::Config;

use anyhow::{anyhow, bail};
use axum::Json;
use axum::extract::Query;
use axum::extract::Request;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum_extra::{json, response::ErasedJson};
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use hmac::{Hmac, Mac};
use http::StatusCode;
use rand::RngCore;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::io::Read;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastRevision {
    pub store_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachixPin {
    pub name: String,
    pub last_revision: LastRevision,
}

fn parse_store_path(
    path: impl AsRef<camino::Utf8Path>,
) -> anyhow::Result<(String, camino::Utf8PathBuf)> {
    let Ok(path) = path
        .as_ref()
        .strip_prefix(std::path::Path::new("/nix/store"))
    else {
        return Err(anyhow!("bad path").into());
    };

    let mut components = path.components();
    let Some(camino::Utf8Component::Normal(hashname)) = components.next() else {
        return Err(anyhow!("bad path").into());
    };

    let re = Regex::new(r"^(?<hash>[0-9a-z]{32})-[-.+_?=0-9a-zA-Z]+$").expect("regex to be valid");
    let Some(m) = re.captures(hashname) else {
        bail!("bad symlink");
    };

    let hash = m
        .name("hash")
        .expect("hash capture to exist")
        .as_str()
        .to_owned();
    let suffix = components.as_path().to_owned();

    Ok((hash, suffix))
}

async fn find_cachix_pin(
    client: &reqwest::Client,
    url: &Url,
    name: &str,
) -> anyhow::Result<String> {
    let r = client
        .get(url.join("pin")?)
        .send()
        .await?
        .error_for_status()?;

    let body: Vec<CachixPin> = r.json().await?;
    let Some(pin) = body.into_iter().find(|pin| pin.name == name) else {
        bail!("pin not found");
    };

    let (hash, _) = parse_store_path(&pin.last_revision.store_path)?;
    Ok(hash)
}

async fn download_file(
    client: &reqwest::Client,
    caches: &[BinaryCache],
    hash: &str,
    path: impl Into<camino::Utf8PathBuf>,
) -> Result<Vec<u8>, PxeError> {
    let mut hash = hash.to_owned();
    let mut path = path.into();

    loop {
        println!("Downloading {hash}/{path}");

        let mut nar = binary_cache::download(client, caches, &hash).await?;
        let mut buf = Vec::new();
        nar.read_to_end(&mut buf).await?;

        let decoder = nix_nar::Decoder::new(&buf[..])?;

        let Some(entry) = decoder
            .entries()?
            .filter_map(|e| e.ok())
            .find(|e| e.path.as_ref() == Some(&path))
        else {
            return Err(anyhow!("file does not exist").into());
        };

        match entry.content {
            nix_nar::Content::Directory => {
                return Err(anyhow!("unexpected directory").into());
            }
            nix_nar::Content::Symlink { target } => {
                (hash, path) = parse_store_path(&target)?;
            }
            nix_nar::Content::File { mut data, size, .. } => {
                let mut buffer = Vec::with_capacity(size as usize);
                data.read_to_end(&mut buffer)?;
                return Ok(buffer);
            }
        }
    }
}

struct PxeState {
    caches: Vec<BinaryCache>,
    client: reqwest::Client,
    config: Config,
    secret: [u8; 32],
}
type Pxe = Arc<PxeState>;

impl PxeState {
    fn mac_url(&self, hash: &str, path: &str) -> Hmac<Sha256> {
        let mut mac = Hmac::new_from_slice(&self.secret).expect("Creating HMAC cannot fail");
        mac.update(hash.as_bytes()); // TODO, bad
        mac.update(path.as_bytes()); // TODO, bad
        mac
    }

    fn file_url(&self, hash: &str, path: &str) -> String {
        let key = self.mac_url(hash, path).finalize().into_bytes();
        format!("/pxe/file/{hash}/{path}?key={}", URL_SAFE.encode(key))
    }

    fn verify_file_url(&self, hash: &str, path: &str, key: &str) -> anyhow::Result<()> {
        let key = URL_SAFE.decode(key)?;
        self.mac_url(hash, path).verify_slice(&key)?;
        Ok(())
    }
}

async fn handler_boot_request(
    Path(mac): Path<String>,
    State(state): State<Pxe>,
) -> Result<ErasedJson, PxeError> {
    let client = reqwest::Client::new();

    let url = Url::parse("https://app.cachix.org/api/v1/cache/")
        .unwrap()
        .join(&format!("{}/", &state.config.pxe.cachix))
        .unwrap();

    let Some((hostname, _host)) = state.config.find_host_by_mac(&mac) else {
        return Err(PxeError::UnknownHost(mac));
    };

    let hash = find_cachix_pin(&client, &url, hostname).await?;
    let cmdline = download_file(&state.client, &state.caches, &hash, "cmdline").await?;

    Ok(json! ({
        "cmdline": String::from_utf8(cmdline)?.trim(),
        "kernel": state.file_url(&hash, "bzImage"),
        "initrd": [state.file_url(&hash, "initrd")],
    }))
}

#[derive(Deserialize)]
struct KeyParam {
    key: Option<String>,
}

enum PxeError {
    InvalidAuthentication,
    UnknownHost(String),
    Internal(anyhow::Error),
}

impl<E> From<E> for PxeError
where
    E: Into<anyhow::Error>,
{
    fn from(e: E) -> Self {
        PxeError::Internal(e.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    error: String,
}

fn error(message: impl Into<String>) -> impl IntoResponse {
    return Json(ErrorDetail {
        error: message.into(),
    });
}

impl IntoResponse for PxeError {
    fn into_response(self) -> Response {
        match self {
            PxeError::InvalidAuthentication => {
                (StatusCode::BAD_REQUEST, error("key is missing or invalid")).into_response()
            }

            PxeError::UnknownHost(mac) => (
                StatusCode::NOT_FOUND,
                error(format!("no PXE configuration for MAC {mac}")),
            )
                .into_response(),

            PxeError::Internal(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Extension(Arc::new(e)),
                error("internal server error"),
            )
                .into_response(),
        }
    }
}

async fn handler_file(
    Path((hash, path)): Path<(String, String)>,
    State(state): State<Pxe>,
    Query(KeyParam { key }): Query<KeyParam>,
) -> Result<Vec<u8>, PxeError> {
    let key = key.ok_or(PxeError::InvalidAuthentication)?;
    state
        .verify_file_url(&hash, &path, &key)
        .map_err(|_| PxeError::InvalidAuthentication)?;

    let data = download_file(&state.client, &state.caches, &hash, &path).await?;
    Ok(data)
}

use axum::middleware::{Next, from_fn};
async fn log_app_errors(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    // If the response contains an AppError Extension, log it.
    if let Some(err) = response.extensions().get::<Arc<anyhow::Error>>() {
        tracing::error!(?err, "an unexpected error occurred inside a handler");
    }
    response
}

pub fn router<S>(config: Config) -> axum::Router<S> {
    use axum::routing::get;

    let mut secret = [0u8; 32];
    rand::rng().fill_bytes(&mut secret);

    let state = Pxe::new(PxeState {
        client: reqwest::Client::new(),
        caches: config
            .pxe
            .caches
            .iter()
            .map(|url| BinaryCache::new(url.clone()))
            .collect(),
        config: config.clone(),
        secret,
    });

    axum::Router::new()
        .route("/v1/boot/{mac}", get(handler_boot_request))
        .route("/file/{hash}/{*path}", get(handler_file))
        .layer(from_fn(log_app_errors))
        .with_state(state)
}
