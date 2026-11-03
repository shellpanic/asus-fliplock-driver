//! Explicit test-only fixtures; this module is absent from production builds.
use anyhow::Result;
use std::{
    fs, os::unix::fs::MetadataExt, path::Path, sync::atomic::Ordering, thread, time::Duration,
};
pub fn enabled() -> bool {
    fs::symlink_metadata("/tmp/numberpad-fixture/enabled")
        .is_ok_and(|m| m.is_file() && m.uid() == 0 && m.mode() & 0o022 == 0)
        && Path::new("/.dockerenv").exists()
}
pub fn owner(role: &str, ready: bool, fail_cleanup: bool) -> Result<()> {
    let mut owner = crate::control::Owner::acquire(role)?;
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        signal_hook::flag::register(signal, owner.stopping.clone())?;
    }
    let _survivor = if fail_cleanup {
        let child = std::process::Command::new("/bin/sleep").arg("30").spawn()?;
        std::fs::write(
            crate::util::runtime_dir()?.join("fixture-survivor.pid"),
            child.id().to_string(),
        )?;
        Some(child)
    } else {
        None
    };
    if ready {
        owner.ready()?;
    }
    while !owner.stopped() {
        thread::sleep(Duration::from_millis(20));
    }
    owner.stopping.store(true, Ordering::SeqCst);
    if role == "session" && !fail_cleanup {
        owner.complete()?;
    }
    Ok(())
}
