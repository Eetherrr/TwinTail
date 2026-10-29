//! Authenticated, bounded local management transport.
//!
//! This endpoint authenticates the local OS user, not a plugin identity supplied
//! in a message. The host must still validate management methods and requests.

#[cfg(not(windows))]
use std::{io, path::Path};
use tokio::sync::oneshot;
#[cfg(not(windows))]
use tokio::sync::{mpsc, watch};
use twintail_protocol::Message;

/// A management request authenticated as the host's local OS user.
pub(crate) struct ManagementCall {
    pub request: Message,
    pub reply: oneshot::Sender<Message>,
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::{
        fs::{self, DirBuilder, Metadata},
        os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        path::{Component, PathBuf},
        time::Duration,
    };
    use tokio::{
        net::{UnixListener, UnixStream},
        task::JoinSet,
        time::timeout,
    };
    use twintail_protocol::{read_message, write_message};

    const SOCKET_NAME: &str = "management.sock";
    const MAX_FRAME: usize = 1024 * 1024;
    const MAX_CONNECTIONS: usize = 16;
    const DEADLINE: Duration = Duration::from_secs(5);

    /// A listener whose socket is removed only while its identity still matches.
    pub(crate) struct LocalServer {
        listener: UnixListener,
        path: PathBuf,
        socket_identity: (u64, u64),
        directory_identity: (u64, u64),
    }

