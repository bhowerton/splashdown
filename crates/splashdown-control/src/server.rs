//! The app's side: accept connections, check who's calling, read one
//! request, pass it to the handler, write the reply.

use std::io::{Read, Write};
use std::sync::Arc;

use crate::{Reply, Request, read_line, to_line};

/// Handles one request; may block (for `Listen`, until the user answers).
/// Each connection runs on its own thread.
pub type Handler = Arc<dyn Fn(Request) -> Reply + Send + Sync + 'static>;

/// Reads one request, answers it, and closes.
pub(crate) fn handle_connection<S: Read + Write>(
    mut stream: S,
    handler: &dyn Fn(Request) -> Reply,
) {
    let reply = match read_line(&mut stream) {
        Ok(Some(line)) => match Request::parse(&line) {
            Ok(request) => handler(request),
            Err(e) => Reply::error(e),
        },
        Ok(None) => return,
        Err(e) => Reply::error(format!("bad request: {e}")),
    };
    let _ = stream.write_all(to_line(&reply).as_bytes());
    let _ = stream.flush();
}

fn spawn_connection(f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new()
        .name("splashdown-control-conn".into())
        .spawn(f)
    {
        tracing::warn!("control: couldn't start a connection thread: {e}");
    }
}

#[cfg(all(unix, test))]
pub(crate) use imp::same_user;
pub use imp::serve;

#[cfg(unix)]
mod imp {
    use std::fs;
    use std::io;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::time::Duration;

    use super::{Handler, handle_connection, spawn_connection};
    use crate::Reply;

    /// A client gets this long to send its request.
    const READ_TIMEOUT: Duration = Duration::from_secs(5);

    /// Starts listening on the control socket, on a thread of its own.
    /// Fails if the socket can't be created, or another copy of the app
    /// is already listening.
    pub fn serve(handler: Handler) -> io::Result<()> {
        let path = crate::endpoint()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home folder"))?;
        let listener = bind(&path)?;
        std::thread::Builder::new()
            .name("splashdown-control".into())
            .spawn(move || accept_loop(listener, handler))?;
        Ok(())
    }

