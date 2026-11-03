#![cfg(feature = "test-fixtures")]
use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{FileTypeExt, PermissionsExt},
            net::UnixStream,
        },
    },
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}
fn spawn(root: &Path, role: &str, extra: &[&str]) -> Process {
    Process(
        Command::new(env!("CARGO_BIN_EXE_asus-numberpad"))
            .args(["_fixture", role])
            .args(extra)
            .env("XDG_RUNTIME_DIR", root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    )
}
fn name(role: &str) -> &str {
    if role == "runtime" {
        "asus-numberpad"
    } else {
        "asus-numberpad-session"
    }
}
fn wait(root: &Path, role: &str, child: &mut Process) -> serde_json::Value {
    let path = root.join(format!("{}.json", name(role)));
    let until = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "fixture exited before ready"
        );
        assert!(Instant::now() < until, "fixture did not start");
        thread::sleep(Duration::from_millis(10));
    }
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn stop(root: &Path, role: &str) -> ExitStatus {
    Command::new(env!("CARGO_BIN_EXE_asus-numberpad"))
        .args(["_fixture-stop", role])
        .env("XDG_RUNTIME_DIR", root)
        .status()
        .unwrap()
}
#[test]
fn authenticated_socket_rejects_wrong_token_and_stops_cleanly() {
    let root = root();
    let mut child = spawn(root.path(), "runtime", &[]);
    wait(root.path(), "runtime", &mut child);
    let socket = root.path().join("asus-numberpad.sock");
    assert!(
        fs::symlink_metadata(&socket)
            .unwrap()
            .file_type()
            .is_socket()
    );
    {
        let mut stream = UnixStream::connect(&socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream.write_all(b"STOP wrong-token\n").unwrap();
        let mut reply = [0];
        assert_eq!(stream.read(&mut reply).unwrap(), 0);
    }
    assert!(child.0.try_wait().unwrap().is_none());
    assert!(stop(root.path(), "runtime").success());
    assert!(child.0.wait().unwrap().success());
    assert!(!root.path().join("asus-numberpad.json").exists());
    assert!(!socket.exists());
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.path().join("asus-numberpad.lock"))
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
}
#[test]
fn duplicate_instance_and_unowned_endpoint_refused() {
    let root = root();
    let mut child = spawn(root.path(), "runtime", &[]);
    wait(root.path(), "runtime", &mut child);
    let mut duplicate = spawn(root.path(), "runtime", &[]);
    assert!(!duplicate.0.wait().unwrap().success());
    assert!(stop(root.path(), "runtime").success());
    assert!(child.0.wait().unwrap().success());
    fs::write(root.path().join("asus-numberpad.sock"), "preserve").unwrap();
    let mut duplicate = spawn(root.path(), "runtime", &[]);
    assert!(!duplicate.0.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(root.path().join("asus-numberpad.sock")).unwrap(),
        "preserve"
    );
}
#[test]
fn forged_process_identity_does_not_stop_owner() {
    let root = root();
    let mut child = spawn(root.path(), "runtime", &[]);
    let mut id = wait(root.path(), "runtime", &mut child);
    let path = root.path().join("asus-numberpad.json");
    let original = fs::read(&path).unwrap();
    id["pid"] = serde_json::json!(std::process::id());
    fs::write(&path, serde_json::to_vec(&id).unwrap()).unwrap();
    assert!(!stop(root.path(), "runtime").success());
    assert!(child.0.try_wait().unwrap().is_none());
    fs::write(&path, original).unwrap();
    assert!(stop(root.path(), "runtime").success());
    assert!(child.0.wait().unwrap().success());
}
#[test]
fn successful_session_cleanup_requires_recorded_completion() {
    let root = root();
    let mut child = spawn(root.path(), "session", &[]);
    wait(root.path(), "session", &mut child);
    assert!(stop(root.path(), "session").success());
    assert!(child.0.wait().unwrap().success());
    assert!(!root.path().join("asus-numberpad-session.json").exists());
    let outcome: serde_json::Value = serde_json::from_slice(
        &fs::read(root.path().join("asus-numberpad-session.result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(outcome["success"], true);
}
struct Survivor {
    pid: i32,
    start: String,
}
impl Drop for Survivor {
    fn drop(&mut self) {
        let path = format!("/proc/{}/stat", self.pid);
        if fs::read_to_string(path)
            .ok()
            .and_then(|s| {
                s.rsplit(')')
                    .next()
                    .and_then(|s| s.split_whitespace().nth(19))
                    .map(str::to_string)
            })
            .as_deref()
            == Some(self.start.as_str())
        {
            unsafe {
                libc::kill(self.pid, libc::SIGTERM);
            }
        }
    }
}
#[test]
fn failed_session_teardown_with_surviving_child_blocks_cleanup() {
    let root = root();
    let mut child = spawn(root.path(), "session", &["--fail-cleanup"]);
    wait(root.path(), "session", &mut child);
    let until = Instant::now() + Duration::from_secs(2);
    let path = root.path().join("fixture-survivor.pid");
    while !path.exists() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(10));
    }
    let pid: i32 = fs::read_to_string(&path).unwrap().parse().unwrap();
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let start = stat
        .rsplit(')')
        .next()
        .unwrap()
        .split_whitespace()
        .nth(19)
        .unwrap()
        .to_string();
    let _survivor = Survivor { pid, start };
    assert!(!stop(root.path(), "session").success());
    assert!(child.0.wait().unwrap().success());
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    assert!(!stop(root.path(), "session").success());
}
