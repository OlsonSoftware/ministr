//! Platform-native listener for the daemon's HTTP API.
//!
//! - Unix (macOS, Linux): `tokio::net::UnixListener` over a filesystem socket.
//! - Windows: a pool-of-one [`NamedPipeServer`] under `\\.\pipe\`, rotating
//!   a fresh instance after each accepted connection so the listener is
//!   always ready to greet the next client.
//!
//! Implements [`axum::serve::Listener`], so [`axum::serve`] can drive it
//! on either platform without caring about the underlying IPC primitive.

use std::io;

use ministr_api::IpcAddr;

#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

/// Cross-platform listener backing the daemon's HTTP API.
pub struct Listener {
    inner: Inner,
}

#[cfg(unix)]
enum Inner {
    Unix(tokio::net::UnixListener),
}

#[cfg(windows)]
enum Inner {
    Pipe {
        name: String,
        /// Server instance that is currently waiting for a client.
        /// Rotated after each accepted connection: the accepted instance
        /// becomes the `Io` handed to axum, and a fresh instance is
        /// created to keep the pipe continuously listenable.
        pending: Option<NamedPipeServer>,
    },
}

impl Listener {
    /// Bind a listener to `addr`.
    ///
    /// On Windows this uses `first_pipe_instance(true)`, so if another
    /// daemon already owns the pipe name, bind fails with `AlreadyExists`
    /// — the Windows analogue of the Unix "socket file already present"
    /// check, but without a stale-file cleanup path.
    ///
    /// # Errors
    ///
    /// Fails with `Unsupported` when the [`IpcAddr`] variant doesn't
    /// match the host platform, and with whatever `io::Error` the
    /// underlying OS returns for bind failures (permission, in-use, etc.).
    pub fn bind(addr: &IpcAddr) -> io::Result<Self> {
        #[cfg(unix)]
        {
            match addr {
                IpcAddr::Unix(path) => {
                    let listener = tokio::net::UnixListener::bind(path)?;
                    Ok(Self {
                        inner: Inner::Unix(listener),
                    })
                }
                IpcAddr::NamedPipe(_) => Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "named-pipe endpoints are not supported on Unix",
                )),
            }
        }
        #[cfg(windows)]
        {
            match addr {
                IpcAddr::NamedPipe(name) => {
                    let pending = ServerOptions::new()
                        .first_pipe_instance(true)
                        .create(name)?;
                    Ok(Self {
                        inner: Inner::Pipe {
                            name: name.clone(),
                            pending: Some(pending),
                        },
                    })
                }
                IpcAddr::Unix(_) => Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "unix-socket endpoints are not supported on Windows",
                )),
            }
        }
    }
}

#[cfg(unix)]
impl axum::serve::Listener for Listener {
    type Io = tokio::net::UnixStream;
    type Addr = tokio::net::unix::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let Inner::Unix(listener) = &mut self.inner;
        loop {
            // UFCS: `tokio::net::UnixListener` also implements
            // `axum::serve::Listener` (on Linux/macOS), so `listener.accept()`
            // would resolve to the trait method and return the tuple directly.
            // We want the inherent `io::Result<_>` version so we can log and
            // retry on errors instead of panicking.
            match tokio::net::UnixListener::accept(listener).await {
                Ok(tup) => return tup,
                Err(e) => {
                    tracing::warn!(error = %e, "UDS accept error — retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        let Inner::Unix(listener) = &self.inner;
        tokio::net::UnixListener::local_addr(listener)
    }
}

#[cfg(windows)]
impl axum::serve::Listener for Listener {
    type Io = NamedPipeServer;
    type Addr = std::sync::Arc<str>;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let Inner::Pipe { name, pending } = &mut self.inner;
        loop {
            // Invariant: `pending` always holds a server instance that
            // is ready to be connected to. Taking it hands the instance
            // over to axum once a client arrives; we then create the
            // next instance before returning.
            let server = pending
                .take()
                .expect("named-pipe listener invariant: pending instance present");
            match server.connect().await {
                Ok(()) => {
                    *pending = Some(create_next_instance(name));
                    return (server, std::sync::Arc::<str>::from(name.as_str()));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "named-pipe accept error — recreating instance");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    *pending = Some(create_next_instance(name));
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        let Inner::Pipe { name, .. } = &self.inner;
        Ok(std::sync::Arc::<str>::from(name.as_str()))
    }
}

/// Build a fresh [`NamedPipeServer`] for a given pipe name, retrying
/// indefinitely if the OS is transiently out of handles.
///
/// We deliberately block this task on retries rather than propagating the
/// error: if instance creation is failing the whole daemon is wedged
/// anyway, and callers of `accept` have no reasonable way to recover —
/// far better to log and back off than to return a half-listening state.
#[cfg(windows)]
fn create_next_instance(name: &str) -> NamedPipeServer {
    loop {
        match ServerOptions::new().create(name) {
            Ok(server) => return server,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    pipe = name,
                    "failed to create next named-pipe instance — retrying in 500ms"
                );
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
}