    fn denied(message: &str) -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, message)
    }

    fn identity(metadata: &Metadata) -> (u64, u64) {
        (metadata.dev(), metadata.ino())
    }

    fn uid() -> u32 {
        rustix::process::geteuid().as_raw()
    }

    // Inspect every path component: checking only the last component would
    // allow a symlinked ancestor to redirect a privileged management client.
    fn checked_path(path: &Path) -> io::Result<PathBuf> {
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut checked = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::ParentDir => {
                    return Err(denied("management path must not contain '..'"));
                }
                Component::CurDir => continue,
                _ => checked.push(component.as_os_str()),
            }
            let metadata = fs::symlink_metadata(&checked)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(denied(
                    "management directory path contains a symlink or non-directory",
                ));
            }
            if metadata.uid() != 0 && metadata.uid() != uid() {
                return Err(denied("management ancestor has an untrusted owner"));
            }
            // Writable ancestors permit another user to rename the protected
            // directory. A root-owned sticky directory (such as /tmp) is safe.
            if metadata.mode() & 0o022 != 0
                && !(metadata.uid() == 0 && metadata.mode() & 0o1000 != 0)
            {
                return Err(denied(
                    "management directory has an unsafe writable ancestor",
                ));
            }
        }
        Ok(checked)
    }

    fn validate_directory(path: &Path) -> io::Result<(PathBuf, Metadata)> {
        let path = checked_path(path)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.uid() != uid() || metadata.mode() & 0o7777 != 0o700 {
            return Err(denied(
                "management data directory must be owned by the effective user with mode 0700",
            ));
        }
        Ok((path, metadata))
    }

    fn validate_socket(path: &Path) -> io::Result<Metadata> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != uid()
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(denied(
                "management socket must be a user-owned socket with mode 0600",
            ));
        }
        Ok(metadata)
    }

    fn validate_peer(stream: &UnixStream) -> io::Result<()> {
        if stream.peer_cred()?.uid() != uid() {
            return Err(denied(
                "management peer UID differs from the effective user",
            ));
        }
        Ok(())
    }

    async fn bounded<T>(future: impl std::future::Future<Output = io::Result<T>>) -> io::Result<T> {
        timeout(DEADLINE, future).await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "management operation exceeded five seconds",
            )
        })?
    }

    impl LocalServer {
        /// Bind a new private endpoint; an existing endpoint requires manual cleanup.
        ///
        /// Existing directories are never chmodded, and existing sockets are never
        /// removed, including sockets left by a crashed host.
        pub(crate) async fn bind(data_dir: &Path) -> io::Result<Self> {
            match fs::symlink_metadata(data_dir) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    // Validate the parent before creating the final component.
                    let parent = data_dir
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new("."));
                    checked_path(parent)?;
                    DirBuilder::new().mode(0o700).create(data_dir)?;
                }
                Err(error) => return Err(error),
            }
            let (directory, metadata) = validate_directory(data_dir)?;
            let path = directory.join(SOCKET_NAME);
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "management endpoint already exists; stop the other host or manually remove the stale endpoint",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let listener = UnixListener::bind(&path)?;
            let socket = fs::symlink_metadata(&path)?;
            // Construct the guard before chmod so an error cleans up this socket.
            let server = Self {
                listener,
                path,
                socket_identity: identity(&socket),
                directory_identity: identity(&metadata),
            };
            if let Err(error) = fs::set_permissions(&server.path, fs::Permissions::from_mode(0o600))
            {
                drop(server);
                return Err(error);
            }
            let current_directory = match validate_directory(data_dir) {
                Ok((_, metadata)) => metadata,
                Err(error) => {
                    drop(server);
                    return Err(error);
                }
            };
            if identity(&current_directory) != server.directory_identity {
                drop(server);
                return Err(denied("management directory changed during bind"));
            }
            if let Err(error) = validate_socket(&server.path) {
                drop(server);
                return Err(error);
            }
            Ok(server)
        }

        /// Serve one request per connection with bounded concurrency and waits.
        ///
        /// Shutdown (including a closed watch channel) aborts and joins every
        /// active handler before removing this server's endpoint.
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
                        let (stream, _) = match accepted {
                            Ok(accepted) => accepted,
                            Err(error) => break Err(error),
                        };
                        if validate_peer(&stream).is_ok() {
                            let tx = tx.clone();
                            handlers.spawn(async move { handle(stream, tx).await });
                        }
                    }
                }
            };
            handlers.abort_all();
            while handlers.join_next().await.is_some() {}
            result
        }
    }

    impl Drop for LocalServer {
        fn drop(&mut self) {
            let Some(parent) = self.path.parent() else {
                return;
            };
            let Ok((_, directory)) = validate_directory(parent) else {
                return;
            };
            let Ok(socket) = fs::symlink_metadata(&self.path) else {
                return;
            };
            if identity(&directory) == self.directory_identity
                && identity(&socket) == self.socket_identity
                && socket.file_type().is_socket()
                && socket.uid() == uid()
            {
                let _ = fs::remove_file(&self.path);
            }
        }
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

    async fn handle(mut stream: UnixStream, tx: mpsc::Sender<ManagementCall>) -> io::Result<()> {
        let request = bounded(read_message(&mut stream, MAX_FRAME)).await?;
        let (reply, response) = oneshot::channel();
        bounded(async {
            tx.send(ManagementCall { request, reply })
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "management dispatcher closed")
                })
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
        bounded(write_message(&mut stream, &response, MAX_FRAME)).await
    }

    /// Call the local host after checking directory, endpoint, and peer identity.
    /// Connect/write have five seconds; lifecycle responses have a 300 second ceiling.
    /// A timed out operation may have applied: query status before retrying.
    pub(crate) async fn call(data_dir: &Path, request: &Message) -> io::Result<Message> {
        let (directory, before) = validate_directory(data_dir)?;
        let path = directory.join(SOCKET_NAME);
        let socket_before = validate_socket(&path)?;
        let mut stream = bounded(UnixStream::connect(&path)).await?;
        validate_peer(&stream)?;
        let (_, after) = validate_directory(data_dir)?;
        let socket_after = validate_socket(&path)?;
        if identity(&before) != identity(&after)
            || identity(&socket_before) != identity(&socket_after)
        {
            return Err(denied("management endpoint changed while connecting"));
        }
        bounded(write_message(&mut stream, request, MAX_FRAME)).await?;
        operation_bound(read_message(&mut stream, MAX_FRAME)).await
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;
        use std::os::unix::fs::symlink;
        use tempfile::{TempDir, tempdir};
        use twintail_protocol::Outcome;

        fn private_directory() -> TempDir {
            let directory = tempdir().unwrap();
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
            directory
        }

        async fn finishes<T>(future: impl std::future::Future<Output = T>) -> T {
            timeout(Duration::from_secs(2), future)
                .await
                .expect("management test operation timed out")
        }

        #[tokio::test]
        async fn authenticated_request_receives_correlated_response() {
            let directory = private_directory();
            let server = LocalServer::bind(directory.path()).await.unwrap();
            let (tx, mut requests) = mpsc::channel(1);
            let (shutdown, receiver) = watch::channel(false);
            let serving = tokio::spawn(server.serve(tx, receiver));
            let request = Message::request("request-1", "health", json!({}));
            let expected = Message::response("request-1", Outcome::Ok(json!({"healthy": true})));
            let client = call(directory.path(), &request);
            let dispatch = async {
                let received = requests.recv().await.unwrap();
                assert_eq!(received.request, request);
                received.reply.send(expected.clone()).unwrap();
            };
            let (response, ()) = finishes(async { tokio::join!(client, dispatch) }).await;
            assert_eq!(response.unwrap(), expected);
            shutdown.send(true).unwrap();
            finishes(serving).await.unwrap().unwrap();
            assert!(!directory.path().join(SOCKET_NAME).exists());
        }

        #[tokio::test]
        async fn duplicate_bind_preserves_live_socket() {
            let directory = private_directory();
            let first = LocalServer::bind(directory.path()).await.unwrap();
            let path = directory.path().join(SOCKET_NAME);
            let original = identity(&fs::symlink_metadata(&path).unwrap());
            let error = LocalServer::bind(directory.path()).await.err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
            assert_eq!(identity(&fs::symlink_metadata(&path).unwrap()), original);
            let client = UnixStream::connect(&path).await.unwrap();
            let (accepted, _) = finishes(first.listener.accept()).await.unwrap();
            validate_peer(&client).unwrap();
            validate_peer(&accepted).unwrap();
            drop(first);
            assert!(!path.exists());
        }

        #[tokio::test]
        async fn bind_creates_private_directory_and_socket() {
            let parent = private_directory();
            let directory = parent.path().join("host");
            let server = LocalServer::bind(&directory).await.unwrap();
            assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o7777, 0o700);
            assert_eq!(
                fs::metadata(directory.join(SOCKET_NAME)).unwrap().mode() & 0o7777,
                0o600
            );
            drop(server);
            assert!(!directory.join(SOCKET_NAME).exists());
            assert!(directory.exists());
        }

        #[tokio::test]
        async fn bind_rejects_unsafe_directory_modes_without_chmod() {
            for mode in [0o755, 0o750, 0o770, 0o777, 0o1700] {
                let directory = private_directory();
                fs::set_permissions(directory.path(), fs::Permissions::from_mode(mode)).unwrap();
                let error = LocalServer::bind(directory.path()).await.err().unwrap();
                assert_eq!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied,
                    "mode {mode:o}"
                );
                assert_eq!(
                    fs::metadata(directory.path()).unwrap().mode() & 0o7777,
                    mode
                );
                assert!(!directory.path().join(SOCKET_NAME).exists());
            }
        }

        #[tokio::test]
        async fn bind_rejects_writable_ancestor() {
            let ancestor = private_directory();
            let directory = ancestor.path().join("host");
            DirBuilder::new().mode(0o700).create(&directory).unwrap();
            fs::set_permissions(ancestor.path(), fs::Permissions::from_mode(0o777)).unwrap();
            let error = LocalServer::bind(&directory).await.err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert!(!directory.join(SOCKET_NAME).exists());
        }

        #[tokio::test]
        async fn bind_rejects_symlinked_directory_and_ancestor() {
            let parent = private_directory();
            let actual = parent.path().join("actual");
            DirBuilder::new().mode(0o700).create(&actual).unwrap();
            let nested = actual.join("nested");
            DirBuilder::new().mode(0o700).create(&nested).unwrap();
            let link = parent.path().join("link");
            symlink(&actual, &link).unwrap();
            for directory in [&link, &link.join("nested")] {
                let error = LocalServer::bind(directory).await.err().unwrap();
                assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            }
            assert!(!actual.join(SOCKET_NAME).exists());
            assert!(!nested.join(SOCKET_NAME).exists());
        }

        #[tokio::test]
        async fn bind_preserves_existing_file_and_symlink() {
            let directory = private_directory();
            let path = directory.path().join(SOCKET_NAME);
            fs::write(&path, "leave intact").unwrap();
            let error = LocalServer::bind(directory.path()).await.err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
            assert_eq!(fs::read_to_string(&path).unwrap(), "leave intact");
            fs::remove_file(&path).unwrap();
            symlink(directory.path().join("missing"), &path).unwrap();
            let error = LocalServer::bind(directory.path()).await.err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
            assert!(
                fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }

        #[tokio::test]
        async fn client_rejects_unsafe_socket_modes() {
            let directory = private_directory();
            let server = LocalServer::bind(directory.path()).await.unwrap();
            let path = directory.path().join(SOCKET_NAME);
            let request = Message::request("1", "health", json!({}));
            for mode in [0o666, 0o660, 0o644, 0o400] {
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
                let error = finishes(call(directory.path(), &request))
                    .await
                    .unwrap_err();
                assert_eq!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied,
                    "mode {mode:o}"
                );
            }
            drop(server);
            assert!(!path.exists());
        }

        #[tokio::test]
        async fn client_rejects_regular_file_and_symlink_as_endpoint() {
            let directory = private_directory();
            let path = directory.path().join(SOCKET_NAME);
            let request = Message::request("1", "health", json!({}));
            fs::write(&path, "not a socket").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(
                call(directory.path(), &request).await.unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            fs::remove_file(&path).unwrap();
            let actual = directory.path().join("actual.sock");
            let _listener = UnixListener::bind(&actual).unwrap();
            fs::set_permissions(&actual, fs::Permissions::from_mode(0o600)).unwrap();
            symlink(&actual, &path).unwrap();
            assert_eq!(
                call(directory.path(), &request).await.unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }

        #[tokio::test]
        async fn cleanup_preserves_replacement_socket() {
            let directory = private_directory();
            let server = LocalServer::bind(directory.path()).await.unwrap();
            let path = directory.path().join(SOCKET_NAME);
            fs::remove_file(&path).unwrap();
            let replacement = UnixListener::bind(&path).unwrap();
            let replacement_identity = identity(&fs::symlink_metadata(&path).unwrap());
            drop(server);
            assert_eq!(
                identity(&fs::symlink_metadata(&path).unwrap()),
                replacement_identity
            );
            drop(replacement);
        }

        #[tokio::test]
        async fn shutdown_cancels_pending_reply_and_removes_socket() {
            let directory = private_directory();
            let server = LocalServer::bind(directory.path()).await.unwrap();
            let (tx, mut requests) = mpsc::channel(1);
            let (shutdown, receiver) = watch::channel(false);
            let serving = tokio::spawn(server.serve(tx, receiver));
            let mut client = UnixStream::connect(directory.path().join(SOCKET_NAME))
                .await
                .unwrap();
            write_message(
                &mut client,
                &Message::request("1", "health", json!({})),
                MAX_FRAME,
            )
            .await
            .unwrap();
            let pending = finishes(requests.recv()).await.unwrap();
            shutdown.send(true).unwrap();
            finishes(serving).await.unwrap().unwrap();
            assert!(pending.reply.is_closed());
            assert!(
                finishes(read_message(&mut client, MAX_FRAME))
                    .await
                    .is_err()
            );
            assert!(!directory.path().join(SOCKET_NAME).exists());
        }

        #[tokio::test]
        async fn closed_shutdown_channel_stops_server() {
            let directory = private_directory();
            let server = LocalServer::bind(directory.path()).await.unwrap();
            let (tx, _requests) = mpsc::channel(1);
            let (shutdown, receiver) = watch::channel(false);
            drop(shutdown);
            finishes(server.serve(tx, receiver)).await.unwrap();
            assert!(!directory.path().join(SOCKET_NAME).exists());
        }
    }
}

