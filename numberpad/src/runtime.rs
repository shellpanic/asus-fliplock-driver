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
// Decode real evdev frames independently of Linux file descriptors. The same
// state machine is exercised by synthetic fold, dropped-frame and fault tests.
struct EventState {
    slots: Slots,
    engine: Engine,
    tablet: bool,
    touch_dropped: bool,
    switch_dropped: bool,
}
enum TouchUpdate {
    Actions(Vec<Action>),
    Resync,
}
impl EventState {
    fn switch(
        &mut self,
        event: libc::input_event,
        current: impl FnOnce() -> Result<bool>,
    ) -> Result<Vec<Action>> {
        if event.type_ == 0 && event.code == 3 {
            self.switch_dropped = true;
            self.tablet = true;
            return Ok(self.engine.reset());
        }
        if event.type_ == 5 && event.code == 1 && !self.switch_dropped {
            self.tablet = event.value != 0;
            if self.tablet {
                return Ok(self.engine.reset());
            }
        } else if event.type_ == 0 && event.code == 0 {
            self.tablet = current()?;
            self.switch_dropped = false;
            if self.tablet {
                return Ok(self.engine.reset());
            }
        }
        Ok(vec![])
    }
    fn touch(&mut self, event: libc::input_event, now: f64) -> Result<TouchUpdate> {
        if event.type_ == 0 && event.code == 3 {
            self.touch_dropped = true;
            return Ok(TouchUpdate::Actions(self.engine.reset()));
        }
        if event.type_ == 0 && event.code == 0 {
            if self.touch_dropped {
                return Ok(TouchUpdate::Resync);
            }
            return Ok(TouchUpdate::Actions(self.frame(now)));
        }
        if event.type_ == 3 && !self.touch_dropped {
            self.slots.event(event.code, event.value)?;
        }
        Ok(TouchUpdate::Actions(vec![]))
    }
    fn frame(&mut self, now: f64) -> Vec<Action> {
        if self.touch_dropped {
            return vec![];
        }
        self.engine.frame(
            &self.slots.contacts(),
            now,
            !self.tablet && !self.switch_dropped,
        )
    }
    fn resynced(&mut self) {
        self.touch_dropped = false;
        self.engine.blocked = true;
    }
}
fn drain_events(mut read: impl FnMut() -> Result<Vec<libc::input_event>>) -> Result<()> {
    for _ in 0..64 {
        if read()?.is_empty() {
            return Ok(());
        }
    }
    bail!("Input stream did not quiesce during dropped-event recovery")
}
// The guard is shared with fault-injection tests: destructive cleanup closes
// input/uinput before I2C, so even a failed ungrab cannot retain pointer control.
trait ActionSink {
    fn action(&mut self, action: Action, mapping: &keymap::Mapping) -> Result<()>;
    fn close_input(&mut self);
}
struct NativeHardware {
    touch: Option<Input>,
    switch: Option<Input>,
    output: Option<Output>,
    light: linux::Light,
}
impl ActionSink for NativeHardware {
    fn action(&mut self, action: Action, mapping: &keymap::Mapping) -> Result<()> {
        match action {
            Action::Key(key) => match &mut self.output {
                Some(output) => output.send(key.and_then(|k| mapping.get(k))),
                None => Ok(()),
            },
            Action::Grab(on) => match &mut self.touch {
                Some(touch) => touch.grab(on),
                None => Ok(()),
            },
            Action::Light(on) => self.light.set(on),
        }
    }
    fn close_input(&mut self) {
        self.output.take();
        self.touch.take();
        self.switch.take();
    }
}
struct Hardware<T: ActionSink>(T);
impl<T: ActionSink> std::ops::Deref for Hardware<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T: ActionSink> std::ops::DerefMut for Hardware<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
impl<T: ActionSink> Hardware<T> {
    fn actions(&mut self, actions: Vec<Action>, mapping: &keymap::Mapping) -> Result<()> {
        let cleanup = actions.iter().all(|action| {
            matches!(
                action,
                Action::Key(None) | Action::Grab(false) | Action::Light(false)
            )
        });
        let mut first = None;
        for action in actions {
            // On a release error, the guard first closes kernel input handles
            // before it retries light-off. Activation must never pass a failed grab.
            if matches!(action, Action::Light(false)) && first.is_some() {
                break;
            }
            if let Err(error) = self.0.action(action, mapping) {
                if !cleanup {
                    return Err(error);
                }
                if first.is_none() {
                    first = Some(error);
                }
            }
        }
        match first {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
impl<T: ActionSink> Drop for Hardware<T> {
    fn drop(&mut self) {
        self.0.close_input();
        if let Err(error) = self.0.action(Action::Light(false), &keymap::Mapping::new()) {
            eprintln!("Illumination cleanup failed: {error}");
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
    let mut hardware = Hardware(NativeHardware {
        touch: None,
        switch: None,
        output: None,
        light: linux::Light::open(&device)?,
    });
    hardware.actions(vec![Action::Light(false)], &mapping)?;
    hardware.touch = Some(Input::open(&device.event, &device.sysfs, true)?);
    let switch = device
        .switch
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing isolated tablet switch"))?;
    hardware.switch = Some(Input::open(&switch.event, &switch.sysfs, false)?);
    let touch = hardware.touch.as_ref().unwrap();
    let slots = Slots::new(touch)?;
    let x = touch.abs(53)?;
    let y = touch.abs(54)?;
    let mut engine = Engine::new((x.minimum, x.maximum, y.minimum, y.maximum), settings)?;
    engine.blocked = !slots.contacts().is_empty();
    hardware.output = Some(Output::new(&device.uinput, &mapping)?);
    let mut state = EventState {
        slots,
        engine,
        tablet: hardware.switch.as_ref().unwrap().tablet()?,
        touch_dropped: false,
        switch_dropped: false,
    };
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
        // A queued fold/drop must take effect before any touch frame from the
        // same poll; touch input may never win this safety decision.
        for index in [1, 0] {
            let fd = &fds[index];
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
                if index == 1 {
                    let actions =
                        state.switch(event, || hardware.switch.as_ref().unwrap().tablet())?;
                    hardware.actions(actions, &mapping)?;
                } else {
                    match state.touch(event, start.elapsed().as_secs_f64())? {
                        TouchUpdate::Actions(actions) => hardware.actions(actions, &mapping)?,
                        TouchUpdate::Resync => {
                            let touch = hardware.touch.as_ref().unwrap();
                            drain_events(|| touch.events())?;
                            state.slots.resync(touch)?;
                            state.resynced();
                            break;
                        }
                    }
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
        hardware.actions(state.frame(start.elapsed().as_secs_f64()), &mapping)?;
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

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
