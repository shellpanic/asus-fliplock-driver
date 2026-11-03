use anyhow::{Result, bail};
use std::{
    fs::File,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Duration,
};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::OwnedFd,
};
pub struct Monitor {
    connection: Connection,
    events: Receiver<Result<bool, String>>,
    _inhibitor: Option<File>,
    threads: Vec<std::thread::JoinHandle<()>>,
}
impl Monitor {
    pub fn new() -> Result<Self> {
        Self::connect(zbus::blocking::connection::Builder::system()?)
    }
    fn connect(builder: zbus::blocking::connection::Builder<'_>) -> Result<Self> {
        let connection = builder.method_timeout(Duration::from_secs(2)).build()?;
        let name_proxy = Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )?;
        // Subscribe before resolving the owner, then bind all methods/signals to
        // that unique connection. A restarted manager needs a NEW inhibitor.
        let mut owners = name_proxy.receive_owner_changed()?;
        let owner = manager_owner(&connection)?;
        let proxy = Proxy::new(
            &connection,
            owner.clone(),
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )?;
        let iter = proxy.receive_signal("PrepareForSleep")?;
        if preparing(&connection, &owner)? {
            bail!("System is preparing for sleep; startup refused");
        }
        let descriptor: OwnedFd = proxy.call(
            "Inhibit",
            &(
                "sleep",
                "ASUS NumberPad",
                "Release input before sleep",
                "delay",
            ),
        )?;
        let inhibitor = File::from(std::os::fd::OwnedFd::from(descriptor));
        if preparing(&connection, &owner)? {
            bail!("Sleep began while acquiring inhibitor; startup refused");
        }
        if manager_owner(&connection)? != owner {
            bail!("Login manager changed during inhibitor setup");
        }
        let (tx, events) = mpsc::channel();
        let owner_tx = tx.clone();
        let owner_thread = thread::spawn(move || {
            let _ = owners.next();
            let _ = owner_tx.send(Err(
                "Login manager owner changed; inhibitor no longer protects cleanup".into(),
            ));
        });
        let thread = thread::spawn(move || {
            for message in iter {
                let value = message
                    .body()
                    .deserialize::<(bool,)>()
                    .map(|v| v.0)
                    .map_err(|error| error.to_string());
                if tx.send(value).is_err() {
                    return;
                }
            }
            let _ = tx.send(Err("Sleep signal subscription closed".into()));
        });
        Ok(Self {
            connection,
            events,
            _inhibitor: Some(inhibitor),
            threads: vec![thread, owner_thread],
        })
    }
    pub fn sleeping(&self) -> Result<bool> {
        if self.connection.is_closed() {
            bail!("Lost login manager signal connection");
        }
        loop {
            match self.events.try_recv() {
                Ok(Ok(true)) => return Ok(true),
                Ok(Ok(false)) => {}
                Ok(Err(error)) => bail!("Invalid sleep signal: {error}"),
                Err(TryRecvError::Empty) => return Ok(false),
                Err(TryRecvError::Disconnected) => bail!("Sleep signal subscription closed"),
            }
        }
    }
}