#[cfg(windows)]
#[path = "management_windows.rs"]
mod platform;

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::*;

    /// A fail-closed placeholder until a safe user-DACL pipe backend is available.
    pub(crate) struct LocalServer;

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "secure local management transport unavailable: Windows requires a safe current-user DACL named-pipe wrapper with first-instance locking and remote-client rejection",
        )
    }

    impl LocalServer {
        pub(crate) async fn bind(_data_dir: &Path) -> io::Result<Self> {
            Err(unsupported())
        }

        pub(crate) async fn serve(
            self,
            _tx: mpsc::Sender<ManagementCall>,
            _shutdown: watch::Receiver<bool>,
        ) -> io::Result<()> {
            Err(unsupported())
        }
    }

    pub(crate) async fn call(_data_dir: &Path, _request: &Message) -> io::Result<Message> {
        Err(unsupported())
    }
}

pub(crate) use platform::{LocalServer, call};

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::UnixStream,
        time::timeout,
    };
    use twintail_protocol::Outcome;

    fn directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }

    fn request() -> Message {
        Message::request("test", "status", serde_json::Value::Null)
    }

    #[tokio::test]
    async fn rejects_insecure_directories_and_symlinks() {
        let directory = directory();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            matches!(LocalServer::bind(directory.path()).await, Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
        );
        assert_eq!(
            call(directory.path(), &request()).await.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let link = directory.path().join("link");
        symlink(directory.path(), &link).unwrap();
        assert!(
            matches!(LocalServer::bind(&link).await, Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
        );
    }

    #[tokio::test]
    async fn duplicate_bind_preserves_socket_and_drop_preserves_replacement() {
        let directory = directory();
        let server = LocalServer::bind(directory.path()).await.unwrap();
        let path = directory.path().join("management.sock");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            matches!(LocalServer::bind(directory.path()).await, Err(error) if error.kind() == io::ErrorKind::AddrInUse)
        );
        assert!(path.exists());
        fs::remove_file(&path).unwrap();
        fs::write(&path, "replacement").unwrap();
        drop(server);
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
    }

    #[tokio::test]
    async fn response_and_shutdown_cleanup() {
        let directory = directory();
        let server = LocalServer::bind(directory.path()).await.unwrap();
        let (tx, mut rx) = mpsc::channel::<ManagementCall>(32);
        let (stop, shutdown) = watch::channel(false);
        let task = tokio::spawn(server.serve(tx, shutdown));
        let response = Message::response("test", Outcome::Ok(serde_json::Value::Null));
        let expected = response.clone();
        let dispatch = tokio::spawn(async move {
            let call = rx.recv().await.unwrap();
            assert_eq!(call.request, request());
            call.reply.send(response).unwrap();
        });
        assert_eq!(call(directory.path(), &request()).await.unwrap(), expected);
        dispatch.await.unwrap();
        stop.send(true).unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!directory.path().join("management.sock").exists());
    }

    #[tokio::test]
    async fn client_rejects_insecure_socket_and_cancelled_response_closes_connection() {
        let directory = directory();
        let server = LocalServer::bind(directory.path()).await.unwrap();
        let path = directory.path().join("management.sock");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(
            call(directory.path(), &request()).await.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let (tx, mut rx) = mpsc::channel::<ManagementCall>(32);
        let (stop, shutdown) = watch::channel(false);
        let task = tokio::spawn(server.serve(tx, shutdown));
        let dispatch = tokio::spawn(async move {
            drop(rx.recv().await.unwrap());
        });
        assert!(call(directory.path(), &request()).await.is_err());
        dispatch.await.unwrap();
        stop.send(true).unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn partial_frame_times_out_and_oversized_prefix_is_rejected() {
        let directory = directory();
        let server = LocalServer::bind(directory.path()).await.unwrap();
        let (tx, _rx) = mpsc::channel::<ManagementCall>(32);
        let (stop, shutdown) = watch::channel(false);
        let task = tokio::spawn(server.serve(tx, shutdown));
        let path = directory.path().join("management.sock");
        let mut oversized = UnixStream::connect(&path).await.unwrap();
        oversized
            .write_all(&(1024_u32 * 1024 + 1).to_be_bytes())
            .await
            .unwrap();
        let mut byte = [0];
        assert_eq!(
            timeout(Duration::from_secs(1), oversized.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        let mut partial = UnixStream::connect(&path).await.unwrap();
        partial.write_all(&[0]).await.unwrap();
        assert_eq!(
            timeout(Duration::from_secs(7), partial.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        stop.send(true).unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn shutdown_cancels_handlers_blocked_on_dispatch() {
        let directory = directory();
        let server = LocalServer::bind(directory.path()).await.unwrap();
        let (tx, _rx) = mpsc::channel::<ManagementCall>(1);
        let (reply, _response) = oneshot::channel();
        tx.send(ManagementCall {
            request: request(),
            reply,
        })
        .await
        .unwrap();
        let (stop, shutdown) = watch::channel(false);
        let task = tokio::spawn(server.serve(tx, shutdown));
        let mut stream = UnixStream::connect(directory.path().join("management.sock"))
            .await
            .unwrap();
        twintail_protocol::write_message(&mut stream, &request(), 1024 * 1024)
            .await
            .unwrap();
        tokio::task::yield_now().await;
        stop.send(true).unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!directory.path().join("management.sock").exists());
    }
}
