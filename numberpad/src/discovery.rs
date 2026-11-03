use crate::util::read;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub const MODEL: &str = "ASUS EXPERTBOOK B5402FVA_B5402FVA";
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Switch {
    pub event: PathBuf,
    pub sysfs: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub model: String,
    pub event: PathBuf,
    pub sysfs: PathBuf,
    pub name: String,
    pub adapter: PathBuf,
    pub adapter_sysfs: PathBuf,
    pub vendor: String,
    pub product: String,
    pub profile: String,
    pub switch: Option<Switch>,
    pub uinput: PathBuf,
    pub physical_acceptance: String,
}
pub fn probe() -> Result<Device> {
    #[cfg(feature = "test-fixtures")]
    if crate::fixtures::enabled() {
        return discover(
            Path::new("/tmp/numberpad-fixture/sys"),
            Path::new("/dev/numberpad-fixture"),
            true,
        );
    }
    discover(Path::new("/sys"), Path::new("/dev"), true)
}
pub fn discover(sys: &Path, dev: &Path, require_switch: bool) -> Result<Device> {
    let model = read(sys.join("class/dmi/id/product_name"))?;
    let mut matches = Vec::new();
    let mut switches = Vec::new();
    let mut entries =
        fs::read_dir(sys.join("class/input"))?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let filename = entry.file_name();
        let filename = filename.to_string_lossy();
        if !filename.starts_with("event") {
            continue;
        }
        let device = entry.path().join("device");
        let parse = || -> Result<_> {
            Ok((
                read(device.join("name"))?,
                read(device.join("id/vendor"))?.to_uppercase(),
                read(device.join("id/product"))?.to_uppercase(),
                bits(&read(device.join("capabilities/ev"))?)?,
                bits(&read(device.join("capabilities/sw"))?)?,
            ))
        };
        let Ok((name, vendor, product, ev, sw)) = parse() else {
            continue;
        };
        if sw & 2 != 0 && ev & 2 == 0 {
            switches.push(Switch {
                event: dev.join("input").join(filename.as_ref()),
                sysfs: fs::canonicalize(entry.path())?,
            });
        }
        if model != MODEL
            || vendor != "04F3"
            || product != "3230"
            || !name.starts_with("ASUE")
            || !name.ends_with("Touchpad")
        {
            continue;
        }
        let physical = fs::canonicalize(&device)?;
        let adapters: Vec<_> = physical
            .ancestors()
            .skip(1)
            .filter(|path| {
                path.file_name().and_then(|x| x.to_str()).is_some_and(|s| {
                    s.strip_prefix("i2c-")
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
                })
            })
            .collect();
        if adapters.len() != 1 {
            continue;
        }
        let adapter_name = adapters[0].file_name().unwrap();
        let adapter_sysfs = sys.join("class/i2c-dev").join(adapter_name);
        matches.push(Device {
            model: model.clone(),
            event: dev.join("input").join(filename.as_ref()),
            sysfs: fs::canonicalize(entry.path())?,
            name,
            adapter: dev.join(adapter_name),
            adapter_sysfs: fs::canonicalize(&adapter_sysfs).unwrap_or(adapter_sysfs),
            vendor,
            product,
            profile: "b7402".into(),
            switch: None,
            uinput: dev.join("uinput"),
            physical_acceptance: "pending-rust-acceptance".into(),
        });
    }
    if matches.len() != 1 {
        bail!(
            "Refusing hardware: expected one exact {MODEL}/04F3:3230 touchpad; found {}",
            matches.len()
        );
    }
    if require_switch && switches.len() != 1 {
        bail!(
            "Refusing hardware: expected one isolated tablet switch without keyboard keys; found {}",
            switches.len()
        );
    }
    let mut device = matches.pop().unwrap();
    if switches.len() == 1 {
        device.switch = switches.pop();
    }
    Ok(device)
}
fn bits(value: &str) -> Result<u128> {
    u128::from_str_radix(&value.replace(' ', ""), 16).context("Invalid device capabilities")
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    fn fixture(root: &Path) {
        let sys = root.join("sys");
        fs::create_dir_all(sys.join("class/dmi/id")).unwrap();
        fs::write(sys.join("class/dmi/id/product_name"), MODEL).unwrap();
        fs::create_dir_all(sys.join("class/input")).unwrap();
        fs::create_dir_all(sys.join("class/i2c-dev")).unwrap();
        for (event, name, ev, sw) in [
            ("event91", "ASUE1234:00 04F3:3230 Touchpad", "b", "0"),
            ("event12", "Intel HID switches", "21", "2"),
        ] {
            let parent = sys.join("devices/pci/i2c-57").join(event);
            fs::create_dir_all(parent.join("device/id")).unwrap();
            fs::create_dir_all(parent.join("device/capabilities")).unwrap();
            for (file, value) in [
                ("name", name),
                ("id/vendor", "04f3"),
                ("id/product", "3230"),
                ("capabilities/ev", ev),
                ("capabilities/sw", sw),
            ] {
                fs::write(parent.join("device").join(file), value).unwrap();
            }
            symlink(&parent, sys.join("class/input").join(event)).unwrap();
        }
        symlink(
            sys.join("devices/pci/i2c-57"),
            sys.join("class/i2c-dev/i2c-57"),
        )
        .unwrap();
    }
    #[test]
    fn exact_paths_and_refusals() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let sys = tmp.path().join("sys");
        let dev = Path::new("/fixture/dev");
        let found = discover(&sys, dev, true).unwrap();
        assert_eq!(found.event, dev.join("input/event91"));
        assert_eq!(found.switch.unwrap().event, dev.join("input/event12"));
        assert_eq!(found.adapter, dev.join("i2c-57"));
        assert_eq!(found.uinput, dev.join("uinput"));
        fs::write(sys.join("class/input/event12/device/capabilities/ev"), "23").unwrap();
        assert!(discover(&sys, dev, true).is_err());
        fs::write(sys.join("class/input/event12/device/capabilities/ev"), "21").unwrap();
        symlink(
            fs::canonicalize(sys.join("class/input/event91")).unwrap(),
            sys.join("class/input/event92"),
        )
        .unwrap();
        assert!(discover(&sys, dev, true).is_err());
    }
}
