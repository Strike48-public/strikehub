//! Cross-platform IPC transport.
//!
//! - Unix: Unix domain sockets (`/tmp/strikehub-{id}.sock`)
//! - Windows: Named pipes (`\\.\pipe\strikehub-{id}`)

use std::path::PathBuf;

/// IPC endpoint address.
#[derive(Clone, Debug)]
pub struct IpcAddr {
    #[cfg(unix)]
    pub(crate) inner: PathBuf,
    #[cfg(windows)]
    pub(crate) inner: String,
}

impl std::fmt::Display for IpcAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(unix)]
        return write!(f, "unix://{}", self.inner.display());
        #[cfg(windows)]
        return write!(f, "pipe://{}", self.inner);
    }
}

impl IpcAddr {
    /// Generate a well-known address for a StrikeHub-managed connector.
    pub fn for_connector(id: &str) -> Self {
        #[cfg(unix)]
        return Self {
            inner: PathBuf::from(format!("/tmp/strikehub-{}.sock", id)),
        };
        #[cfg(windows)]
        return Self {
            inner: format!(r"\\.\pipe\strikehub-{}", id),
        };
    }

    /// Create from an explicit path string (custom IPC connectors or STRIKEHUB_SOCKET).
    pub fn from_string(s: &str) -> Self {
        #[cfg(unix)]
        return Self {
            inner: PathBuf::from(s),
        };
        #[cfg(windows)]
        return Self {
            inner: s.to_string(),
        };
    }

    /// Create from a Unix socket path.
    #[cfg(unix)]
    pub fn from_path(path: PathBuf) -> Self {
        Self { inner: path }
    }

    /// The string to pass as `STRIKEHUB_SOCKET` env var.
    pub fn to_env_string(&self) -> String {
        #[cfg(unix)]
        return self.inner.to_string_lossy().to_string();
        #[cfg(windows)]
        return self.inner.clone();
    }

    /// Returns `true` if the endpoint already exists on the filesystem.
    /// Always `false` on Windows (named pipes are kernel-managed).
    pub fn exists(&self) -> bool {
        #[cfg(unix)]
        return self.inner.exists();
        #[cfg(windows)]
        return false;
    }

    /// Remove the socket file (Unix) or no-op (Windows named pipes auto-cleanup).
    pub fn cleanup(&self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.inner);
        }
    }

    /// Convert to a `PathBuf` for backward compatibility with code that stores paths.
    pub fn to_path_buf(&self) -> PathBuf {
        #[cfg(unix)]
        return self.inner.clone();
        #[cfg(windows)]
        return PathBuf::from(&self.inner);
    }
}

// ---------------------------------------------------------------------------
// Client-side IPC stream
// ---------------------------------------------------------------------------

pub struct IpcStream {
    #[cfg(unix)]
    inner: tokio::net::UnixStream,
    #[cfg(windows)]
    inner: tokio::net::windows::named_pipe::NamedPipeClient,
}

impl IpcStream {
    pub async fn connect(addr: &IpcAddr) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            let inner = tokio::net::UnixStream::connect(&addr.inner).await?;
            Ok(Self { inner })
        }
        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ClientOptions;
            let inner = ClientOptions::new().open(&addr.inner)?;
            Ok(Self { inner })
        }
    }
}

impl tokio::io::AsyncRead for IpcStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for IpcStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
