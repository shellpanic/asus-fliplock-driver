use crate::{
    control, discovery,
    engine::{Action, Engine, Settings},
    keymap,
    linux::{self, Input, Output},
    session, sleep,
};
use anyhow::{Result, bail};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
struct Slots {
    count: usize,
    selector: usize,
    ids: Vec<i32>,
    x: Vec<i32>,
    y: Vec<i32>,
}
impl Slots {
    fn new(input: &Input) -> Result<Self> {
        let count = input
            .abs(47)?
            .maximum
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Invalid multitouch slot count"))?;
        if !(1..=32).contains(&count) {
            bail!("Unsupported multitouch slot count");
        }
        let mut slots = Self {
            count: count as usize,
            selector: 0,
            ids: vec![],
            x: vec![],
            y: vec![],
        };
        slots.resync(input)?;
        Ok(slots)
    }
    fn resync(&mut self, input: &Input) -> Result<()> {
        self.ids = input.slots(self.count, 57)?;
        self.x = input.slots(self.count, 53)?;
        self.y = input.slots(self.count, 54)?;
        self.selector = input.abs(47)?.value as usize;
        if self.selector >= self.count {
            bail!("Invalid multitouch selector");
        }
        Ok(())
    }
    fn event(&mut self, code: u16, value: i32) -> Result<()> {
        match code {
            47 => {
                if value < 0 || value as usize >= self.count {
                    bail!("Invalid multitouch slot");
                }
                self.selector = value as usize;
            }
            57 => self.ids[self.selector] = value,
            53 => self.x[self.selector] = value,
            54 => self.y[self.selector] = value,
            _ => {}
        }
        Ok(())
    }
    fn contacts(&self) -> Vec<(i32, i32, i32)> {
        (0..self.count)
            .filter(|i| self.ids[*i] >= 0)
            .map(|i| (self.ids[i], self.x[i], self.y[i]))
            .collect()
    }
}
struct Hardware {
    touch: Option<Input>,
    switch: Option<Input>,
    output: Option<Output>,
    light: linux::Light,
    reset_started: bool,
}
impl Hardware {
    fn actions(&mut self, actions: Vec<Action>, mapping: &keymap::Mapping) -> Result<()> {
        let mut first = None;
        for action in actions {
            let result = match action {
                Action::Key(key) => {
                    if let Some(output) = &mut self.output {
                        output.send(key.and_then(|k| mapping.get(k)))
                    } else {
                        Ok(())
                    }
                }
                Action::Grab(on) => {
                    if let Some(touch) = &mut self.touch {
                        touch.grab(on)
                    } else {
                        Ok(())
                    }
                }
                Action::Light(on) => self.light.set(on),
            };
            if let Err(error) = result
                && first.is_none()
            {
                first = Some(error);
            }
        }
        if let Some(error) = first {
            return Err(error);
        }
        Ok(())
    }
}
impl Drop for Hardware {
    fn drop(&mut self) {
        if let Some(output) = &mut self.output {
            let _ = output.send(None);
        }
        if self.reset_started
            && let Err(error) = self.light.set(false)
        {
            eprintln!("Illumination cleanup failed: {error}");
        }
        if let Some(touch) = &mut self.touch {
            let _ = touch.grab(false);
        }
    }
}
pub fn run(settings: Settings) -> Result<()> {
    settings.validate()?;
    #[cfg(feature = "test-fixtures")]
    if crate::fixtures::enabled() {
        return crate::fixtures::owner(
            "runtime",
            !std::path::Path::new("/tmp/numberpad-fixture/fail-ready").exists(),
            false,
        );
    }
    if !session::active()? {
        bail!("Need an active unlocked local login owned by this user");
    }
    let mut owner = control::Owner::acquire("runtime")?;
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        signal_hook::flag::register(signal, owner.stopping.clone())?;
    }
    let device = discovery::probe()?;
    let source = keymap::source()?;
    let mapping = keymap::resolve(&source)?;
    let monitor = sleep::Monitor::new()?;
    if monitor.sleeping()? || owner.stopped() {
        return Ok(());
    }
    let mut hardware = Hardware {
        touch: None,
        switch: None,
        output: None,
        light: linux::Light::open(&device)?,
        reset_started: true,
    };
    hardware.light.set(false)?;
    hardware.touch = Some(Input::open(&device.event, &device.sysfs, true)?);
    let switch = device
        .switch
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing isolated tablet switch"))?;
    hardware.switch = Some(Input::open(&switch.event, &switch.sysfs, false)?);
    let touch = hardware.touch.as_ref().unwrap();
    let mut slots = Slots::new(touch)?;
    let x = touch.abs(53)?;
    let y = touch.abs(54)?;
    let mut engine = Engine::new((x.minimum, x.maximum, y.minimum, y.maximum), settings)?;
    engine.blocked = !slots.contacts().is_empty();
    hardware.output = Some(Output::new(&device.uinput, &mapping)?);
    let mut tablet = hardware.switch.as_ref().unwrap().tablet()?;
    let mut dropped = false;
    let mut switch_dropped = false;
    let start = Instant::now();
    let mut check = Instant::now();
    owner.ready()?;
    while !owner.stopped() {
        if monitor.sleeping()? {
            eprintln!("NumberPad stopped for sleep; illumination and input released.");
            return Ok(());
        }
        let mut fds = [
            libc::pollfd {
                fd: hardware.touch.as_ref().unwrap().fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: hardware.switch.as_ref().unwrap().fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 50) };
        if rc < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        for (index, fd) in fds.iter().enumerate() {
            if fd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                bail!("Selected input device disconnected");
            }
            if fd.revents & libc::POLLIN == 0 {
                continue;
            }
            let events = if index == 0 {
                hardware.touch.as_ref().unwrap().events()?
            } else {
                hardware.switch.as_ref().unwrap().events()?
            };
            for event in events {
                if event.type_ == 0 && event.code == 3 {
                    hardware.actions(engine.reset(), &mapping)?;
                    if index == 0 {
                        dropped = true;
                    } else {
                        switch_dropped = true;
                        tablet = true;
                    }
                    continue;
                }
                if index == 1 {
                    if event.type_ == 5 && event.code == 1 && !switch_dropped {
                        tablet = event.value != 0;
                        if tablet {
                            hardware.actions(engine.reset(), &mapping)?;
                        }
                    } else if event.type_ == 0 && event.code == 0 {
                        tablet = hardware.switch.as_ref().unwrap().tablet()?;
                        switch_dropped = false;
                    }
                    continue;
                }
                if event.type_ == 0 && event.code == 0 {
                    if dropped {
                        while !hardware.touch.as_ref().unwrap().events()?.is_empty() {}
                        slots.resync(hardware.touch.as_ref().unwrap())?;
                        dropped = false;
                        engine.blocked = true;
                        break;
                    }
                    hardware.actions(
                        engine.frame(&slots.contacts(), start.elapsed().as_secs_f64(), !tablet),
                        &mapping,
                    )?;
                } else if event.type_ == 3 && !dropped {
                    slots.event(event.code, event.value)?;
                }
            }
        }
        if check.elapsed() >= Duration::from_millis(500) {
            if !session::active()? {
                eprintln!("NumberPad stopped: login inactive or locked.");
                return Ok(());
            }
            let current = keymap::source()?;
            if current != source {
                eprintln!("NumberPad stopped: session keymap changed; restart with the new map.");
                return Ok(());
            }
            check = Instant::now();
        }
        if !dropped {
            hardware.actions(
                engine.frame(&slots.contacts(), start.elapsed().as_secs_f64(), !tablet),
                &mapping,
            )?;
        }
    }
    owner.stopping.store(true, Ordering::SeqCst);
    Ok(())
}
pub fn recover(missing_ok: bool) -> Result<()> {
    let _lock = control::Lock::acquire("runtime")?;
    recover_locked(missing_ok)
}
pub fn recover_locked(missing_ok: bool) -> Result<()> {
    #[cfg(feature = "test-fixtures")]
    if crate::fixtures::enabled() {
        if std::path::Path::new("/tmp/numberpad-fixture/fail-recovery").exists() {
            bail!("Fixture illumination recovery failed");
        }
        return Ok(());
    }
    match discovery::probe() {
        Ok(device) => linux::light(&device, false)?,
        Err(_) if missing_ok => {
            eprintln!(
                "No unique present hardware: illumination cannot be verified; revoking only owned grants."
            );
        }
        Err(error) => return Err(error),
    }
    Ok(())
}
