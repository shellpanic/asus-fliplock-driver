//! Private per-user control sockets, guarded by kernel lifetime locks.
use crate::util;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
fn paths(kind: &str) -> Result<(PathBuf, PathBuf, PathBuf)> {
    if !["runtime", "session"].contains(&kind) {
        bail!("Invalid process role");
    }
    let root = util::runtime_dir()?;
    let name = if kind == "runtime" {
        "asus-numberpad".to_string()
    } else {
        format!("asus-numberpad-{kind}")
    };
    Ok((
        root.join(format!("{name}.lock")),
        root.join(format!("{name}.json")),
        root.join(format!("{name}.sock")),
    ))
}
pub struct Lock {
    pub file: File,
}
impl Lock {
    pub fn acquire(kind: &str) -> Result<Self> {
        let (path, _, _) = paths(kind)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        let info = file.metadata()?;
        if !info.is_file() || info.uid() != util::uid() {
            bail!("Unowned process lock preserved");
        }
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc < 0 {
            bail!("NumberPad {kind} is still running or an operation is in progress");
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(Self { file })
    }
}
#[derive(Debug, Serialize, Deserialize)]
struct Identity {
    pid: i32,
    uid: u32,
    exe_dev: u64,
    exe_inode: u64,
    lock_dev: u64,
    lock_inode: u64,
    kind: String,
    token: String,
    ready: bool,
}
pub struct Owner {
    lock: Lock,
    identity: Identity,
    metadata: PathBuf,
    socket: PathBuf,
    socket_inode: u64,
    pub stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    completed: bool,
}
impl Owner {
    pub fn acquire(kind: &str) -> Result<Self> {
        let lock = Lock::acquire(kind)?;
        let (_, metadata, socket) = paths(kind)?;
        if kind == "session" {
            verify_previous(&metadata)?;
        }
        if let Ok(info) = fs::symlink_metadata(&socket) {
            if info.uid() != util::uid() || !info.file_type().is_socket() {
                bail!("Unknown control endpoint preserved");
            }
            fs::remove_file(&socket)?;
        }
        if let Ok(info) = fs::symlink_metadata(&metadata)
            && (info.uid() != util::uid() || !info.is_file() || info.mode() & 0o077 != 0)
        {
            bail!("Unknown process metadata preserved");
        }
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        let socket_inode = fs::symlink_metadata(&socket)?.ino();
        listener.set_nonblocking(true)?;
        let mut bytes = [0u8; 24];
        File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let token: String = bytes.iter().map(|v| format!("{v:02x}")).collect();
        let exe = fs::metadata("/proc/self/exe")?;
        let info = lock.file.metadata()?;
        let identity = Identity {
            pid: std::process::id() as i32,
            uid: util::uid(),
            exe_dev: exe.dev(),
            exe_inode: exe.ino(),
            lock_dev: info.dev(),
            lock_inode: info.ino(),
            kind: kind.into(),
            token: token.clone(),
            ready: false,
        };
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_stop = stopping.clone();
        let peer_uid = util::uid();
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
                        if peer(&stream).is_ok_and(|cred| cred.uid == peer_uid) {
                            let mut data = Vec::new();
                            let mut one = [0u8; 1];
                            while data.len() < 128 {
                                match stream.read(&mut one) {
                                    Ok(1) => {
                                        data.push(one[0]);
                                        if one[0] == b'\n' {
                                            break;
                                        }
                                    }
                                    _ => break,
                                }
                            }
                            if data == format!("STOP {token}\n").as_bytes() {
                                thread_stop.store(true, Ordering::SeqCst);
                                let _ = stream.write_all(b"stopping\n");
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => {
                        thread_stop.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
        });
        let owner = Self {
            lock,
            identity,
            metadata,
            socket,
            socket_inode,
            stopping,
            thread: Some(thread),
            completed: false,
        };
        owner.store()?;
        Ok(owner)
    }
    fn store(&self) -> Result<()> {
        let temporary = self
            .metadata
            .with_extension(format!("{}.tmp", self.identity.token));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        let result = (|| -> Result<()> {
            serde_json::to_writer(&mut file, &self.identity)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.metadata)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
    pub fn complete(&mut self) -> Result<()> {
        write_outcome(
            &self.metadata,
            &Outcome {
                token: self.identity.token.clone(),
                success: true,
            },
        )?;
        self.completed = true;
        Ok(())
    }
    pub fn ready(&mut self) -> Result<()> {
        self.identity.ready = true;
        self.store()
    }
    pub fn stopped(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if fs::symlink_metadata(&self.socket).is_ok_and(|m| m.ino() == self.socket_inode) {
            let _ = fs::remove_file(&self.socket);
        }
        if self.identity.kind == "session" && !self.completed {
            let _ = write_outcome(
                &self.metadata,
                &Outcome {
                    token: self.identity.token.clone(),
                    success: false,
                },
            );
            return; // Retain identity so failed teardown cannot look clean.
        }
        if fs::read(&self.metadata)
            .ok()
            .and_then(|data| serde_json::from_slice::<Identity>(&data).ok())
            .is_some_and(|v| v.token == self.identity.token)
        {
            let _ = fs::remove_file(&self.metadata);
        }
        let _ = &self.lock;
    }
}
fn peer(stream: &UnixStream) -> Result<libc::ucred> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&cred) as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if rc < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(cred)
}
fn held_by(file: &File, id: &Identity) -> Result<bool> {
    let info = file.metadata()?;
    if info.dev() != id.lock_dev || info.ino() != id.lock_inode {
        return Ok(false);
    }
    for line in fs::read_to_string("/proc/locks")?.lines() {
        let f: Vec<_> = line.split_whitespace().collect();
        if f.len() < 6
            || f[1..4] != ["FLOCK", "ADVISORY", "WRITE"]
            || f[4].parse::<i32>().ok() != Some(id.pid)
        {
            continue;
        }
        let nums: Vec<_> = f[5].split(':').collect();
        if nums.len() == 3
            && u32::from_str_radix(nums[0], 16).ok() == Some(libc::major(info.dev()))
            && u32::from_str_radix(nums[1], 16).ok() == Some(libc::minor(info.dev()))
            && nums[2].parse::<u64>().ok() == Some(info.ino())
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn identity(kind: &str) -> Result<Option<(Identity, File)>> {
    let (lock, metadata, _) = paths(kind)?;
    if !lock.exists() {
        return Ok(None);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&lock)?;
    if !file.metadata()?.is_file() || file.metadata()?.uid() != util::uid() {
        bail!("Unowned process lock preserved");
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        if kind == "session" {
            verify_previous(&metadata)?;
        }
        return Ok(None);
    }
    let info = fs::symlink_metadata(&metadata).context(
        "Older or unknown driver is active; stop its foreground terminal once before upgrading",
    )?;
    if !info.is_file() || info.uid() != util::uid() || info.mode() & 0o077 != 0 {
        bail!("Unknown process identity preserved");
    }
    let id: Identity = serde_json::from_slice(&fs::read(metadata)?)?;
    if id.kind != kind || id.uid != util::uid() || id.pid <= 1 || !held_by(&file, &id)? {
        bail!("Process identity does not match the kernel lifetime lock");
    }
    let exe = fs::metadata(format!("/proc/{}/exe", id.pid))?;
    if exe.dev() != id.exe_dev
        || exe.ino() != id.exe_inode
        || fs::metadata(format!("/proc/{}", id.pid))?.uid() != id.uid
    {
        bail!("Process executable/owner changed; cleanup refused");
    }
    Ok(Some((id, file)))
}
pub fn ready(kind: &str) -> Result<bool> {
    Ok(identity(kind)?.is_some_and(|(id, _)| id.ready))
}
pub fn stop(kind: &str) -> Result<()> {
    let Some((id, file)) = identity(kind)? else {
        return Ok(());
    };
    let (_, _, socket) = paths(kind)?;
    let info = fs::symlink_metadata(&socket)?;
    if info.uid() != util::uid() || !info.file_type().is_socket() || info.mode() & 0o077 != 0 {
        bail!("Unowned control endpoint preserved");
    }
    let mut stream = UnixStream::connect(&socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let cred = peer(&stream)?;
    if cred.uid != id.uid || cred.pid != id.pid || !held_by(&file, &id)? {
        bail!("Control endpoint does not belong to the recorded driver");
    }
    stream.write_all(format!("STOP {}\n", id.token).as_bytes())?;
    let mut reply = [0u8; 9];
    stream.read_exact(&mut reply)?;
    if &reply != b"stopping\n" {
        bail!("Unexpected driver shutdown reply");
    }
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            if kind == "session" {
                let (_, metadata, _) = paths(kind)?;
                let outcome: Outcome = serde_json::from_slice(
                    &fs::read(result_path(&metadata))
                        .context("Supervisor teardown outcome missing; cleanup refused")?,
                )?;
                if outcome.token != id.token || !outcome.success {
                    bail!("Supervisor teardown failed; permission revocation and removal refused");
                }
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(30));
    }
    bail!("Driver failed to stop; permission revocation and removal refused")
}

#[derive(Serialize, Deserialize)]
struct Outcome {
    token: String,
    success: bool,
}
fn result_path(metadata: &std::path::Path) -> PathBuf {
    metadata.with_extension("result.json")
}
fn write_outcome(metadata: &std::path::Path, value: &Outcome) -> Result<()> {
    let path = result_path(metadata);
    if let Ok(info) = fs::symlink_metadata(&path)
        && (!info.is_file() || info.uid() != util::uid() || info.mode() & 0o077 != 0)
    {
        bail!("Unowned completion record preserved");
    }
    let temporary = path.with_extension(format!("{}.tmp", value.token));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        serde_json::to_writer(&mut file, value)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
fn verify_previous(metadata: &std::path::Path) -> Result<()> {
    let info = match fs::symlink_metadata(metadata) {
        Ok(info) => info,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !info.is_file() || info.uid() != util::uid() || info.mode() & 0o077 != 0 {
        bail!("Unknown prior session identity preserved");
    }
    let id: Identity = serde_json::from_slice(&fs::read(metadata)?)?;
    let path = result_path(metadata);
    let outcome: Outcome = serde_json::from_slice(
        &fs::read(path).context("Prior session ended without verified child cleanup")?,
    )?;
    if id.kind != "session"
        || id.uid != util::uid()
        || outcome.token != id.token
        || !outcome.success
    {
        bail!("Prior session cleanup is incomplete; enrollment and package retained");
    }
    Ok(())
}
