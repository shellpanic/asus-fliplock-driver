use crate::{session, util};
use anyhow::{Result, bail};
use std::{
    collections::BTreeMap,
    ffi::{CString, c_char, c_void},
    time::Duration,
};
pub type Mapping = BTreeMap<String, Vec<u16>>;
#[link(name = "libxkbcommon.so.0", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn xkb_context_new(flags: u32) -> *mut c_void;
    fn xkb_context_unref(ctx: *mut c_void);
    fn xkb_keymap_new_from_string(
        ctx: *mut c_void,
        text: *const c_char,
        format: u32,
        flags: u32,
    ) -> *mut c_void;
    fn xkb_keymap_unref(map: *mut c_void);
    fn xkb_keymap_num_layouts(map: *mut c_void) -> u32;
    fn xkb_state_new(map: *mut c_void) -> *mut c_void;
    fn xkb_state_unref(state: *mut c_void);
    fn xkb_state_update_mask(
        state: *mut c_void,
        a: u32,
        b: u32,
        c: u32,
        d: u32,
        e: u32,
        f: u32,
    ) -> u32;
    fn xkb_state_update_key(state: *mut c_void, key: u32, direction: u32) -> u32;
    fn xkb_state_key_get_one_sym(state: *mut c_void, key: u32) -> u32;
    fn xkb_state_key_get_utf32(state: *mut c_void, key: u32) -> u32;
}
struct Context(*mut c_void);
impl Drop for Context {
    fn drop(&mut self) {
        unsafe { xkb_context_unref(self.0) }
    }
}
struct Map(*mut c_void);
impl Drop for Map {
    fn drop(&mut self) {
        unsafe { xkb_keymap_unref(self.0) }
    }
}
struct State(*mut c_void);
impl Drop for State {
    fn drop(&mut self) {
        unsafe { xkb_state_unref(self.0) }
    }
}
pub fn source() -> Result<Vec<u8>> {
    let kind = session::display_kind()?;
    util::output(
        "xkbcli",
        &[
            &format!("dump-keymap-{kind}"),
            if kind == "x11" {
                "--format=v1"
            } else {
                "--raw"
            },
        ],
        Duration::from_secs(2),
    )
}
pub fn resolve(source: &[u8]) -> Result<Mapping> {
    let text = CString::new(source)?;
    let ctx = Context(unsafe { xkb_context_new(0) });
    if ctx.0.is_null() {
        bail!("Cannot create XKB context");
    }
    let map = Map(unsafe { xkb_keymap_new_from_string(ctx.0, text.as_ptr(), 1, 0) });
    if map.0.is_null() {
        bail!("Cannot parse actual session keymap");
    }
    if unsafe { xkb_keymap_num_layouts(map.0) } != 1 {
        bail!("Multiple XKB groups need active-group tracking; refusing activation");
    }
    let state = State(unsafe { xkb_state_new(map.0) });
    if state.0.is_null() {
        bail!("Cannot create XKB state");
    }
    let mut shifts = vec![];
    let mut thirds = vec![];
    for key in 9..256 {
        match unsafe { xkb_state_key_get_one_sym(state.0, key) } {
            0xffe1 | 0xffe2 => shifts.push(key),
            0xfe03 | 0xff7e => thirds.push(key),
            _ => {}
        }
    }
    let mut combos = vec![vec![]];
    combos.extend(shifts.iter().chain(&thirds).map(|key| vec![*key]));
    for shift in &shifts {
        for third in &thirds {
            combos.push(vec![*shift, *third]);
        }
    }
    let mut result = Mapping::new();
    for modifiers in combos {
        unsafe {
            xkb_state_update_mask(state.0, 0, 0, 0, 0, 0, 0);
        }
        for key in &modifiers {
            unsafe {
                xkb_state_update_key(state.0, *key, 1);
            }
        }
        for key in 9..256 {
            if [71, 72, 73, 75, 76, 77, 79, 80, 81, 82, 83, 95, 121].contains(&(key - 8))
                || modifiers.contains(&key)
            {
                continue;
            }
            if let Some(ch) = char::from_u32(unsafe { xkb_state_key_get_utf32(state.0, key) })
                && "0123456789/*-%.+=".contains(ch)
            {
                let mut codes: Vec<_> = modifiers.iter().map(|k| (*k - 8) as u16).collect();
                codes.push((key - 8) as u16);
                result.entry(ch.to_string()).or_insert(codes);
            }
        }
        for key in modifiers.iter().rev() {
            unsafe {
                xkb_state_update_key(state.0, *key, 0);
            }
        }
    }
    if "0123456789/*-%.+="
        .chars()
        .any(|ch| !result.contains_key(&ch.to_string()))
    {
        bail!("Session keymap cannot produce every printed b7402 symbol");
    }
    unsafe {
        xkb_state_update_mask(state.0, 0, 0, 0, 0, 0, 0);
    }
    for (name, sym) in [("backspace", 0xff08), ("enter", 0xff0d)] {
        let key = (9..256)
            .find(|key| unsafe { xkb_state_key_get_one_sym(state.0, *key) } == sym)
            .ok_or_else(|| anyhow::anyhow!("Session keymap lacks {name}"))?;
        result.insert(name.into(), vec![(key - 8) as u16]);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[repr(C)]
    struct Names {
        rules: *const c_char,
        model: *const c_char,
        layout: *const c_char,
        variant: *const c_char,
        options: *const c_char,
    }
    unsafe extern "C" {
        fn xkb_keymap_new_from_names(
            ctx: *mut c_void,
            names: *const Names,
            flags: u32,
        ) -> *mut c_void;
        fn xkb_keymap_get_as_string(map: *mut c_void, format: u32) -> *mut c_char;
    }
    fn layout(name: &str) -> Vec<u8> {
        let name = CString::new(name).unwrap();
        let null = std::ptr::null();
        let names = Names {
            rules: null,
            model: null,
            layout: name.as_ptr(),
            variant: null,
            options: null,
        };
        let ctx = Context(unsafe { xkb_context_new(0) });
        let map = Map(unsafe { xkb_keymap_new_from_names(ctx.0, &names, 0) });
        assert!(!map.0.is_null());
        let text = unsafe { xkb_keymap_get_as_string(map.0, 1) };
        assert!(!text.is_null());
        let result = unsafe { std::ffi::CStr::from_ptr(text) }
            .to_bytes()
            .to_vec();
        unsafe { libc::free(text.cast()) };
        result
    }
    #[test]
    fn real_us_de_fr_maps_and_multigroup_refusal() {
        for name in ["us", "de", "fr"] {
            let mapping = resolve(&layout(name)).unwrap();
            assert_eq!(mapping.len(), 19);
            assert!(mapping.values().all(|keys| !keys.is_empty()));
            if name == "fr" {
                assert!(mapping["1"].len() > 1);
            }
        }
        assert!(
            resolve(&layout("us,de"))
                .unwrap_err()
                .to_string()
                .contains("Multiple XKB")
        );
    }
    #[test]
    fn malformed_map_refused() {
        assert!(resolve(b"invalid").is_err());
    }
}
