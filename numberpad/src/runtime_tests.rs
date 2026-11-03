use super::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Clone, PartialEq)]
enum Observed {
    Action(Action),
    Closed,
}
struct Faults {
    trace: Rc<RefCell<Vec<Observed>>>,
    fail: Option<Action>,
}
impl ActionSink for Faults {
    fn action(&mut self, action: Action, _: &keymap::Mapping) -> Result<()> {
        self.trace
            .borrow_mut()
            .push(Observed::Action(action.clone()));
        if self.fail == Some(action) {
            bail!("injected output failure");
        }
        Ok(())
    }
    fn close_input(&mut self) {
        self.trace.borrow_mut().push(Observed::Closed);
    }
}
fn faults(fail: Option<Action>) -> (Hardware<Faults>, Rc<RefCell<Vec<Observed>>>) {
    let trace = Rc::new(RefCell::new(vec![]));
    (
        Hardware(Faults {
            trace: trace.clone(),
            fail,
        }),
        trace,
    )
}
#[test]
fn activation_stops_at_failed_grab_then_closes_input_before_light() {
    let (mut hardware, trace) = faults(Some(Action::Grab(true)));
    assert!(
        hardware
            .actions(
                vec![Action::Grab(true), Action::Light(true)],
                &keymap::Mapping::new()
            )
            .is_err()
    );
    drop(hardware);
    assert_eq!(
        *trace.borrow(),
        vec![
            Observed::Action(Action::Grab(true)),
            Observed::Closed,
            Observed::Action(Action::Light(false))
        ]
    );
}
#[test]
fn cleanup_faults_still_close_kernel_input_before_retrying_light() {
    for failure in [Action::Key(None), Action::Grab(false), Action::Light(false)] {
        let (mut hardware, trace) = faults(Some(failure));
        let mut engine = Engine::new((0, 2000, 0, 1200), Settings::default()).unwrap();
        assert!(
            hardware
                .actions(engine.off(), &keymap::Mapping::new())
                .is_err()
        );
        drop(hardware);
        let trace = trace.borrow();
        assert_eq!(trace[0], Observed::Action(Action::Key(None)));
        assert_eq!(trace[1], Observed::Action(Action::Grab(false)));
        let close = trace.iter().position(|v| *v == Observed::Closed).unwrap();
        assert_eq!(trace[close + 1], Observed::Action(Action::Light(false)));
    }
}
#[test]
fn normal_off_and_panic_cleanup_release_pointer_before_i2c() {
    let (mut hardware, trace) = faults(None);
    let mut engine = Engine::new((0, 2000, 0, 1200), Settings::default()).unwrap();
    hardware
        .actions(engine.off(), &keymap::Mapping::new())
        .unwrap();
    assert_eq!(
        &*trace.borrow(),
        &[
            Observed::Action(Action::Key(None)),
            Observed::Action(Action::Grab(false)),
            Observed::Action(Action::Light(false))
        ]
    );
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _hardware = hardware;
        panic!("simulated runtime panic");
    }));
    assert!(result.is_err());
    assert_eq!(
        &trace.borrow()[3..],
        &[Observed::Closed, Observed::Action(Action::Light(false))]
    );
}
fn event(kind: u16, code: u16, value: i32) -> libc::input_event {
    let mut event: libc::input_event = unsafe { std::mem::zeroed() };
    event.type_ = kind;
    event.code = code;
    event.value = value;
    event
}
fn state() -> EventState {
    EventState {
        slots: Slots {
            count: 2,
            selector: 0,
            ids: vec![-1, -1],
            x: vec![0, 0],
            y: vec![0, 0],
        },
        engine: Engine::new((0, 2000, 0, 1200), Settings::default()).unwrap(),
        tablet: false,
        touch_dropped: false,
        switch_dropped: false,
    }
}
fn touch(state: &mut EventState, kind: u16, code: u16, value: i32, time: f64) -> Vec<Action> {
    match state.touch(event(kind, code, value), time).unwrap() {
        TouchUpdate::Actions(actions) => actions,
        TouchUpdate::Resync => panic!("unexpected resync"),
    }
}
fn contact(state: &mut EventState, id: i32, x: i32, y: i32, time: f64) -> Vec<Action> {
    touch(state, 3, 57, id, time);
    touch(state, 3, 53, x, time);
    touch(state, 3, 54, y, time);
    touch(state, 0, 0, 0, time)
}
fn lift(state: &mut EventState, time: f64) -> Vec<Action> {
    touch(state, 3, 57, -1, time);
    touch(state, 0, 0, 0, time)
}
fn tap(state: &mut EventState, time: f64) -> Vec<Action> {
    contact(state, 99, 1900, 50, time);
    lift(state, time + 0.05)
}
#[test]
fn fold_releases_held_key_and_unfold_requires_lift_and_fresh_gesture() {
    let mut state = state();
    tap(&mut state, 0.);
    tap(&mut state, 0.15);
    assert!(state.engine.active);
    assert_eq!(
        contact(&mut state, 10, 360, 315, 1.),
        vec![Action::Key(Some("7"))]
    );
    let reset = state.switch(event(5, 1, 1), || unreachable!()).unwrap();
    assert_eq!(
        reset,
        vec![Action::Key(None), Action::Grab(false), Action::Light(false)]
    );
    assert!(!state.engine.active);
    state.switch(event(5, 1, 0), || unreachable!()).unwrap();
    assert!(state.frame(2.).is_empty());
    contact(&mut state, 10, 1900, 50, 2.1);
    lift(&mut state, 2.15);
    assert!(
        !state.engine.active,
        "contact spanning folding must not activate"
    );
    tap(&mut state, 3.);
    assert!(!state.engine.active);
    tap(&mut state, 3.15);
    assert!(state.engine.active);
}
#[test]
fn fold_cancels_half_double_tap_and_switch_drop_is_fail_closed() {
    let mut state = state();
    tap(&mut state, 0.);
    state.switch(event(5, 1, 1), || unreachable!()).unwrap();
    state.switch(event(5, 1, 0), || unreachable!()).unwrap();
    tap(&mut state, 0.2);
    assert!(!state.engine.active);
    lift(&mut state, 0.3);
    state.engine.active = true;
    let reset = state.switch(event(0, 3, 0), || unreachable!()).unwrap();
    assert!(reset.contains(&Action::Grab(false)));
    state.switch(event(5, 1, 0), || unreachable!()).unwrap();
    assert!(state.tablet && state.switch_dropped);
    assert!(state.frame(1.).is_empty());
    state.switch(event(0, 0, 0), || Ok(false)).unwrap();
    assert!(!state.switch_dropped && !state.tablet && !state.engine.active);
    // An ioctl failure must propagate to the guard instead of guessing state.
    assert!(
        state
            .switch(event(0, 0, 0), || anyhow::bail!("switch read failed"))
            .is_err()
    );
}
#[test]
fn dropped_touch_ignores_stale_frames_and_resync_requires_all_fingers_up() {
    let mut state = state();
    state.engine.active = true;
    contact(&mut state, 10, 360, 315, 0.);
    assert!(touch(&mut state, 0, 3, 0, 0.1).contains(&Action::Key(None)));
    touch(&mut state, 3, 57, -1, 0.15);
    assert_eq!(state.slots.ids[0], 10, "stale dropped data was applied");
    assert!(matches!(
        state.touch(event(0, 0, 0), 0.2).unwrap(),
        TouchUpdate::Resync
    ));
    state.slots.ids = vec![20, 21];
    state.slots.x = vec![1900, 1850];
    state.slots.y = vec![50, 50];
    state.resynced();
    assert!(state.frame(0.3).is_empty());
    state.slots.ids[1] = -1;
    assert!(state.frame(0.4).is_empty());
    state.slots.ids[0] = -1;
    state.frame(0.5);
    tap(&mut state, 0.6);
    assert!(!state.engine.active);
    tap(&mut state, 0.75);
    assert!(state.engine.active);
}
#[test]
fn malformed_slot_and_continuous_drop_drain_are_bounded_errors() {
    let mut state = state();
    for slot in [-1, 2, i32::MAX] {
        assert!(state.touch(event(3, 47, slot), 0.).is_err());
    }
    let mut reads = 0;
    let error = drain_events(|| {
        reads += 1;
        Ok(vec![event(0, 0, 0)])
    })
    .unwrap_err();
    assert!(error.to_string().contains("did not quiesce"));
    assert_eq!(reads, 64);
    let mut reads = 0;
    drain_events(|| {
        reads += 1;
        Ok(if reads == 3 {
            vec![]
        } else {
            vec![event(0, 0, 0)]
        })
    })
    .unwrap();
    assert_eq!(reads, 3);
}

#[test]
fn startup_resets_illumination_and_never_restores_active_state() {
    for fail in [None, Some(Action::Light(false))] {
        let (mut hardware, trace) = faults(fail.clone());
        let result = hardware.actions(vec![Action::Light(false)], &keymap::Mapping::new());
        assert_eq!(result.is_err(), fail.is_some());
        assert!(!state().engine.active);
        drop(hardware);
        assert!(trace.borrow().iter().all(|event| !matches!(
            event,
            Observed::Action(Action::Light(true) | Action::Grab(true) | Action::Key(Some(_)))
        )));
    }
}
