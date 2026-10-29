//! Current-user-only Windows management pipes.
//!
//! Included by `management` as its Windows platform module. Pipe names are
//! scoped by the canonical data directory and process-token user SID. Filesystem
//! permissions are not used as authentication: both endpoints verify the peer's
//! process-token SID before exchanging any management frame.

use super::ManagementCall;
use interprocess::{
    ConnectWaitMode,
    os::windows::{
        named_pipe::{
            PipeListenerOptions,
            pipe_mode::Bytes,
            tokio::{PipeListener, PipeStream},
        },
        security_descriptor::SecurityDescriptor,
    },
};
use sha2::{Digest, Sha256};
use std::{io, num::NonZeroU8, os::windows::ffi::OsStrExt, path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
    time::{sleep, timeout},
};
use twintail_protocol::{Message, read_message, write_message};
use widestring::U16CString;
use winsafe::{HPROCESS, TokenInfo, co};

const MAX_FRAME: usize = 1024 * 1024;
const MAX_CONNECTIONS: usize = 16;
const DEADLINE: Duration = Duration::from_secs(5);
const RESPONSE_ACK: u8 = 0xa5;
type Stream = PipeStream<Bytes, Bytes>;

fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn windows_error(error: co::ERROR) -> io::Error {
    io::Error::from_raw_os_error(error.raw() as i32)
}

fn process_sid(process: &HPROCESS) -> io::Result<String> {
    let token = process
        .OpenProcessToken(co::TOKEN::QUERY)
        .map_err(windows_error)?;
    let information = token
        .GetTokenInformation(co::TOKEN_INFORMATION_CLASS::User)
        .map_err(windows_error)?;
    let TokenInfo::User(user) = information else {
        return Err(denied("Windows did not return the process token user"));
    };
    let sid = user
        .User
        .Sid()
        .ok_or_else(|| denied("process token has no user SID"))?;
    winsafe::ConvertSidToStringSid(sid).map_err(windows_error)
}

fn current_sid() -> io::Result<String> {
    process_sid(&HPROCESS::GetCurrentProcess())
}

fn validate_peer(stream: &Stream, expected_sid: &str) -> io::Result<()> {
    let pid = stream.peer_process_id()?;
    // Keep the process handle alive throughout validation. Any inaccessible,
    // exited, or otherwise unverifiable peer is rejected rather than trusted.
    let process = HPROCESS::OpenProcess(co::PROCESS::QUERY_LIMITED_INFORMATION, false, pid)
        .map_err(windows_error)?;
    if process_sid(&process)? != expected_sid || stream.peer_process_id()? != pid {
        return Err(denied("management peer SID differs from the current user"));
    }
    Ok(())
}

fn endpoint(data_dir: &Path, sid: &str) -> io::Result<String> {
    let directory = std::fs::canonicalize(data_dir)?;
    if !directory.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "management data path is not a directory",
        ));
    }
    let mut hash = Sha256::new();
    hash.update(b"TwinTail management v1\0");
    hash.update(sid.as_bytes());
    hash.update([0]);
    for unit in directory.as_os_str().encode_wide() {
        hash.update(unit.to_le_bytes());
    }
    Ok(format!(
        r"\\.\pipe\twintail-management-v1-{:x}",
        hash.finalize()
    ))
}

async fn bounded<T>(future: impl std::future::Future<Output = io::Result<T>>) -> io::Result<T> {
    timeout(DEADLINE, future).await.map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "management operation exceeded five seconds",
        )
    })?
}

async fn operation_bound<T>(
    future: impl std::future::Future<Output = io::Result<T>>,
) -> io::Result<T> {
    timeout(Duration::from_secs(300), future)
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "management operation deadline exceeded; query status before retry",
            )
        })?
}

// Interprocess normally retains dirty pipes in a background FlushFileBuffers
// pool at drop. A peer which never reads can keep that task alive indefinitely.
// Our explicit response acknowledgement replaces that unbounded flush. This
// guard also runs when an async handler is aborted during shutdown.
struct Connection(Stream);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.assume_flushed();
    }
}

