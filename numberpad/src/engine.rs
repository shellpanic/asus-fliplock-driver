use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
pub const KEYS: [[&str; 5]; 4] = [
    ["7", "8", "9", "/", "backspace"],
    ["4", "5", "6", "*", "backspace"],
    ["1", "2", "3", "-", "%"],
    ["0", ".", "enter", "+", "="],
];
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Activation {
    #[default]
    DoubleTap,
    Hold,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub activation: Activation,
    pub hold_seconds: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            activation: Activation::DoubleTap,
            hold_seconds: 1.0,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !self.hold_seconds.is_finite() || !(0.2..=5.).contains(&self.hold_seconds) {
            bail!("Hold duration must be 0.2 to 5 seconds");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Key(Option<&'static str>),
    Grab(bool),
    Light(bool),
}
#[derive(Debug, Clone, Copy)]
struct Contact {
    id: i32,
    start: f64,
    icon: bool,
    x: i32,
    y: i32,
}
pub struct Engine {
    pub active: bool,
    pub blocked: bool,
    bounds: (i32, i32, i32, i32),
    settings: Settings,
    contact: Option<Contact>,
    pending: Option<f64>,
    down: Option<&'static str>,
}
impl Engine {
    pub fn new(bounds: (i32, i32, i32, i32), settings: Settings) -> Result<Self> {
        settings.validate()?;
        if !matches!(bounds.1.checked_sub(bounds.0), Some(401..=1_000_000))
            || !matches!(bounds.3.checked_sub(bounds.2), Some(501..=1_000_000))
            || bounds.0.checked_add(250).is_none()
            || bounds.2.checked_add(250).is_none()
        {
            bail!("Invalid touchpad bounds");
        }
        Ok(Self {
            active: false,
            blocked: false,
            bounds,
            settings,
            contact: None,
            pending: None,
            down: None,
        })
    }
    pub fn off(&mut self) -> Vec<Action> {
        self.active = false;
        self.contact = None;
        self.pending = None;
        self.down = None;
        vec![Action::Key(None), Action::Light(false), Action::Grab(false)]
    }
    pub fn reset(&mut self) -> Vec<Action> {
        let actions = self.off();
        self.blocked = true;
        actions
    }
    fn toggle(&mut self) -> Vec<Action> {
        if self.active {
            self.off()
        } else {
            self.active = true;
            vec![Action::Grab(true), Action::Light(true)]
        }
    }
    pub fn frame(&mut self, contacts: &[(i32, i32, i32)], now: f64, allowed: bool) -> Vec<Action> {
        if self.pending.is_some_and(|last| now - last > 0.4) {
            self.pending = None;
        }
        if !allowed {
            let actions = if self.active || self.contact.is_some() {
                self.off()
            } else {
                self.pending = None;
                vec![]
            };
            self.blocked = !contacts.is_empty();
            return actions;
        }
        if self.blocked {
            if contacts.is_empty() {
                self.blocked = false;
                self.contact = None;
            }
            return vec![];
        }
        if contacts.len() != 1 {
            let mut actions = vec![];
            if contacts.is_empty()
                && self.settings.activation == Activation::DoubleTap
                && self
                    .contact
                    .is_some_and(|c| c.icon && (0.0..=0.25).contains(&(now - c.start)))
            {
                if self
                    .pending
                    .is_some_and(|last| (0.0..=0.4).contains(&(now - last)))
                {
                    actions = self.toggle();
                    self.pending = None;
                } else {
                    self.pending = Some(now);
                }
            } else if !contacts.is_empty() || self.contact.is_some() {
                self.pending = None;
            }
            if self.down.take().is_some() {
                actions.push(Action::Key(None));
            }
            self.contact = None;
            if !contacts.is_empty() {
                self.blocked = true;
            }
            return actions;
        }
        let (id, x, y) = contacts[0];
        let (xmin, xmax, ymin, ymax) = self.bounds;
        let icon = x >= xmax - 250 && x <= xmax && y >= ymin && y <= ymin + 250;
        if self.contact.is_some_and(|c| c.id != id) {
            self.pending = None;
            self.blocked = true;
            self.contact = None;
            return vec![Action::Key(None)];
        }
        if self.contact.is_none() {
            if !icon {
                self.pending = None;
            }
            self.contact = Some(Contact {
                id,
                start: now,
                icon,
                x,
                y,
            });
        }
        let contact = self.contact.unwrap();
        if contact.icon && (x.abs_diff(contact.x) > 60 || y.abs_diff(contact.y) > 60) {
            self.pending = None;
            self.blocked = true;
            return vec![];
        }
        if icon && contact.icon {
            if self.settings.activation == Activation::Hold
                && now - contact.start >= self.settings.hold_seconds
            {
                let actions = self.toggle();
                self.blocked = true;
                return actions;
            }
            return vec![];
        }
        if contact.icon {
            self.pending = None;
            self.blocked = true;
            return vec![];
        }
        let key =
            if self.active && x >= xmin + 200 && x < xmax - 200 && y >= ymin + 200 && y < ymax - 80
            {
                let col = ((x - xmin - 200) * 5 / (xmax - xmin - 400)).clamp(0, 4) as usize;
                let row = ((y - ymin - 200) * 4 / (ymax - ymin - 280)).clamp(0, 3) as usize;
                Some(KEYS[row][col])
            } else {
                None
            };
        if key != self.down {
            self.down = key;
            vec![Action::Key(key)]
        } else {
            vec![]
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn engine(activation: Activation) -> Engine {
        Engine::new(
            (0, 2000, 0, 1200),
            Settings {
                activation,
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn tap(e: &mut Engine, t: f64, id: i32) -> Vec<Action> {
        e.frame(&[(id, 1900, 50)], t, true);
        e.frame(&[], t + 0.05, true)
    }
    #[test]
    fn double_tap_lifts_and_off() {
        let mut e = engine(Activation::DoubleTap);
        tap(&mut e, 0., 1);
        assert!(!e.active);
        e.frame(&[(2, 1900, 50)], 0.15, true);
        assert!(!e.active);
        assert_eq!(
            e.frame(&[], 0.2, true),
            vec![Action::Grab(true), Action::Light(true)]
        );
        tap(&mut e, 1., 3);
        let actions = tap(&mut e, 1.15, 4);
        assert!(!e.active);
        assert!(actions.contains(&Action::Grab(false)));
    }
    #[test]
    fn hold_requires_lift_and_reset() {
        let mut e = engine(Activation::Hold);
        e.frame(&[(1, 1900, 50)], 0., true);
        e.frame(&[(1, 1900, 50)], 1.01, true);
        assert!(e.active);
        e.frame(&[(1, 1900, 50)], 5., true);
        assert!(e.active);
        e.frame(&[], 6., true);
        e.frame(&[(2, 1900, 50)], 7., true);
        e.frame(&[(2, 1900, 50)], 8.01, true);
        assert!(!e.active);
    }
    #[test]
    fn incomplete_gestures_refused() {
        let mut e = engine(Activation::DoubleTap);
        tap(&mut e, 0., 1);
        tap(&mut e, 1., 2);
        assert!(!e.active);
        e.frame(&[(3, 1900, 50)], 2., true);
        e.frame(&[(3, 1900, 50)], 3., true);
        e.frame(&[], 3.1, true);
        assert!(!e.active);
        e.reset();
        e.frame(&[], 4., true);
        tap(&mut e, 4.1, 4);
        assert!(!e.active);
        e.frame(&[(5, 1900, 50), (6, 1800, 50)], 4.2, true);
        e.frame(&[], 4.25, true);
        tap(&mut e, 4.3, 7);
        assert!(!e.active);
    }
    #[test]
    fn every_printed_cell_and_cleanup() {
        let mut e = engine(Activation::DoubleTap);
        tap(&mut e, 0., 1);
        tap(&mut e, 0.15, 2);
        assert!(e.active);
        for (row, keys) in KEYS.iter().enumerate() {
            for (col, key) in keys.iter().enumerate() {
                let actions = e.frame(
                    &[(
                        10,
                        200 + (col as i32 * 320) + 160,
                        200 + (row as i32 * 230) + 115,
                    )],
                    1.,
                    true,
                );
                assert_eq!(actions, vec![Action::Key(Some(key))]);
                assert_eq!(e.frame(&[], 1.05, true), vec![Action::Key(None)]);
            }
        }
        assert!(e.frame(&[], 2., false).contains(&Action::Grab(false)));
        assert!(!e.active);
    }
}
