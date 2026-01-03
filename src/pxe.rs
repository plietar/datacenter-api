use crate::config::Config;
use anyhow::bail;
use async_compression::tokio::bufread::ZstdDecoder;
use axum::Json;
use axum::extract::{Path, State};
use futures::TryStreamExt;
use nix_nar::Decoder;
use regex::Regex;
use serde::Deserialize;
use std::io::Read;
use tokio::io::AsyncReadExt;
use tokio_util::io::StreamReader;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct NarInfo {
    pub compression: String,
    pub nar_size: u64,
    pub file_size: u64,
    pub url: String,
}

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

async fn fetch_narinfo(client: &reqwest::Client, url: &Url, hash: &str) -> anyhow::Result<NarInfo> {
    let r = client
        .get(url.join(&format!("{hash}.narinfo"))?)
        .header("accept", "application/json")
        .send()
        .await?;

    let body: NarInfo = r.json().await?;

    Ok(body)
}

async fn fetch_nar(
    client: &reqwest::Client,
    url: &Url,
    narinfo: &NarInfo,
) -> anyhow::Result<Vec<u8>> {
    let r = client.get(url.join(&narinfo.url)?).send().await?;

    let mut data = Vec::with_capacity(narinfo.nar_size as usize);

    let reader = StreamReader::new(r.bytes_stream().map_err(|e| -> std::io::Error { panic!("{:?}", e) }));
    let mut decoder = ZstdDecoder::new(reader);
    decoder.read_to_end(&mut data).await?;

    Ok(data)
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
    url: &Url,
    hash: &str,
    path: impl Into<camino::Utf8PathBuf>,
) -> anyhow::Result<Vec<u8>> {
    let mut hash = hash.to_owned();
    let mut path = path.into();

    loop {
        println!("Downloading {hash}/{path}");

        let narinfo = fetch_narinfo(client, url, &hash).await.unwrap();
        let nar = fetch_nar(client, url, &narinfo).await.unwrap();
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
        .join(&format!("{}/", &config.pxe.cache))
        .unwrap();

    let Some((hostname, _host)) = config.find_host_by_mac(&mac) else {
        todo!();
    };

    let hash = find_cachix_pin(&client, &url, hostname).await.unwrap();
    let cmdline = download_file(&client, &url, &hash, "cmdline")
        .await
        .unwrap();

    Json(serde_json::json! ({
        "cmdline": String::from_utf8(cmdline).unwrap().trim(),
        "kernel": format!("/pxe/file/{hash}/kernel"),
        "initrd": format!("/pxe/file/{hash}/initrd"),
    }))
}

pub async fn pxe_file_handler(
    Path((hash, path)): Path<(String, String)>,
    State(config): State<Config>,
) -> Vec<u8> {
    let client = reqwest::Client::new();
    let url = Url::parse("https://app.cachix.org/api/v1/cache/")
        .unwrap()
        .join(&format!("{}/", &config.pxe.cache))
        .unwrap();
    download_file(&client, &url, &hash, &path).await.unwrap()
}
