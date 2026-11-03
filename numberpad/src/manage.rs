use crate::{
    control,
    discovery::{self, Device},
    engine::{Activation, Settings},
    keymap, session, util,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env,
    ffi::CStr,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
const BIN: &str = "/usr/bin/asus-numberpad";
const RULE: &str = "/etc/udev/rules.d/70-asus-numberpad.rules";
const ENROLLMENT: &str = "/var/lib/asus-numberpad/enrollment.json";
const MODULES: &str = "/etc/modules-load.d/asus-numberpad.conf";
const MARKER: &str = "# Owned by shellpanic/asus-fliplock-driver NumberPad v3\n";
const AUTOSTART: &str = "[Desktop Entry]\nType=Application\nName=ASUS NumberPad\nExec=/usr/bin/asus-numberpad session\nTerminal=false\nComment=Optional numbers-only keypad, initially off\n";
const UNIT: &str = "[Unit]\nDescription=ASUS NumberPad in the enrolled graphical session\n[Service]\nType=simple\nExecStart=/usr/bin/asus-numberpad run\nRestart=on-failure\nRestartSec=3\nTimeoutStopSec=12\n";
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Account {
    name: String,
    uid: u32,
}
fn account(uid: u32) -> Result<Account> {
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut pointer = std::ptr::null_mut();
    let mut buffer = vec![0u8; 16384];
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut pointer,
        )
    };
    if rc != 0 || pointer.is_null() {
        bail!("Cannot identify selected account");
    }
    let name = unsafe { CStr::from_ptr(entry.pw_name) }
        .to_str()?
        .to_string();
    if uid == 0
        || name.is_empty()
        || !name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-$".contains(&c))
    {
        bail!("Select a supported regular user account");
    }
    Ok(Account { name, uid })
}
fn named_account(name: &str) -> Result<Account> {
    let name = std::ffi::CString::new(name)?;
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut pointer = std::ptr::null_mut();
    let mut buffer = vec![0u8; 16384];
    let rc = unsafe {
        libc::getpwnam_r(
            name.as_ptr(),
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut pointer,
        )
    };
    if rc != 0 || pointer.is_null() {
        bail!("Cannot resolve selected account");
    }
    let user = account(entry.pw_uid)?;
    if user.name.as_bytes() != name.as_bytes() {
        bail!("Account identity changed");
    }
    Ok(user)
}
fn config_root() -> Result<PathBuf> {
    Ok(env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(PathBuf::from(env::var("HOME")?).join(".config")))
}
fn config_path() -> Result<PathBuf> {
    Ok(config_root()?.join("asus-numberpad/config.json"))
}
fn user_files() -> Result<Vec<(PathBuf, String)>> {
    let root = config_root()?;
    Ok(vec![
        (
            root.join("autostart/asus-numberpad.desktop"),
            AUTOSTART.into(),
        ),
        (
            root.join("systemd/user/asus-numberpad.service"),
            UNIT.into(),
        ),
    ])
}
fn module_content() -> String {
    format!("{MARKER}uinput\ni2c-dev\n")
}
fn rules(user: &str) -> String {
    format!(
        "{MARKER}# Selected account: {user}\nSUBSYSTEM==\"input\", KERNEL==\"event*\", PROGRAM=\"{BIN} _match %p\", RESULT==\"input\", RUN+=\"{BIN} _refresh %p\", OWNER=\"{user}\", MODE=\"0600\", ENV{{ASUS_NUMBERPAD_OWNER}}=\"{user}\"\nSUBSYSTEM==\"i2c-dev\", PROGRAM=\"{BIN} _match %p\", RESULT==\"adapter\", OWNER=\"{user}\", MODE=\"0600\", ENV{{ASUS_NUMBERPAD_OWNER}}=\"{user}\"\nSUBSYSTEM==\"misc\", KERNEL==\"uinput\", OWNER=\"{user}\", MODE=\"0600\", ENV{{ASUS_NUMBERPAD_OWNER}}=\"{user}\"\n"
    )
}
fn enrollment(user: &str) -> String {
    format!("{{\"owner\": \"{user}\"}}\n")
}
fn owned(path: &Path, contents: &[String], owner: u32) -> Result<bool> {
    let info = match fs::symlink_metadata(path) {
        Ok(info) => info,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !info.is_file()
        || info.uid() != owner
        || info.mode() & 0o022 != 0
        || !contents.contains(&fs::read_to_string(path)?)
    {
        bail!("Preserve modified or unowned file: {}", path.display());
    }
    Ok(true)
}
fn atomic(path: &Path, value: &str, mode: u32) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".numberpad-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(value.as_bytes())?;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
#[derive(Serialize, Deserialize)]
struct Snapshot {
    account: Account,
    device: Option<Device>,
    files: BTreeMap<String, Option<String>>,
    enable: bool,
    modules: bool,
}
fn snapshot(user: Account, enable: bool, modules: bool) -> Result<Snapshot> {
    let mut files = BTreeMap::new();
    let clear = format!(
        "{MARKER}ENV{{ASUS_NUMBERPAD_OWNER}}==\"{}\", ENV{{ASUS_NUMBERPAD_OWNER}}=\"\"\n",
        user.name
    );
    for (path, accepted) in [
        (RULE, vec![rules(&user.name), clear]),
        (ENROLLMENT, vec![enrollment(&user.name)]),
        (MODULES, vec![module_content()]),
    ] {
        let exists = owned(Path::new(path), &accepted, 0)?;
        files.insert(
            path.into(),
            if exists {
                Some(fs::read_to_string(path)?)
            } else {
                None
            },
        );
    }
    let device = match discovery::probe() {
        Ok(device) => Some(device),
        Err(error) if enable => return Err(error),
        Err(_) => None,
    };
    Ok(Snapshot {
        account: user,
        device,
        files,
        enable,
        modules,
    })
}
fn token(plan: &Snapshot) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(plan)?)))
}
fn show(plan: &Snapshot) -> Result<()> {
    if let Some(device) = &plan.device {
        println!("{}", serde_json::to_string_pretty(device)?);
    }
    println!(
        "Account: {} (uid {}). {} permissions for selected touchpad and isolated tablet switch, entire resolved I2C adapter (all addresses), and uinput arbitrary input injection. No global input group or physical keyboard access.",
        plan.account.name,
        plan.account.uid,
        if plan.enable { "Enroll" } else { "Revoke" }
    );
    println!(
        "Root files: {RULE}, {ENROLLMENT}, {MODULES}. Device/bus numbers are rediscovered, not stored in config."
    );
    if plan.enable {
        println!(
            "Load fixed uinput/i2c-dev interfaces and persist them in the owned module file; prepare current-user startup and start supervision now, initially off. Double-tap the upper-right icon; inactive taps may also reach the desktop as ordinary clicks. Hold remains configurable."
        );
    } else {
        println!(
            "Stop only authenticated owned NumberPad processes, remove owned startup/module config, revoke device grants, retain user config and shared modules/dependencies."
        );
    }
    Ok(())
}
fn confirm(yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    print!("Type apply to accept these changes: ");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    if value.trim() != "apply" {
        bail!("Cancelled");
    }
    Ok(())
}
fn native(plan: &Snapshot) -> Result<()> {
    let fingerprint = token(plan)?;
    let program = if Path::new("/usr/bin/sudo").exists() {
        "/usr/bin/sudo"
    } else if Path::new("/usr/bin/doas").exists() {
        "/usr/bin/doas"
    } else {
        bail!("Native sudo/doas authentication is required");
    };
    util::command(
        program,
        &[
            BIN,
            "_permissions",
            &plan.account.name,
            if plan.enable { "enable" } else { "disable" },
            "--expected",
            &fingerprint,
            if plan.modules {
                "--modules"
            } else {
                "--no-modules"
            },
        ],
    )
}
pub fn settings() -> Result<Settings> {
    let path = config_path()?;
    let settings = if path.exists() {
        let info = fs::symlink_metadata(&path)?;
        if !info.is_file() || info.uid() != util::uid() {
            bail!("Unowned configuration preserved");
        }
        serde_json::from_slice(&fs::read(&path)?)?
    } else {
        Settings::default()
    };
    settings.validate()?;
    Ok(settings)
}
pub fn install(apply: bool, yes: bool) -> Result<()> {
    util::require_user()?;
    if !session::active()? {
        bail!("Install from an active unlocked local graphical login");
    }
    keymap::resolve(&keymap::source()?)?;
    settings()?;
    let user = account(util::uid())?;
    for (path, value) in user_files()? {
        owned(&path, &[value], util::uid())?;
    }
    let plan = snapshot(user, true, true)?;
    show(&plan)?;
    if !apply {
        return Ok(());
    }
    confirm(yes)?;
    // Stop prior managed versions before enrollment/startup. Legacy unmanaged
    // foreground versions require their terminal to be stopped once.
    control::stop("session")?;
    control::stop("runtime")?;
    {
        let _parent = control::Lock::acquire("session")?;
        let _runtime = control::Lock::acquire("runtime")?;
        if token(&snapshot(plan.account.clone(), true, true)?)? != token(&plan)? {
            bail!("Setup state changed; review a fresh plan");
        }
        native(&plan)?;
        for (path, value) in user_files()? {
            if !owned(&path, std::slice::from_ref(&value), util::uid())? {
                atomic(&path, &value, 0o644)?;
            }
        }
        if !config_path()?.exists() {
            atomic(
                &config_path()?,
                &serde_json::to_string_pretty(&Settings::default())?,
                0o600,
            )?;
        }
    }
    if let Err(error) = start() {
        bail!(
            "Startup failed: {error}. Package/enrollment remain recoverable: repeat Install or use Remove. No completion reported."
        );
    }
    Ok(())
}
pub fn disable(apply: bool, yes: bool) -> Result<()> {
    util::require_user()?;
    let user = account(util::uid())?;
    for (path, value) in user_files()? {
        owned(&path, &[value], util::uid())?;
    }
    let plan = snapshot(user, false, false)?;
    show(&plan)?;
    if !apply {
        return Ok(());
    }
    confirm(yes)?;
    control::stop("session")?;
    control::stop("runtime")?;
    let _parent = control::Lock::acquire("session")?;
    let _runtime = control::Lock::acquire("runtime")?;
    // The runtime has released input/uinput and illumination before its control
    // lock is released. No kernel I/O is attempted after grants are revoked.
    if token(&snapshot(plan.account.clone(), false, false)?)? != token(&plan)? {
        bail!("Cleanup state changed; review a fresh plan");
    }
    if plan.files.values().any(Option::is_some) {
        native(&plan)?;
    }
    for (path, value) in user_files()? {
        if owned(&path, &[value], util::uid())? {
            fs::remove_file(path)?;
        }
    }
    let service = util::runtime_dir()?.join("asus-numberpad-runit");
    if service.exists() {
        let info = fs::symlink_metadata(&service)?;
        if !info.is_dir() || info.uid() != util::uid() {
            bail!("Unknown session service directory preserved");
        }
        let run = service.join("driver/run");
        if fs::read_to_string(&run).ok().as_deref() != Some(run_script()) {
            bail!("Modified session service preserved");
        }
        validate_service_tree(&service, "")?;
        fs::remove_dir_all(service)?;
    }
    println!(
        "NumberPad stopped; enrollment, module persistence and startup removed. User settings/shared dependencies retained."
    );
    Ok(())
}
// No user-added runtime files may be erased during normal removal.
fn validate_service_tree(path: &Path, relative: &str) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .context("Unknown session service filename preserved")?;
        let value = if relative.is_empty() {
            name.to_string()
        } else {
            format!("{relative}/{name}")
        };
        let info = fs::symlink_metadata(entry.path())?;
        let directory = matches!(value.as_str(), "driver" | "driver/supervise");
        let known = directory
            || matches!(
                value.as_str(),
                "driver/run"
                    | "driver/supervise/control"
                    | "driver/supervise/ok"
                    | "driver/supervise/lock"
                    | "driver/supervise/status"
                    | "driver/supervise/stat"
                    | "driver/supervise/pid"
            );
        if !known
            || info.uid() != util::uid()
            || info.file_type().is_symlink()
            || (directory && !info.is_dir())
            || (!directory && !info.is_file() && !info.file_type().is_fifo())
        {
            bail!(
                "Unknown or modified session service content preserved: {}",
                entry.path().display()
            );
        }
        if directory {
            validate_service_tree(&entry.path(), &value)?;
        }
    }
    Ok(())
}
fn root_binary() -> Result<()> {
    let info = fs::metadata(env::current_exe()?)?;
    if util::uid() != 0 || info.uid() != 0 || info.mode() & 0o022 != 0 {
        bail!("Root helper requires the native root-owned package binary");
    }
    Ok(())
}
pub fn permissions(user: &str, enable: bool, modules: bool, expected: &str) -> Result<()> {
    root_binary()?;
    let user = named_account(user)?;
    validate_caller(
        &user,
        env::var("SUDO_UID").ok().as_deref(),
        env::var("DOAS_USER").ok().as_deref(),
    )?;
    let plan = snapshot(user.clone(), enable, modules)?;
    if token(&plan)? != expected {
        bail!("Reviewed setup state changed; no root actions applied");
    }
    if enable {
        let before = plan.device.as_ref().context("Exact hardware required")?;
        if modules {
            util::command(
                module_tool()?
                    .to_str()
                    .context("Invalid native tool path")?,
                &["uinput"],
            )?;
            util::command(
                module_tool()?
                    .to_str()
                    .context("Invalid native tool path")?,
                &["i2c-dev"],
            )?;
        }
        let current = discovery::probe()?;
        if current.event != before.event
            || current.sysfs != before.sysfs
            || current.adapter != before.adapter
            || current.switch.as_ref().map(|s| &s.sysfs) != before.switch.as_ref().map(|s| &s.sysfs)
        {
            bail!("Hardware changed while loading modules");
        }
        for path in [
            &current.event,
            &current.adapter,
            &current.uinput,
            &current
                .switch
                .as_ref()
                .context("Missing tablet switch")?
                .event,
        ] {
            use std::os::unix::fs::FileTypeExt;
            if !fs::metadata(path)?.file_type().is_char_device() {
                bail!(
                    "Required device node absent or not character device: {}",
                    path.display()
                );
            }
        }
        if modules && !Path::new(MODULES).exists() {
            atomic(Path::new(MODULES), &module_content(), 0o644)?;
        }
        if !Path::new(ENROLLMENT).exists() {
            atomic(Path::new(ENROLLMENT), &enrollment(&user.name), 0o644)?;
        }
        if !Path::new(RULE).exists() {
            atomic(Path::new(RULE), &rules(&user.name), 0o644)?;
        }
        reload()?;
        let mut paths = vec![
            sys_class("misc").join("uinput"),
            current.sysfs,
            current.adapter_sysfs,
            current.switch.unwrap().sysfs,
        ];
        trigger(&mut paths)?;
    } else {
        if Path::new(RULE).exists() || Path::new(ENROLLMENT).exists() {
            // The user holds both lifetime locks across native authentication.
            // Root recovery also covers setup interrupted before udev granted access.
            crate::runtime::recover_locked(true)?;
            let clear = format!(
                "{MARKER}ENV{{ASUS_NUMBERPAD_OWNER}}==\"{}\", ENV{{ASUS_NUMBERPAD_OWNER}}=\"\"\n",
                user.name
            );
            atomic(Path::new(RULE), &clear, 0o644)?;
            reload()?;
            let mut paths = vec![];
            for class in [sys_class("input"), sys_class("i2c-dev"), sys_class("misc")] {
                let Ok(entries) = fs::read_dir(class) else {
                    continue;
                };
                for entry in entries {
                    let entry = entry?;
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if !(name.starts_with("event") || name.starts_with("i2c-") || name == "uinput")
                    {
                        continue;
                    }
                    let data = util::output(
                        "udevadm",
                        &[
                            "info",
                            "--query=property",
                            "--path",
                            entry.path().to_str().context("Non-UTF8 sysfs path")?,
                        ],
                        Duration::from_secs(2),
                    )?;
                    let text = String::from_utf8(data)?;
                    let props: BTreeMap<_, _> =
                        text.lines().filter_map(|l| l.split_once('=')).collect();
                    if props.get("ASUS_NUMBERPAD_OWNER").copied() != Some(user.name.as_str()) {
                        continue;
                    }
                    if let Some(node) = props.get("DEVNAME") {
                        let node = Path::new(node);
                        if !node.starts_with("/dev")
                            || node
                                .components()
                                .any(|c| matches!(c, std::path::Component::ParentDir))
                        {
                            bail!("Invalid enrolled device path");
                        }
                        if let Ok(info) = fs::symlink_metadata(node) {
                            use std::os::unix::fs::FileTypeExt;
                            if !info.file_type().is_char_device() {
                                bail!("Owned device is no longer a character node");
                            }
                            if info.uid() == user.uid {
                                let c =
                                    std::ffi::CString::new(node.as_os_str().as_encoded_bytes())?;
                                if unsafe { libc::chown(c.as_ptr(), 0, 0) } != 0 {
                                    return Err(io::Error::last_os_error().into());
                                }
                                fs::set_permissions(node, fs::Permissions::from_mode(0o600))?;
                            }
                        }
                    }
                    paths.push(entry.path());
                }
            }
            trigger(&mut paths)?;
            fs::remove_file(RULE)?;
            reload()?;
            if Path::new(ENROLLMENT).exists() {
                fs::remove_file(ENROLLMENT)?;
            }
        }
        if Path::new(MODULES).exists() {
            fs::remove_file(MODULES)?;
        }
    }
    Ok(())
}
fn reload() -> Result<()> {
    util::command("udevadm", &["control", "--reload-rules"])
}
fn trigger(paths: &mut Vec<PathBuf>) -> Result<()> {
    paths.sort();
    paths.dedup();
    for path in paths {
        if path.exists() {
            util::command(
                "udevadm",
                &[
                    "trigger",
                    "--action=change",
                    path.to_str().context("Non-UTF8 sysfs path")?,
                ],
            )?;
        }
    }
    util::command("udevadm", &["settle", "--timeout=5"])
}
pub fn matching(path: &str, refresh: bool) -> Result<()> {
    if path.contains("..") || !path.starts_with('/') {
        bail!("Invalid sysfs path");
    }
    let device = discovery::discover(Path::new("/sys"), Path::new("/dev"), false)?;
    let selected = fs::canonicalize(Path::new("/sys").join(path.trim_start_matches('/')))?;
    if refresh {
        root_binary()?;
        if selected == device.sysfs {
            util::command(
                "udevadm",
                &[
                    "trigger",
                    "--action=change",
                    device
                        .adapter_sysfs
                        .to_str()
                        .context("Non-UTF8 sysfs path")?,
                ],
            )?;
        }
        return Ok(());
    }
    if selected == device.sysfs || device.switch.as_ref().is_some_and(|s| s.sysfs == selected) {
        println!("input");
    } else if selected == device.adapter_sysfs {
        println!("adapter");
    }
    Ok(())
}
fn run_script() -> &'static str {
    "#!/bin/sh\nsleep 2\nexec /usr/bin/asus-numberpad run\n"
}
pub fn supervise() -> Result<()> {
    util::require_user()?;
    if !session::active()? {
        bail!("Start supervision from the active unlocked graphical session");
    }
    let mut owner = control::Owner::acquire("session")?;
    let mut children_started = false;
    let result = supervise_owned(&mut owner, &mut children_started);
    if !children_started {
        owner.complete()?;
    }
    result
}
fn supervise_owned(owner: &mut control::Owner, children_started: &mut bool) -> Result<()> {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        signal_hook::flag::register(signal, owner.stopping.clone())?;
    }
    if !session::active()? {
        bail!("Start supervision from the active unlocked graphical session");
    }
    if Path::new("/usr/bin/systemctl").exists()
        && Command::new("systemctl")
            .args(["--user", "show-environment"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success()
    {
        let names = [
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XAUTHORITY",
            "XDG_SESSION_TYPE",
            "XDG_SESSION_ID",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
            "XDG_CONFIG_HOME",
        ];
        let mut unset = vec!["--user", "unset-environment"];
        unset.extend(names);
        util::command("systemctl", &unset)?;
        let mut import = vec!["--user", "import-environment"];
        import.extend(names.into_iter().filter(|name| env::var_os(name).is_some()));
        util::command("systemctl", &import)?;
        util::command("systemctl", &["--user", "daemon-reload"])?;
        *children_started = true;
        let result = (|| -> Result<()> {
            util::command("systemctl", &["--user", "start", "asus-numberpad.service"])?;
            owner.ready()?;
            let mut was_active = true;
            while !owner.stopped() {
                let active = match session::active() {
                    Ok(value) => value,
                    Err(_) => break,
                };
                if active != was_active {
                    util::command(
                        "systemctl",
                        &[
                            "--user",
                            if active { "start" } else { "stop" },
                            "asus-numberpad.service",
                        ],
                    )?;
                    was_active = active;
                }
                thread::sleep(Duration::from_millis(200));
            }
            Ok(())
        })();
        util::command("systemctl", &["--user", "stop", "asus-numberpad.service"])?;
        owner.complete()?;
        return result;
    }
    let root = util::runtime_dir()?.join("asus-numberpad-runit");
    let driver = root.join("driver");
    fs::create_dir_all(&driver)?;
    let run = driver.join("run");
    if run.exists() && fs::read_to_string(&run)? != run_script() {
        bail!("Modified runit session script preserved");
    }
    if !run.exists() {
        atomic(&run, run_script(), 0o700)?;
    }
    let mut child = Command::new("runsvdir").arg(&root).spawn()?;
    *children_started = true;
    let result = (|| -> Result<()> {
        owner.ready()?;
        let mut was_active = true;
        while !owner.stopped() && child.try_wait()?.is_none() {
            let active = match session::active() {
                Ok(value) => value,
                Err(_) => break,
            };
            if active != was_active {
                util::command(
                    "sv",
                    &[
                        if active { "up" } else { "down" },
                        driver.to_str().context("Invalid service path")?,
                    ],
                )?;
                was_active = active;
            }
            thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    })();
    if child.try_wait()?.is_none() {
        let rc = unsafe { libc::kill(child.id() as i32, libc::SIGHUP) };
        if rc != 0 {
            return Err(io::Error::last_os_error().into());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            bail!("Session supervisor failed to exit; cleanup incomplete");
        }
        thread::sleep(Duration::from_millis(20));
    }
    wait_runit(&driver)?;
    owner.complete()?;
    result
}
fn wait_runit(driver: &Path) -> Result<()> {
    let path = driver.join("supervise/lock");
    if !path.exists() {
        return Ok(());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        if Instant::now() >= until {
            bail!("Runit child remains active; stop refused");
        }
        thread::sleep(Duration::from_millis(20));
    }
}
pub fn start() -> Result<()> {
    if control::ready("runtime")? {
        println!("NumberPad already available.");
        return Ok(());
    }
    if !control::ready("session")? {
        let path = util::runtime_dir()?.join("asus-numberpad-session.log");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        if log.metadata()?.uid() != util::uid() {
            bail!("Unowned session log preserved");
        }
        Command::new(BIN)
            .arg("session")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .process_group(0)
            .spawn()?;
    }
    let until = Instant::now() + Duration::from_secs(12);
    while Instant::now() < until {
        if control::ready("runtime").unwrap_or(false) {
            println!("NumberPad available now; initially off.");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    control::stop("session")?;
    bail!("No ready runtime; inspect $XDG_RUNTIME_DIR/asus-numberpad-session.log")
}
pub fn gesture(value: Activation, apply: bool) -> Result<()> {
    util::require_user()?;
    settings()?;
    println!(
        "Set upper-right NumberPad icon activation to {} in {}; retain other settings. If running, stop and restart only the owned driver, initially off.",
        serde_json::to_string(&value)?,
        config_path()?.display()
    );
    if !apply {
        return Ok(());
    }
    let was_running = control::ready("runtime")?;
    control::stop("session")?;
    control::stop("runtime")?;
    let path = config_path()?;
    let mut data: serde_json::Value = if path.exists() {
        serde_json::from_slice(&fs::read(&path)?)?
    } else {
        serde_json::json!({})
    };
    let object = data
        .as_object_mut()
        .context("Configuration must be an object")?;
    object.insert("activation".into(), serde_json::to_value(value)?);
    atomic(&path, &serde_json::to_string_pretty(&data)?, 0o600)?;
    if was_running {
        start()?;
    }
    Ok(())
}

fn validate_caller(user: &Account, sudo_uid: Option<&str>, doas_user: Option<&str>) -> Result<()> {
    if let Some(uid) = sudo_uid
        && uid.parse::<u32>()? != user.uid
    {
        bail!("Native sudo caller differs from selected account");
    }
    if let Some(name) = doas_user
        && name != user.name
    {
        bail!("Native doas caller differs from selected account");
    }
    // Direct invocation by real root is an explicit administrative operation.
    Ok(())
}
fn sys_class(name: &str) -> PathBuf {
    #[cfg(feature = "test-fixtures")]
    if crate::fixtures::enabled() {
        return Path::new("/tmp/numberpad-fixture/sys/class").join(name);
    }
    Path::new("/sys/class").join(name)
}
fn module_tool() -> Result<PathBuf> {
    for path in [
        "/usr/sbin/modprobe",
        "/usr/bin/modprobe",
        "/sbin/modprobe",
        "/bin/modprobe",
    ] {
        if let Ok(info) = fs::metadata(path)
            && info.is_file()
            && info.uid() == 0
            && info.mode() & 0o022 == 0
        {
            return Ok(PathBuf::from(path));
        }
    }
    bail!("Missing trusted native module loader")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supervision_tree_preserves_unknown_files_and_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let driver = root.path().join("driver");
        fs::create_dir_all(driver.join("supervise")).unwrap();
        fs::write(driver.join("run"), run_script()).unwrap();
        fs::write(driver.join("supervise/lock"), "").unwrap();
        validate_service_tree(root.path(), "").unwrap();
        fs::write(driver.join("notes"), "user data").unwrap();
        assert!(validate_service_tree(root.path(), "").is_err());
        assert_eq!(
            fs::read_to_string(driver.join("notes")).unwrap(),
            "user data"
        );
        fs::remove_file(driver.join("notes")).unwrap();
        std::os::unix::fs::symlink(root.path().join("outside"), driver.join("supervise/stat"))
            .unwrap();
        assert!(validate_service_tree(root.path(), "").is_err());
    }
    #[test]
    fn exact_owned_files_and_modified_preservation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("owned");
        assert!(!owned(&path, &["expected".into()], util::uid()).unwrap());
        atomic(&path, "expected", 0o600).unwrap();
        assert!(owned(&path, &["expected".into()], util::uid()).unwrap());
        assert!(owned(&path, &["different".into()], util::uid()).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(root.path().join("missing"), &path).unwrap();
        assert!(owned(&path, &["expected".into()], util::uid()).is_err());
        assert!(path.is_symlink());
    }
    #[test]
    fn sudo_doas_identity_must_match() {
        let user = Account {
            name: "fixture".into(),
            uid: 1001,
        };
        assert!(validate_caller(&user, Some("1001"), None).is_ok());
        assert!(validate_caller(&user, None, Some("fixture")).is_ok());
        assert!(validate_caller(&user, Some("1002"), None).is_err());
        assert!(validate_caller(&user, None, Some("other")).is_err());
        assert!(validate_caller(&user, Some("1001"), Some("other")).is_err());
        assert!(validate_caller(&user, None, None).is_ok());
    }
    #[test]
    fn plan_token_binds_hardware_scope_account_and_current_files() {
        let mut plan = Snapshot {
            account: Account {
                name: "fixture".into(),
                uid: 1001,
            },
            device: None,
            files: BTreeMap::new(),
            enable: true,
            modules: true,
        };
        let original = token(&plan).unwrap();
        plan.modules = false;
        assert_ne!(original, token(&plan).unwrap());
        plan.modules = true;
        plan.account.uid = 1002;
        assert_ne!(original, token(&plan).unwrap());
        plan.account.uid = 1001;
        plan.files.insert("rule".into(), Some("changed".into()));
        assert_ne!(original, token(&plan).unwrap());
    }
}
