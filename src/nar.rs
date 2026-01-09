use anyhow::bail;
use camino::{Utf8Path, Utf8PathBuf};
use std::pin::Pin;
use std::task::{Poll, ready};
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;

pub struct Teller<R> {
    inner: R,
    position: u64,
}

impl<R> Teller<R> {
    fn new(inner: R) -> Teller<R> {
        Teller { inner, position: 0 }
    }
}

impl<R: tokio::io::AsyncRead + Unpin> Teller<R> {
    async fn skip_to(&mut self, position: u64) -> anyhow::Result<()> {
        assert!(position >= self.position);
        let n = position - self.position;
        skip_bytes(self, n).await?;
        Ok(())
    }
}

impl<R: tokio::io::AsyncRead + Unpin> AsyncRead for Teller<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();

        let inner = Pin::new(&mut self.inner);
        ready!(inner.poll_read(cx, buf))?;

        let after = buf.filled().len();
        self.position += (after - before) as u64;

        Poll::Ready(Ok(()))
    }
}

async fn skip_bytes<R: AsyncRead + Unpin>(mut r: R, mut n: u64) -> anyhow::Result<()> {
    while n > 0 {
        let mut buf = [0u8; 4096 * 8];
        let wanted = std::cmp::min(n, buf.len() as u64);
        let actual = r.read(&mut buf[..wanted as usize]).await?;
        if actual == 0 {
            anyhow::bail!("Unexpected EOF");
        } else {
            n -= actual as u64;
        }
    }
    Ok(())
}

#[derive(Debug)]
enum State {
    Start,
    Closed,
    Object { context: Context },
    ObjectEnd { context: Context },
    Regular { offset: u64, context: Context },
    Directory { context: Context },
}

pub struct Reader<R> {
    inner: Teller<Pin<Box<R>>>,
    state: Option<State>,
}

#[derive(derive_more::Debug)]
pub struct Entry<'a, R> {
    pub path: Option<Utf8PathBuf>,
    pub contents: Contents<'a, R>,
}

#[derive(Debug)]
struct Context(Option<Utf8PathBuf>);
impl Context {
    fn new() -> Context {
        Context(None)
    }

    fn push(&mut self, name: impl AsRef<Utf8Path>) {
        match self.0 {
            Some(ref mut path) => path.push(name),
            None => self.0 = Some(name.as_ref().to_owned()),
        }
    }

    fn pop(&mut self) -> bool {
        match self.0 {
            Some(ref mut path) => {
                path.pop();
                // https://github.com/rust-lang/rust/issues/36861
                if path.as_str().is_empty() {
                    self.0 = None;
                }
                true
            }
            None => false,
        }
    }
}

#[derive(derive_more::Debug)]
#[allow(dead_code)]
pub enum Contents<'a, R> {
    Regular {
        executable: bool,
        size: u64,
        #[debug(skip)]
        data: tokio::io::Take<&'a mut Teller<Pin<Box<R>>>>,
    },
    Symlink {
        target: String,
    },
    Directory,
}

impl<R: AsyncRead> Reader<R> {
    pub fn new(r: R) -> Reader<R> {
        Reader {
            inner: Teller::new(Box::pin(r)),
            state: Some(State::Start),
        }
    }

    async fn read_str(&mut self) -> anyhow::Result<String> {
        let n = self.inner.read_u64_le().await?;
        assert!(n < 4096);
        let mut buf = vec![0u8; n.next_multiple_of(8) as usize];
        self.inner.read_exact(&mut buf).await?;
        buf.resize(n as usize, 0);

        Ok(String::from_utf8(buf)?)
    }

    async fn expect_str(&mut self, expected: &str) -> anyhow::Result<()> {
        let actual = self.read_str().await?;
        if actual != expected {
            bail!("expected '{}' got '{}'", expected, actual);
        }
        Ok(())
    }

    async fn regular_header(&mut self) -> anyhow::Result<(bool, u64)> {
        let mut executable = false;
        let mut s = self.read_str().await?;
        if s == "executable" {
            executable = true;
            self.expect_str("").await?;
            s = self.read_str().await?;
        }
        if s != "contents" {
            bail!("expected '{}', got '{}'", "contents", s);
        }

        let size = self.inner.read_u64_le().await?;
        Ok((executable, size))
    }