/// First-instance listener with a protected DACL granting only the token user.
/// The listener always retains a pipe instance until it is dropped.
pub(crate) struct LocalServer {
    listener: PipeListener<Bytes, Bytes>,
    sid: String,
}

impl LocalServer {
    /// Bind a new endpoint, rejecting any existing pipe with the same name.
    /// The descriptor is protected from inherited ACEs and remote clients are
    /// rejected by the kernel. No existing endpoint is removed or replaced.
    pub(crate) async fn bind(data_dir: &Path) -> io::Result<Self> {
        let sid = current_sid()?;
        std::fs::create_dir_all(data_dir)?;
        let name = endpoint(data_dir, &sid)?;
        let sddl = U16CString::from_str(format!("O:{sid}D:P(A;;GA;;;{sid})"))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let descriptor = SecurityDescriptor::deserialize(&sddl)?;
        // interprocess 2.4.7's create_tokio always sets
        // FILE_FLAG_FIRST_PIPE_INSTANCE on the initial instance. accept creates
        // the replacement before handing out the connected instance.
        let listener = PipeListenerOptions::new()
            .path(name)
            .security_descriptor(Some(descriptor))
            .accept_remote(false)
            .inheritable(false)
            .instance_limit(NonZeroU8::new((MAX_CONNECTIONS + 1) as u8))
            .create_tokio_duplex::<Bytes>()?;
        Ok(Self { listener, sid })
    }

    /// Serve at most sixteen handlers. I/O has five seconds; lifecycle response waits
    /// have a 300 second ceiling. Shutdown, including watch closure, aborts and joins all
    /// handlers before releasing this listener's final pipe instance.
    pub(crate) async fn serve(
        self,
        tx: mpsc::Sender<ManagementCall>,
        mut shutdown: watch::Receiver<bool>,
    ) -> io::Result<()> {
        let mut handlers = JoinSet::new();
        let result = loop {
            if *shutdown.borrow() {
                break Ok(());
            }
            tokio::select! {
                biased;
                _ = shutdown.changed() => {
                    if *shutdown.borrow() || shutdown.has_changed().is_err() {
                        break Ok(());
                    }
                }
                _ = handlers.join_next(), if !handlers.is_empty() => {}
                accepted = self.listener.accept(), if handlers.len() < MAX_CONNECTIONS => {
                    let connection = match accepted {
                        Ok(stream) => Connection(stream),
                        Err(error) => break Err(error),
                    };
                    if validate_peer(&connection.0, &self.sid).is_ok() {
                        let tx = tx.clone();
                        handlers.spawn(async move { handle(connection, tx).await });
                    }
                }
            }
        };
        handlers.abort_all();
        while handlers.join_next().await.is_some() {}
        result
    }
}

async fn handle(mut connection: Connection, tx: mpsc::Sender<ManagementCall>) -> io::Result<()> {
    let request = bounded(read_message(&mut connection.0, MAX_FRAME)).await?;
    let (reply, response) = oneshot::channel();
    bounded(async {
        tx.send(ManagementCall { request, reply })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "management dispatcher closed"))
    })
    .await?;
    let response = operation_bound(async {
        response.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "management response cancelled",
            )
        })
    })
    .await?;
    bounded(write_message(&mut connection.0, &response, MAX_FRAME)).await?;
    let ack = bounded(connection.0.read_u8()).await?;
    if ack != RESPONSE_ACK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid management response acknowledgement",
        ));
    }
    Ok(())
}

