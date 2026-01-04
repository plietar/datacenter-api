use crate::binary_cache::{self, BinaryCache};

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
struct LastRevision {
    pub store_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachixPin {
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

pub async fn pxe_boot_handler(
    Path(mac): Path<String>,
    State(config): State<Config>,
) -> Json<serde_json::Value> {
    let client = reqwest::Client::new();

    let url = Url::parse("https://app.cachix.org/api/v1/cache/")
        .unwrap()
        .join(&format!("{}/", &config.pxe.cachix))
        .unwrap();

    let Some((hostname, _host)) = config.find_host_by_mac(&mac) else {
        todo!();
    };

    let hash = find_cachix_pin(&client, &url, hostname).await.unwrap();

    let caches = config
        .pxe
        .caches
        .iter()
        .map(|url| BinaryCache::new(url.clone()))
        .collect::<Vec<_>>();
    let cmdline = download_file(&client, &caches, &hash, "cmdline")
        .await
        .unwrap();

    Json(serde_json::json! ({
        "cmdline": String::from_utf8(cmdline).unwrap().trim(),
        "kernel": format!("/pxe/file/{hash}/bzImage"),
        "initrd": [format!("/pxe/file/{hash}/initrd")],
    }))
}

pub async fn pxe_file_handler(
    Path((hash, path)): Path<(String, String)>,
    State(config): State<Config>,
) -> Vec<u8> {
    let client = reqwest::Client::new();
    let caches = config
        .pxe
        .caches
        .iter()
        .map(|url| BinaryCache::new(url.clone()))
        .collect::<Vec<_>>();
    download_file(&client, &caches, &hash, &path).await.unwrap()
}
