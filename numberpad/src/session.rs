use crate::util;
use anyhow::{Result, bail};
use std::{collections::BTreeMap, env, time::Duration};

pub fn display_kind() -> Result<&'static str> {
    kind(&env::vars().collect())
}
fn kind(vars: &BTreeMap<String, String>) -> Result<&'static str> {
    let get = |key: &str| vars.get(key).map(String::as_str).unwrap_or("");
    match get("XDG_SESSION_TYPE") {
        "x11" if !get("DISPLAY").is_empty() => Ok("x11"),
        "wayland" if !get("WAYLAND_DISPLAY").is_empty() => Ok("wayland"),
        "tty"
            if get("WAYLAND_DISPLAY").is_empty()
                && ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
                    .iter()
                    .all(|key| get(key).is_empty())
                && local_display(get("DISPLAY")) =>
        {
            Ok("x11")
        }
        _ => bail!(
            "Need a real X11/Wayland session; console-started X11 requires an unambiguous local DISPLAY"
        ),
    }
}
fn local_display(value: &str) -> bool {
    value.strip_prefix(':').is_some_and(|rest| {
        let parts: Vec<_> = rest.split('.').collect();
        (1..=2).contains(&parts.len())
            && parts
                .iter()
                .all(|v| !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit()))
    })
}
pub fn active() -> Result<bool> {
    #[cfg(feature = "test-fixtures")]
    if crate::fixtures::enabled() {
        return Ok(crate::util::read("/tmp/numberpad-fixture/session-state")? == "active");
    }
    if display_kind().is_err() {
        return Ok(false);
    }
    let id = env::var("XDG_SESSION_ID")?;
    let data = util::output(
        "loginctl",
        &[
            "show-session",
            &id,
            "-p",
            "Active",
            "-p",
            "LockedHint",
            "-p",
            "Type",
            "-p",
            "User",
            "-p",
            "Remote",
        ],
        Duration::from_millis(750),
    )?;
    let data = String::from_utf8(data)?;
    let values: BTreeMap<_, _> = data
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let get = |key| values.get(key).copied().unwrap_or("");
    Ok(get("Active") == "yes"
        && get("LockedHint") == "no"
        && get("Remote") == "no"
        && get("User") == util::uid().to_string()
        && get("Type") == env::var("XDG_SESSION_TYPE").unwrap_or_default())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_x11_only() {
        for display in [":0", ":10.1"] {
            assert!(local_display(display));
        }
        for display in ["", ":", ":0.", ":-1", ":0.1.2", "localhost:0", "host:0"] {
            assert!(!local_display(display));
        }
        let mut vars = BTreeMap::from([
            ("XDG_SESSION_TYPE".into(), "tty".into()),
            ("DISPLAY".into(), ":0".into()),
        ]);
        assert_eq!(kind(&vars).unwrap(), "x11");
        for key in ["SSH_CLIENT", "SSH_TTY", "SSH_CONNECTION", "WAYLAND_DISPLAY"] {
            vars.insert(key.into(), "present".into());
            assert!(kind(&vars).is_err());
            vars.remove(key);
        }
    }
}