async fn connect(name: &str) -> io::Result<Connection> {
    bounded(async {
        loop {
            // A zero timeout never enters interprocess's spawn_blocking wait
            // path, so cancelling this future leaves no outstanding wait task.
            match Stream::connect_by_path_with_wait_mode(
                name,
                ConnectWaitMode::Timeout(Duration::ZERO),
            )
            .await
            {
                Ok(stream) => return Ok(Connection(stream)),
                Err(error)
                    if error.kind() == io::ErrorKind::TimedOut
                        || error.kind() == io::ErrorKind::WouldBlock
                        || error.raw_os_error() == Some(231) =>
                {
                    sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
}

/// Call an authenticated same-user server with bounded connect/write/read waits.
/// The response is acknowledged before the server closes its pipe, preventing
/// Windows from discarding unread response bytes when the server handle closes.
pub(crate) async fn call(data_dir: &Path, request: &Message) -> io::Result<Message> {
    let sid = current_sid()?;
    let name = endpoint(data_dir, &sid)?;
    let mut connection = connect(&name).await?;
    validate_peer(&connection.0, &sid)?;
    bounded(write_message(&mut connection.0, request, MAX_FRAME)).await?;
    let response = operation_bound(read_message(&mut connection.0, MAX_FRAME)).await?;
    bounded(connection.0.write_u8(RESPONSE_ACK)).await?;
    let mut trailing = [0];
    match bounded(connection.0.read(&mut trailing)).await {
        Ok(0) => Ok(response),
        // Windows reports a closed server pipe as ERROR_BROKEN_PIPE.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(response),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected trailing management data",
        )),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use twintail_protocol::Outcome;

    fn request() -> Message {
        Message::request("windows-test", "health", serde_json::Value::Null)
    }

    async fn finishes<T>(future: impl std::future::Future<Output = T>) -> T {
        timeout(Duration::from_secs(2), future)
            .await
            .expect("Windows management test timed out")
    }

    #[tokio::test]
    async fn duplicate_bind_preserves_original_and_drop_releases_name() {
        let directory = tempfile::tempdir().unwrap();
        let first = LocalServer::bind(directory.path()).await.unwrap();
        assert!(LocalServer::bind(directory.path()).await.is_err());
        drop(first);
        let replacement = LocalServer::bind(directory.path()).await.unwrap();
        drop(replacement);
    }

    #[tokio::test]
    async fn authenticated_roundtrip_completes_acknowledgement() {
        let directory = tempfile::tempdir().unwrap();
        let server = LocalServer::bind(directory.path()).await.unwrap();
        let (tx, mut requests) = mpsc::channel(1);
        let (stop, shutdown) = watch::channel(false);
        let serving = tokio::spawn(server.serve(tx, shutdown));
        let response = Message::response("windows-test", Outcome::Ok(serde_json::Value::Null));
        let expected = response.clone();
        let dispatch = tokio::spawn(async move {
            let received = requests.recv().await.unwrap();
            assert_eq!(received.request, request());
            received.reply.send(response).unwrap();
        });
        assert_eq!(
            finishes(call(directory.path(), &request())).await.unwrap(),
            expected
        );
        dispatch.await.unwrap();
        stop.send(true).unwrap();
        finishes(serving).await.unwrap().unwrap();
        LocalServer::bind(directory.path()).await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_cancels_pending_reply_and_closes_pipe() {
        let directory = tempfile::tempdir().unwrap();
        let server = LocalServer::bind(directory.path()).await.unwrap();
        let (tx, mut requests) = mpsc::channel(1);
        let (stop, shutdown) = watch::channel(false);
        let serving = tokio::spawn(server.serve(tx, shutdown));
        let name = endpoint(directory.path(), &current_sid().unwrap()).unwrap();
        let mut connection = connect(&name).await.unwrap();
        write_message(&mut connection.0, &request(), MAX_FRAME)
            .await
            .unwrap();
        let pending = finishes(requests.recv()).await.unwrap();
        stop.send(true).unwrap();
        finishes(serving).await.unwrap().unwrap();
        assert!(pending.reply.is_closed());
        assert!(
            finishes(read_message(&mut connection.0, MAX_FRAME))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn closed_shutdown_channel_releases_listener() {
        let directory = tempfile::tempdir().unwrap();
        let server = LocalServer::bind(directory.path()).await.unwrap();
        let (tx, _requests) = mpsc::channel(1);
        let (stop, shutdown) = watch::channel(false);
        drop(stop);
        finishes(server.serve(tx, shutdown)).await.unwrap();
        LocalServer::bind(directory.path()).await.unwrap();
    }
}
