use crate::nar;
use anyhow::Context;
use std::path::PathBuf;
use tempfile::tempdir_in;
use tokio::io::AsyncRead;

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: impl Into<PathBuf>) -> Store {
        Store { path: path.into() }
    }

    pub async fn lookup(&self, hash: &str) -> anyhow::Result<Option<PathBuf>> {
        let path = self.path.join(hash);
        if path.exists() {
            Ok(Some(path))
        } else {
            Ok(None)
        }
    }

    pub async fn add(&self, hash: &str, data: impl AsyncRead) -> anyhow::Result<PathBuf> {
        let workdir = tempdir_in(&self.path)?;
        let dst = workdir.path().join(hash);

        nar::Reader::new(data)
            .extract(&dst)
            .await
            .context("Cannot extract NAR")?;

        let target = self.path.join(hash);
        tokio::fs::rename(&dst, &target).await?;

        Ok(target)
    }
}
