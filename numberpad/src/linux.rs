use anyhow::{Result, bail};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    mem,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};
fn request(direction: u64, kind: u8, nr: u8, size: usize) -> libc::c_ulong {
    ((direction << 30) | ((size as u64) << 16) | ((kind as u64) << 8) | nr as u64) as _
}
fn ioctl<T>(fd: i32, request: libc::c_ulong, value: *mut T) -> Result<i32> {
    let result = unsafe { libc::ioctl(fd, request, value) };
    if result < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(result)
}
pub struct Input {
    file: File,
    grabbed: bool,
}
impl Input {
    pub fn open(path: &Path, sysfs: &Path, touchpad: bool) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?;
        let info = file.metadata()?;
        let expected = crate::util::read(sysfs.join("dev"))?;
        let (major, minor) = expected
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("Invalid input node identity"))?;
        if info.mode() & libc::S_IFMT != libc::S_IFCHR
            || libc::major(info.rdev()) != major.parse::<u32>()?
            || libc::minor(info.rdev()) != minor.parse::<u32>()?
        {
            bail!("Input node identity changed");
        }
        if touchpad {
            let mut id: libc::input_id = unsafe { mem::zeroed() };
            ioctl(
                file.as_raw_fd(),
                request(2, b'E', 2, mem::size_of_val(&id)),
                &mut id,
            )?;
            if id.vendor != 0x04f3 || id.product != 0x3230 {
                bail!("Touchpad identity changed");
            }
        } else {
            let mut keys = [0u8; 128];
            ioctl(
                file.as_raw_fd(),
                request(2, b'E', 0x21, keys.len()),
                keys.as_mut_ptr(),
            )?;
            if keys.iter().any(|x| *x != 0) {
                bail!("Tablet switch now exposes keyboard keys");
            }
        }
        Ok(Self {
            file,
            grabbed: false,
        })
    }
    pub fn fd(&self) -> i32 {
        self.file.as_raw_fd()
    }
    pub fn abs(&self, code: u8) -> Result<libc::input_absinfo> {
        let mut info = unsafe { mem::zeroed() };
        ioctl(
            self.fd(),
            request(2, b'E', 0x40 + code, mem::size_of::<libc::input_absinfo>()),
            &mut info,
        )?;
        Ok(info)
    }
    pub fn grab(&mut self, on: bool) -> Result<()> {
        if self.grabbed != on {
            let result =
                unsafe { libc::ioctl(self.fd(), request(1, b'E', 0x90, 4), on as libc::c_int) };
            if result < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            self.grabbed = on;
        }
        Ok(())
    }
    pub fn tablet(&self) -> Result<bool> {
        let mut bits = [0u8; 8];
        ioctl(
            self.fd(),
            request(2, b'E', 0x1b, bits.len()),
            bits.as_mut_ptr(),
        )?;
        Ok(bits[0] & 2 != 0)
    }
    pub fn events(&self) -> Result<Vec<libc::input_event>> {
        let mut events: [libc::input_event; 64] = unsafe { mem::zeroed() };
        let size = mem::size_of_val(&events);
        let result = unsafe { libc::read(self.fd(), events.as_mut_ptr().cast(), size) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(vec![]);
            }
            return Err(error.into());
        }
        if result == 0 {
            bail!("Input device disconnected");
        }
        if !(result as usize).is_multiple_of(mem::size_of::<libc::input_event>()) {
            bail!("Partial input event");
        }
        Ok(events[..result as usize / mem::size_of::<libc::input_event>()].to_vec())
    }
    pub fn slots(&self, count: usize, code: i32) -> Result<Vec<i32>> {
        let mut values = vec![0i32; count + 1];
        values[0] = code;
        ioctl(
            self.fd(),
            request(2, b'E', 0x0a, values.len() * 4),
            values.as_mut_ptr(),
        )?;
        Ok(values[1..].to_vec())
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        let _ = self.grab(false);
    }
}
#[repr(C)]
struct USetup {
    id: libc::input_id,
    name: [u8; 80],
    ff_effects_max: u32,
}
pub struct Output {
    file: File,
    pressed: Vec<u16>,
    created: bool,
}
impl Output {
    pub fn new(path: &Path, mapping: &crate::keymap::Mapping) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?;
        if file.metadata()?.mode() & libc::S_IFMT != libc::S_IFCHR {
            bail!("uinput is not a character device");
        }
        let mut output = Self {
            file,
            pressed: vec![],
            created: false,
        };
        for (nr, value) in std::iter::once((100, 1)).chain(
            mapping
                .values()
                .flatten()
                .copied()
                .map(|code| (101, code as i32)),
        ) {
            let result =
                unsafe { libc::ioctl(output.file.as_raw_fd(), request(1, b'U', nr, 4), value) };
            if result < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let mut setup: USetup = unsafe { mem::zeroed() };
        setup.id.bustype = 0x06;
        setup.id.vendor = 1;
        setup.id.product = 1;
        let name = b"ASUS NumberPad numbers only";
        setup.name[..name.len()].copy_from_slice(name);
        ioctl(
            output.file.as_raw_fd(),
            request(1, b'U', 3, mem::size_of::<USetup>()),
            &mut setup,
        )?;
        let result = unsafe { libc::ioctl(output.file.as_raw_fd(), request(0, b'U', 1, 0)) };
        if result < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        output.created = true;
        Ok(output)
    }
    fn event(&mut self, kind: u16, code: u16, value: i32) -> Result<()> {
        let mut event: libc::input_event = unsafe { mem::zeroed() };
        event.type_ = kind;
        event.code = code;
        event.value = value;
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&event as *const libc::input_event).cast::<u8>(),
                mem::size_of_val(&event),
            )
        };
        self.file.write_all(bytes)?;
        Ok(())
    }
    pub fn send(&mut self, codes: Option<&Vec<u16>>) -> Result<()> {
        while let Some(code) = self.pressed.last().copied() {
            self.event(1, code, 0)?;
            self.pressed.pop();
        }
        if let Some(codes) = codes {
            for code in codes {
                self.pressed.push(*code);
                self.event(1, *code, 1)?;
            }
        }
        self.event(0, 0, 0)
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.send(None);
        if self.created {
            unsafe {
                libc::ioctl(self.file.as_raw_fd(), request(0, b'U', 2, 0));
            }
        }
    }
}
#[repr(C)]
struct Message {
    addr: u16,
    flags: u16,
    length: u16,
    buffer: *mut u8,
}
#[repr(C)]
struct Transfer {
    messages: *mut Message,
    count: u32,
}
pub struct Light {
    file: File,
}
impl Light {
    pub fn open(device: &crate::discovery::Device) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&device.adapter)?;
        let info = file.metadata()?;
        let expected = crate::util::read(device.adapter_sysfs.join("dev"))?;
        let (major, minor) = expected
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("Invalid I2C node identity"))?;
        if info.mode() & libc::S_IFMT != libc::S_IFCHR
            || libc::major(info.rdev()) != major.parse::<u32>()?
            || libc::minor(info.rdev()) != minor.parse::<u32>()?
        {
            bail!("I2C adapter identity changed");
        }
        Ok(Self { file })
    }
    fn packet(&self, value: u8) -> Result<()> {
        let mut bytes = [5, 0, 0x3d, 3, 6, 0, 7, 0, 0x0d, 0x14, 3, value, 0xad];
        let mut message = Message {
            addr: 0x15,
            flags: 0,
            length: bytes.len() as _,
            buffer: bytes.as_mut_ptr(),
        };
        let mut transfer = Transfer {
            messages: &mut message,
            count: 1,
        };
        let count = ioctl(self.file.as_raw_fd(), 0x0707, &mut transfer)?;
        if count != 1 {
            bail!("Incomplete I2C transfer");
        }
        Ok(())
    }
    pub fn set(&self, on: bool) -> Result<()> {
        if on {
            self.packet(0x60)?;
        }
        self.packet(if on { 1 } else { 0 })
    }
}
pub fn light(device: &crate::discovery::Device, on: bool) -> Result<()> {
    Light::open(device)?.set(on)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ioctl_abi() {
        assert_eq!(request(1, b'E', 0x90, 4), 0x40044590);
        assert_eq!(request(1, b'U', 3, mem::size_of::<USetup>()), 0x405c5503);
        assert_eq!(mem::size_of::<USetup>(), 92);
    }
}
