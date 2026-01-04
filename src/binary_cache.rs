use anyhow::anyhow;
use async_compression::tokio::bufread::{XzDecoder, ZstdDecoder};
use futures::TryStreamExt as _;
use serde::Deserialize;
use std::collections::HashMap;
use tokio::io::AsyncReadExt as _;
use tokio_util::io::StreamReader;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct NarInfo {
    pub compression: String,
    pub nar_size: u64,
    pub file_size: u64,
    pub url: String,
}

impl NarInfo {
    pub fn parse(s: &str) -> anyhow::Result<NarInfo> {
        let fields: HashMap<_, _> = s
            .lines()
            .map(|l| l.split_once(": ").ok_or_else(|| anyhow!("Invalid line")))
            .collect::<Result<_, _>>()?;

        Ok(NarInfo {
            url: fields
                .get("URL")
                .ok_or_else(|| anyhow!("Missing URL field"))?
                .to_string(),
            nar_size: fields
                .get("NarSize")
                .ok_or_else(|| anyhow!("Missing NarSize field"))?
                .parse()?,
            file_size: fields
                .get("FileSize")
                .ok_or_else(|| anyhow!("Missing FileSize field"))?
                .parse()?,
            compression: fields
                .get("Compression")
                .ok_or_else(|| anyhow!("Missing Compression field"))?
                .to_string(),
        })
    }
}

pub struct BinaryCache {
    url: Url,
}

impl BinaryCache {
    pub fn new(url: Url) -> BinaryCache {
        BinaryCache { url }
    }

    pub async fn fetch_narinfo(
        &self,
        client: &reqwest::Client,
        hash: &str,
    ) -> anyhow::Result<NarInfo> {
        let r = client
            .get(self.url.join(&format!("{hash}.narinfo"))?)
            .header("accept", "text/x-nix-narinfo")
            .send()
            .await?;
        r.error_for_status_ref()?;

        Ok(NarInfo::parse(&r.text().await?)?)
    }

    pub async fn fetch_nar(
        &self,
        client: &reqwest::Client,
        narinfo: &NarInfo,
    ) -> anyhow::Result<Vec<u8>> {
        let r = client.get(self.url.join(&narinfo.url)?).send().await?;
        r.error_for_status_ref()?;

        let mut data = Vec::with_capacity(narinfo.nar_size as usize);

        let mut reader = StreamReader::new(
            r.bytes_stream()
                .map_err(|e| -> std::io::Error { panic!("{:?}", e) }),
        );

        match narinfo.compression.as_str() {
            "none" => {
                reader.read_to_end(&mut data).await?;
            }
            "xz" => {
                let mut decoder = XzDecoder::new(reader);
                decoder.read_to_end(&mut data).await?;
            }
            "zstd" => {
                let mut decoder = ZstdDecoder::new(reader);
                decoder.read_to_end(&mut data).await?;
            }
            "bzip2" | "gzip" => anyhow::bail!(
                "Compression method {} is not implemented yet",
                narinfo.compression
            ),
            _ => {
                anyhow::bail!("Unsupported compression type: {}", narinfo.compression);
            }
        }

        Ok(data)
    }

    pub async fn download(&self, client: &reqwest::Client, hash: &str) -> anyhow::Result<Vec<u8>> {
        println!("Downloading {hash} from {}", self.url);

        let narinfo = self.fetch_narinfo(client, hash).await?;
        let result = self.fetch_nar(client, &narinfo).await?;
        Ok(result)
    }
}

pub async fn download(
    client: &reqwest::Client,
    caches: &[BinaryCache],
    hash: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut error = anyhow::anyhow!("No configured binary cache");
    for c in caches {
        match c.download(client, hash).await {
            Ok(result) => return Ok(result),
            Err(err) => {
                error = err;
            }
        }
    }
    Err(error)
}
