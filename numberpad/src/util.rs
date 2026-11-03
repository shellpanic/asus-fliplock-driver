use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn read(path: impl AsRef<Path>) -> Result<String> {
    Ok(fs::read_to_string(path.as_ref())
        .with_context(|| format!("Read {}", path.as_ref().display()))?
        .trim()
        .into())
}
pub fn output(program: &str, args: &[&str], limit: Duration) -> Result<Vec<u8>> {
    let trusted = if uid() == 0 {
        Some(root_tool(program)?)
    } else {
        None
    };
    let mut command = Command::new(trusted.as_deref().unwrap_or(Path::new(program)));
    if uid() == 0 {
        command
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C");
    }
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Start {program}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let drain = |mut input: std::process::ChildStdout| -> Result<Vec<u8>> {
        let mut data = Vec::new();
        input.read_to_end(&mut data)?;
        Ok(data)
    };
    let out = thread::spawn(move || drain(stdout));
    let err = thread::spawn(move || {
        let mut input = stderr;
        let mut data = Vec::new();
        input.read_to_end(&mut data).map(|_| data)
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            let _ = out.join();
            let _ = err.join();
            bail!("{program} timed out");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let bytes = out
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader failed"))??;
    let errors = err
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader failed"))??;
    if !status.success() {
        bail!("{program} failed: {}", String::from_utf8_lossy(&errors));
    }
    Ok(bytes)
}
pub fn command(program: &str, args: &[&str]) -> Result<()> {
    let trusted = if uid() == 0 {
        Some(root_tool(program)?)
    } else {
        None
    };
    let mut command = Command::new(trusted.as_deref().unwrap_or(Path::new(program)));
    if uid() == 0 {
        command
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C");
    }
    let status = command
        .args(args)
        .status()
        .with_context(|| format!("Run {program}"))?;
    if !status.success() {
        bail!("{program} failed ({status})");
    }
    Ok(())
}
pub fn uid() -> u32 {
    unsafe { libc::getuid() }
}
pub fn require_user() -> Result<()> {
    if uid() == 0 {
        bail!("Run as the intended regular graphical user");
    }
    Ok(())
}
pub fn runtime_dir() -> Result<std::path::PathBuf> {
    use std::os::unix::fs::MetadataExt;
    require_user()?;
    let path = std::path::PathBuf::from(
        std::env::var("XDG_RUNTIME_DIR").context("Missing user runtime directory")?,
    );
    let info = fs::symlink_metadata(&path)?;
    if !info.is_dir() || info.uid() != uid() || info.mode() & 0o077 != 0 {
        bail!("Runtime directory must be private and owned by the session user");
    }
    Ok(path)
}

pub fn root_tool(program: &str) -> Result<std::path::PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let name = Path::new(program)
        .file_name()
        .context("Missing tool name")?;
    for directory in ["/usr/sbin", "/usr/bin", "/sbin", "/bin"] {
        let path = Path::new(directory).join(name);
        if let Ok(info) = fs::metadata(&path)
            && info.is_file()
            && info.uid() == 0
            && info.mode() & 0o022 == 0
        {
            return Ok(path);
        }
    }
    bail!("No trusted root-owned native tool: {program}")
}