    fn accept_loop(listener: UnixListener, handler: Handler) {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("control: accept failed: {e}");
                    // Out of file descriptors, say: don't spin.
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
            };
            if !same_user(&stream) {
                tracing::warn!("control: refused a connection from another user");
                let _ = handle_refused(stream);
                continue;
            }
            let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
            let handler = handler.clone();
            spawn_connection(move || handle_connection(stream, &*handler));
        }
    }

    fn handle_refused(mut stream: UnixStream) -> io::Result<()> {
        use std::io::Write;
        stream.write_all(crate::to_line(&Reply::error("permission denied")).as_bytes())
    }

    /// Creates the socket, readable and writable by this user only. A
    /// socket left by a copy that quit is replaced; a live one isn't.
    fn bind(path: &Path) -> io::Result<UnixListener> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        match fs::symlink_metadata(path) {
            Ok(meta) => {
                if UnixStream::connect(path).is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "another copy of Splashdown is already listening",
                    ));
                }
                if !meta.file_type().is_socket() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("{} exists and isn't a socket", path.display()),
                    ));
                }
                fs::remove_file(path)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        // Bind under a temporary name and restrict it before renaming, so
        // the real name never has looser permissions. (The peer check
        // covers the temporary name.)
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        let _ = fs::remove_file(&tmp);
        let listener = UnixListener::bind(&tmp)?;
        let placed = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
            .and_then(|()| fs::rename(&tmp, path));
        if let Err(e) = placed {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(listener)
    }

    /// Whether the process at the other end runs as the same user as us.
    pub(crate) fn same_user(stream: &UnixStream) -> bool {
        // SAFETY: `geteuid` has no preconditions and can't fail.
        let me = unsafe { libc::geteuid() };
        peer_uid(stream).is_ok_and(|uid| uid == me)
    }

    #[cfg(not(target_os = "linux"))]
    fn peer_uid(stream: &UnixStream) -> io::Result<libc::uid_t> {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        // SAFETY: the descriptor is a connected Unix socket owned by
        // `stream`, which outlives the call; `uid` and `gid` are valid,
        // writable out-parameters.
        let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
        if rc == 0 {
            Ok(uid)
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(target_os = "linux")]
    fn peer_uid(stream: &UnixStream) -> io::Result<libc::uid_t> {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: the descriptor is a connected Unix socket owned by
        // `stream`; `cred` and `len` are valid out-parameters, and `len`
        // holds the size of `cred`.
        let rc = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&raw mut cred).cast(),
                &mut len,
            )
        };
        if rc == 0 {
            Ok(cred.uid)
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn socket_is_private_and_replaces_only_stale_ones() {
            let dir = std::env::temp_dir().join(format!("sd-ctl-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            let path = dir.join("control.sock");

            let listener = bind(&path).unwrap();
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            assert!(
                bind(&path).is_err(),
                "a live socket belongs to another copy"
            );

            // A socket left by a copy that quit is replaced.
            drop(listener);
            let listener = bind(&path).unwrap();
            let client = UnixStream::connect(&path).unwrap();
            let (server, _) = listener.accept().unwrap();
            assert!(same_user(&server) && same_user(&client));
            drop(listener);

            // Anything else at that path is left alone.
            fs::remove_file(&path).unwrap();
            fs::write(&path, "not a socket").unwrap();
            assert!(bind(&path).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), "not a socket");
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    use super::{Handler, handle_connection, spawn_connection};
    use crate::PIPE_NAME;

    /// Starts listening on the named pipe, on a thread of its own. Fails
    /// if another copy of the app already owns the pipe (`AddrInUse`, as
    /// on macOS).
    pub fn serve(handler: Handler) -> io::Result<()> {
        let first = create_instance(true).map_err(|e| {
            if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
                io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another copy of Splashdown is already listening",
                )
            } else {
                e
            }
        })?;
        std::thread::Builder::new()
            .name("splashdown-control".into())
            .spawn(move || accept_loop(first, handler))?;
        Ok(())
    }

    fn accept_loop(first: File, handler: Handler) {
        let mut next = Some(first);
        loop {
            let pipe = match next.take().map_or_else(|| create_instance(false), Ok) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("control: couldn't create a pipe instance: {e}");
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }
            };
            // SAFETY: `pipe` owns a valid pipe handle for the whole call; a
            // null OVERLAPPED makes this a plain blocking wait.
            let connected =
                unsafe { ConnectNamedPipe(pipe.as_raw_handle() as HANDLE, ptr::null_mut()) };
            if connected == 0
                && io::Error::last_os_error().raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32)
            {
                continue;
            }
            let handler = handler.clone();
            spawn_connection(move || {
                handle_connection(&pipe, &*handler);
                let raw = pipe.as_raw_handle() as HANDLE;
                // SAFETY: `pipe` still owns the handle; flushing lets the
                // client read the reply before we disconnect it.
                unsafe {
                    FlushFileBuffers(raw);
                    DisconnectNamedPipe(raw);
                }
            });
        }
    }

    fn create_instance(first: bool) -> io::Result<File> {
        let name: Vec<u16> = PIPE_NAME.encode_utf16().chain(Some(0)).collect();
        let mut open_mode = PIPE_ACCESS_DUPLEX;
        if first {
            // Fail if someone else already created the pipe.
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: `name` is NUL-terminated UTF-16 and outlives the call. A
        // null security descriptor gives the default DACL: full access for
        // this user, SYSTEM and administrators, read-only for everyone
        // else, so other users can't send commands. Remote clients are
        // refused by the pipe mode.
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `handle` is a valid pipe handle that nothing else owns;
        // the `File` closes it when dropped.
        Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
    }
}