fn manager_owner(connection: &Connection) -> Result<String> {
    let bus = Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;
    Ok(bus.call("GetNameOwner", &("org.freedesktop.login1",))?)
}
fn preparing(connection: &Connection, owner: &str) -> Result<bool> {
    let properties = Proxy::new(
        connection,
        owner,
        "/org/freedesktop/login1",
        "org.freedesktop.DBus.Properties",
    )?;
    let value: zbus::zvariant::OwnedValue = properties.call(
        "Get",
        &("org.freedesktop.login1.Manager", "PreparingForSleep"),
    )?;
    Ok(bool::try_from(value)?)
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self._inhibitor.take();
        let _ = futures_lite::future::block_on(self.connection.inner().clone().close());
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read},
        os::unix::net::UnixStream,
        process::{Command, Stdio},
        sync::Mutex,
        time::Instant,
    };
    struct Login {
        descriptor: Mutex<Option<std::os::fd::OwnedFd>>,
        transition: bool,
        lose_owner: bool,
        preparing: std::sync::atomic::AtomicBool,
    }
    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl Login {
        async fn inhibit(
            &self,
            _what: &str,
            _who: &str,
            _why: &str,
            _mode: &str,
            #[zbus(connection)] connection: &zbus::Connection,
        ) -> zbus::fdo::Result<OwnedFd> {
            assert_eq!(_what, "sleep");
            assert_eq!(_mode, "delay");
            if self.transition {
                self.preparing
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if self.lose_owner {
                connection
                    .release_name("org.freedesktop.login1")
                    .await
                    .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
            }
            self.descriptor
                .lock()
                .unwrap()
                .take()
                .map(OwnedFd::from)
                .ok_or_else(|| zbus::fdo::Error::Failed("Already inhibited".into()))
        }
        #[zbus(property)]
        fn preparing_for_sleep(&self) -> bool {
            self.preparing.load(std::sync::atomic::Ordering::SeqCst)
        }
    }
    #[test]
    fn restricted_bus_sleep_and_inhibitor_lifetime() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("bus.conf");
        std::fs::write(&config,r#"<busconfig><type>system</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><policy context="default"><allow user="*"/><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/><deny send_interface="org.freedesktop.DBus.Monitoring"/></policy></busconfig>"#).unwrap();
        let mut daemon = Command::new("dbus-daemon")
            .arg("--nofork")
            .arg(format!("--config-file={}", config.display()))
            .arg("--print-address")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_string();
        assert!(!address.is_empty());
        let result = std::panic::catch_unwind(|| {
            let (read_end, mut peer) = UnixStream::pair().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let service = zbus::blocking::connection::Builder::address(address.as_str())
                .unwrap()
                .method_timeout(Duration::from_secs(2))
                .name("org.freedesktop.login1")
                .unwrap()
                .serve_at(
                    "/org/freedesktop/login1",
                    Login {
                        descriptor: Mutex::new(Some(read_end.into())),
                        transition: false,
                        lose_owner: false,
                        preparing: std::sync::atomic::AtomicBool::new(false),
                    },
                )
                .unwrap()
                .build()
                .unwrap();
            let monitor = Monitor::connect(
                zbus::blocking::connection::Builder::address(address.as_str()).unwrap(),
            )
            .unwrap();
            let proxy = Proxy::new(
                &monitor.connection,
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus.Monitoring",
            )
            .unwrap();
            assert!(
                proxy
                    .call::<_, _, ()>("BecomeMonitor", &(Vec::<&str>::new(), 0u32))
                    .is_err()
            );
            assert!(!monitor.sleeping().unwrap());
            let spoofer = zbus::blocking::connection::Builder::address(address.as_str())
                .unwrap()
                .build()
                .unwrap();
            spoofer
                .emit_signal(
                    Some(monitor.connection.unique_name().unwrap().as_str()),
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                    "PrepareForSleep",
                    &(true,),
                )
                .unwrap();
            thread::sleep(Duration::from_millis(40));
            assert!(
                !monitor.sleeping().unwrap(),
                "Unicast signal from wrong sender accepted"
            );

            service
                .emit_signal(
                    None::<&str>,
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                    "PrepareForSleep",
                    &(true,),
                )
                .unwrap();
            let until = Instant::now() + Duration::from_secs(2);
            while !monitor.sleeping().unwrap() {
                assert!(Instant::now() < until);
                thread::sleep(Duration::from_millis(10));
            }
            drop(monitor);
            let mut byte = [0];
            assert_eq!(peer.read(&mut byte).unwrap(), 0, "inhibitor FD leaked");
        });
        let _ = daemon.kill();
        let _ = daemon.wait();
        assert!(result.is_ok());
    }
    #[test]
    fn sleep_transition_during_inhibit_refuses_startup_and_closes_fd() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("bus.conf");
        std::fs::write(&config,r#"<busconfig><type>system</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><policy context="default"><allow user="*"/><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/><deny send_interface="org.freedesktop.DBus.Monitoring"/></policy></busconfig>"#).unwrap();
        let mut daemon = Command::new("dbus-daemon")
            .arg("--nofork")
            .arg(format!("--config-file={}", config.display()))
            .arg("--print-address")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_string();
        assert!(!address.is_empty());
        let result = std::panic::catch_unwind(|| {
            let (read_end, mut peer) = UnixStream::pair().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let _service = zbus::blocking::connection::Builder::address(address.as_str())
                .unwrap()
                .method_timeout(Duration::from_secs(2))
                .name("org.freedesktop.login1")
                .unwrap()
                .serve_at(
                    "/org/freedesktop/login1",
                    Login {
                        descriptor: Mutex::new(Some(read_end.into())),
                        transition: true,
                        lose_owner: false,
                        preparing: std::sync::atomic::AtomicBool::new(false),
                    },
                )
                .unwrap()
                .build()
                .unwrap();
            let error = Monitor::connect(
                zbus::blocking::connection::Builder::address(address.as_str()).unwrap(),
            )
            .err()
            .expect("startup should be refused");
            assert!(error.to_string().contains("Sleep began"));
            let mut byte = [0];
            assert_eq!(peer.read(&mut byte).unwrap(), 0);
        });
        let _ = daemon.kill();
        let _ = daemon.wait();
        assert!(result.is_ok());
    }
    struct PrivateBus {
        _root: tempfile::TempDir,
        daemon: std::process::Child,
        address: String,
    }
    impl PrivateBus {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let config = root.path().join("bus.conf");
            std::fs::write(&config, r#"<busconfig><type>system</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><policy context="default"><allow user="*"/><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/><deny send_interface="org.freedesktop.DBus.Monitoring"/></policy></busconfig>"#).unwrap();
            let mut daemon = Command::new("dbus-daemon")
                .args(["--nofork", "--print-address"])
                .arg(format!("--config-file={}", config.display()))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut address = String::new();
            BufReader::new(daemon.stdout.take().unwrap())
                .read_line(&mut address)
                .unwrap();
            assert!(!address.trim().is_empty());
            Self {
                _root: root,
                daemon,
                address: address.trim().into(),
            }
        }
        fn builder(&self) -> zbus::blocking::connection::Builder<'_> {
            zbus::blocking::connection::Builder::address(self.address.as_str())
                .unwrap()
                .method_timeout(Duration::from_secs(2))
        }
        fn login(&self, fd: std::os::fd::OwnedFd, lose_owner: bool) -> Connection {
            self.builder()
                .name("org.freedesktop.login1")
                .unwrap()
                .serve_at(
                    "/org/freedesktop/login1",
                    Login {
                        descriptor: Mutex::new(Some(fd)),
                        transition: false,
                        lose_owner,
                        preparing: std::sync::atomic::AtomicBool::new(false),
                    },
                )
                .unwrap()
                .build()
                .unwrap()
        }
    }
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }
    #[test]
    fn login_manager_loss_or_replacement_invalidates_inhibitor_on_live_bus() {
        for replacement in [false, true] {
            let bus = PrivateBus::new();
            let (fd, mut peer) = UnixStream::pair().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let service = bus.login(fd.into(), false);
            let monitor = Monitor::connect(bus.builder()).unwrap();
            assert!(!monitor.sleeping().unwrap());
            service.release_name("org.freedesktop.login1").unwrap();
            let _new_owner = if replacement {
                Some(
                    bus.builder()
                        .name("org.freedesktop.login1")
                        .unwrap()
                        .build()
                        .unwrap(),
                )
            } else {
                None
            };
            assert!(!monitor.connection.is_closed(), "test must keep bus alive");
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                if monitor.sleeping().is_err() {
                    break;
                }
                assert!(Instant::now() < until, "owner loss did not stop runtime");
                thread::sleep(Duration::from_millis(10));
            }
            drop(monitor);
            assert_eq!(peer.read(&mut [0]).unwrap(), 0, "stale inhibitor FD leaked");
        }
    }
    #[test]
    fn owner_loss_during_inhibit_refuses_startup_and_releases_fd() {
        let bus = PrivateBus::new();
        let (fd, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let _service = bus.login(fd.into(), true);
        assert!(Monitor::connect(bus.builder()).is_err());
        assert_eq!(
            peer.read(&mut [0]).unwrap(),
            0,
            "startup leaked inhibitor FD"
        );
    }
    #[test]
    fn bus_disconnect_is_detected_without_waiting_for_sleep_signal() {
        let mut bus = PrivateBus::new();
        let (fd, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let _service = bus.login(fd.into(), false);
        let monitor = Monitor::connect(bus.builder()).unwrap();
        bus.daemon.kill().unwrap();
        bus.daemon.wait().unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        while monitor.sleeping().is_ok() {
            assert!(Instant::now() < until, "bus disconnect was not detected");
            thread::sleep(Duration::from_millis(10));
        }
        drop(monitor);
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }
}