    pub async fn next<'a>(&'a mut self) -> anyhow::Result<Option<Entry<'a, R>>> {
        loop {
            match self.state.take().expect("missing state") {
                State::Start => {
                    self.expect_str("nix-archive-1").await?;
                    self.state = Some(State::Object {
                        context: Context::new(),
                    });
                }
                State::Object { context } => {
                    let path = context.0.clone();
                    self.expect_str("(").await?;
                    self.expect_str("type").await?;
                    let t = self.read_str().await?;
                    match t.as_ref() {
                        "regular" => {
                            let (executable, size) = self.regular_header().await?;
                            let offset = self.inner.position + size.next_multiple_of(8);
                            let data = (&mut self.inner).take(size);

                            self.state = Some(State::Regular { offset, context });
                            return Ok(Some(Entry {
                                path,
                                contents: Contents::Regular {
                                    executable,
                                    size,
                                    data,
                                },
                            }));
                        }
                        "directory" => {
                            self.state = Some(State::Directory { context });
                            return Ok(Some(Entry {
                                path,
                                contents: Contents::Directory,
                            }));
                        }
                        "symlink" => {
                            self.expect_str("target").await?;
                            let target = self.read_str().await?;
                            self.expect_str(")").await?;
                            self.state = Some(State::ObjectEnd { context });
                            return Ok(Some(Entry {
                                path,
                                contents: Contents::Symlink { target },
                            }));
                        }
                        t => bail!("invalid entry type: {}", t),
                    }
                }
                State::Regular { context, offset } => {
                    self.inner.skip_to(offset).await?;
                    self.expect_str(")").await?;
                    self.state = Some(State::ObjectEnd { context });
                }
                State::Directory { mut context } => {
                    let s = self.read_str().await?;
                    if s == "entry" {
                        self.expect_str("(").await?;
                        self.expect_str("name").await?;
                        let name = self.read_str().await?;
                        self.expect_str("node").await?;
                        context.push(name);
                        self.state = Some(State::Object { context })
                    } else if s == ")" {
                        self.state = Some(State::ObjectEnd { context });
                    } else {
                        bail!("expected '{}' or ')', got '{}'", "entry", s);
                    }
                }
                State::ObjectEnd { mut context } => {
                    if context.pop() {
                        self.expect_str(")").await?;
                        self.state = Some(State::Directory { context });
                    } else {
                        self.state = Some(State::Closed);
                    }
                }
                State::Closed => return Ok(None),
            }
        }
    }
}

pub async fn find<'a, R: tokio::io::AsyncRead>(
    reader: &'a mut Reader<R>,
    path: &Utf8Path,
) -> anyhow::Result<Option<Entry<'a, R>>> {
    loop {
        // https://github.com/danielhenrymantilla/polonius-the-crab.rs/issues/15
        let reader: &mut Reader<R> = unsafe { &mut *(reader as *mut _) };
        let Some(entry) = reader.next().await? else {
            return Ok(None);
        };
        if entry.path.as_deref() == Some(path) {
            return Ok(Some(entry));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::process::Stdio;
    use tempdir::TempDir;
    use tokio::process::Command;
    use anyhow::Context as _;

    async fn create_nar(path: impl AsRef<OsStr>) -> anyhow::Result<impl AsyncRead> {
        let child = Command::new("nix")
            .arg("nar")
            .arg("pack")
            .args(["--extra-experimental-features", "nix-command"])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .context("Could not run nix command")?;
        Ok(child.stdout.unwrap())
    }

    async fn enumerate_nar(
        stream: impl AsyncRead,
    ) -> anyhow::Result<Vec<(Option<Utf8PathBuf>, char)>> {
        let mut reader = Reader::new(stream);
        let mut result = vec![];
        while let Some(entry) = reader.next().await? {
            let c = match entry.contents {
                Contents::Regular { .. } => 'f',
                Contents::Symlink { .. } => 'l',
                Contents::Directory { .. } => 'd',
            };
            result.push((entry.path, c));
        }
        Ok(result)
    }

    #[tokio::test]
    async fn nar_file() -> anyhow::Result<()> {
        let root = TempDir::new("root")?;
        std::fs::write(root.path().join("hello.txt"), "hello").unwrap();

        let stream = create_nar(root.path().join("hello.txt")).await?;
        let result = enumerate_nar(stream).await?;

        assert_eq!(result, vec![(None, 'f')]);

        Ok(())
    }

    #[tokio::test]
    async fn nar_symlink() -> anyhow::Result<()> {
        let root = TempDir::new("root")?;
        std::os::unix::fs::symlink("/foobar", root.path().join("hello.txt"))?;

        let stream = create_nar(root.path().join("hello.txt")).await?;
        let result = enumerate_nar(stream).await?;

        assert_eq!(result, vec![(None, 'l')]);

        Ok(())
    }

    #[tokio::test]
    async fn nar_empty_directory() -> anyhow::Result<()> {
        let root = TempDir::new("root")?;
        let stream = create_nar(root.path()).await?;
        let result = enumerate_nar(stream).await?;

        assert_eq!(result, vec![(None, 'd')]);

        Ok(())
    }

    #[tokio::test]
    async fn nar_directory() -> anyhow::Result<()> {
        let root = TempDir::new("root")?;
        std::fs::write(root.path().join("hello.txt"), "Hello")?;
        std::fs::create_dir(root.path().join("nested"))?;
        std::fs::write(root.path().join("nested/world.txt"), "World")?;

        let stream = create_nar(root.path()).await?;
        let result = enumerate_nar(stream).await?;

        assert_eq!(
            result,
            vec![
                (None, 'd'),
                (Some(Utf8PathBuf::from("hello.txt")), 'f'),
                (Some(Utf8PathBuf::from("nested")), 'd'),
                (Some(Utf8PathBuf::from("nested/world.txt")), 'f')
            ]
        );

        Ok(())
    }
}
