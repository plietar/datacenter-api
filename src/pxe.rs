use crate::binary_cache::{self, BinaryCache};
use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE, Engine as _};
        use sha2::Sha256;
        use hmac::{Hmac, Mac};

use axum::extract::Query;
use crate::config::Config;
use anyhow::bail;
use axum::Json;
use axum::extract::{Path, State};
use nix_nar::Decoder;
use regex::Regex;
use serde::Deserialize;
use std::io::Read;
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
        bail!("bad path");
    };

    let mut components = path.components();
    let Some(camino::Utf8Component::Normal(hashname)) = components.next() else {
        bail!("bad path");
    };

    let re = Regex::new(r"^(?<hash>[0-9a-z]{32})-[-.+_?=0-9a-zA-Z]+$").unwrap();
    let Some(m) = re.captures(hashname) else {
        bail!("bad symlink");
    };

    let hash = m.name("hash").unwrap().as_str().to_owned();
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
) -> anyhow::Result<Vec<u8>> {
    let mut hash = hash.to_owned();
    let mut path = path.into();

    loop {
        println!("Downloading {hash}/{path}");

        let nar = binary_cache::download(client, caches, &hash).await?;
        let decoder = Decoder::new(&nar[..]).unwrap();

        let Some(entry) = decoder
            .entries()
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.path.as_ref() == Some(&path))
        else {
            bail!("file does not exist");
        };

        match entry.content {
            nix_nar::Content::Directory => bail!("unexpected directory"),
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
    secret: Vec<u8>,
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
) -> Json<serde_json::Value> {
    let client = reqwest::Client::new();

    let url = Url::parse("https://app.cachix.org/api/v1/cache/")
        .unwrap()
        .join(&format!("{}/", &state.config.pxe.cachix))
        .unwrap();

    let Some((hostname, _host)) = state.config.find_host_by_mac(&mac) else {
        todo!();
    };

    let hash = find_cachix_pin(&client, &url, hostname).await.unwrap();

    let cmdline = download_file(&state.client, &state.caches, &hash, "cmdline")
        .await
        .unwrap();

    Json(serde_json::json! ({
        "cmdline": String::from_utf8(cmdline).unwrap().trim(),
        "kernel": state.file_url(&hash, "bzImage"),
        "initrd": state.file_url(&hash, "initrd"),
    }))
}

#[derive(Deserialize)]
struct KeyParam { key: String, }

async fn handler_file(
    Path((hash, path)): Path<(String, String)>,
    State(state): State<Pxe>,
    Query(KeyParam { key }): Query<KeyParam>,
) -> Vec<u8> {
    state.verify_file_url(&hash, &path, &key).unwrap();
    download_file(&state.client, &state.caches, &hash, &path).await.unwrap()
}

pub fn router<S>(config: Config) -> axum::Router<S> {
    use axum::routing::get;

    let state = Pxe::new(PxeState {
        client: reqwest::Client::new(),
        caches: config .pxe .caches .iter() .map(|url| BinaryCache::new(url.clone())).collect(),
        config: config.clone(),
        secret: vec![],
    });

    axum::Router::new()
        .route("/v1/boot/{mac}", get(handler_boot_request))
        .route("/file/{hash}/{*path}", get(handler_file))
        .with_state(state)
}
