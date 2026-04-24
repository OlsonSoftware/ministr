//! Platform-native IPC transport for the daemon client.
//!
//! - Unix (macOS, Linux): `tokio::net::UnixStream` over a filesystem socket.
//! - Windows: `tokio::net::windows::named_pipe::NamedPipeClient` over a
//!   named pipe under `\\.\pipe\`.
//!
//! Both stream types implement `AsyncRead + AsyncWrite + Unpin`, so the
//! rest of `ministr-api` can drive the stream generically. Callers that
//! need platform-agnostic code should depend on [`Stream`] via its trait
//! impls rather than on the underlying type.

use std::io;

use crate::IpcAddr;

/// Platform-native bidirectional stream to the daemon.
///
/// `tokio::net::UnixStream` on Unix; `NamedPipeClient` on Windows. Both
/// implement `tokio::io::AsyncRead + AsyncWrite + Unpin + Send`.
#[cfg(unix)]
pub type Stream = tokio::net::UnixStream;

/// Platform-native bidirectional stream to the daemon.
#[cfg(windows)]
pub type Stream = tokio::net::windows::named_pipe::NamedPipeClient;

/// Open a new client stream to the daemon at `addr`.
///
/// On Windows, transparently retries `ERROR_PIPE_BUSY` (all named-pipe
/// instances currently attached) so a flurry of concurrent clients
/// doesn't surface a spurious failure while the server rotates fresh
/// instances in.
///
/// # Errors
///
/// - `NotFound` if no daemon is listening at the endpoint.
/// - `Unsupported` if `addr` is a variant that isn't valid on this
///   platform (e.g. `IpcAddr::Unix` on Windows).
/// - `WouldBlock` (Windows only) if every named-pipe instance remains
///   busy after the internal retry budget is exhausted.
/// - Any other `io::Error` surfaced by the OS (permissions, etc.).
pub async fn connect(addr: &IpcAddr) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        match addr {
            IpcAddr::Unix(path) => tokio::net::UnixStream::connect(path).await,
            IpcAddr::NamedPipe(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "named-pipe endpoints are not supported on Unix",
            )),
        }
    }
    #[cfg(windows)]
    {
        match addr {
            IpcAddr::NamedPipe(name) => open_named_pipe_with_busy_retry(name).await,
            IpcAddr::Unix(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unix-socket endpoints are not supported on Windows",
            )),
        }
    }
}

/// `ERROR_PIPE_BUSY` — raised when every pipe instance has an attached
/// client and no pending instance is available for a new connection.
/// Canonical Windows pattern is for the client to wait briefly and retry
/// rather than fail the whole operation; the server is expected to
/// continuously rotate fresh instances in.
#[cfg(windows)]
const ERROR_PIPE_BUSY: i32 = 231;

/// Inner-loop retry for the Windows named-pipe open path.
///
/// Caps total wait at roughly 2.5s (50ms × 50 attempts) so a genuinely
/// missing daemon still fails fast at the client layer instead of
/// stalling on endless busy retries.
#[cfg(windows)]
async fn open_named_pipe_with_busy_retry(name: &str) -> io::Result<Stream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const BUSY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);
    const BUSY_MAX_ATTEMPTS: usize = 50;

    for _ in 0..BUSY_MAX_ATTEMPTS {
        match ClientOptions::new().open(name) {
            Ok(client) => return Ok(client),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(BUSY_BACKOFF).await;
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "all named-pipe instances remained busy after retries",
    ))
}
