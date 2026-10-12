//! "Start with Windows": a value in
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (current user only,
//! no admin rights). Off unless the user turns it on.

use std::io;
use std::path::Path;

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Name of the value the app uses.
pub const VALUE_NAME: &str = "FileOrganizer";

/// `"C:\path\file-organizer.exe" --tray`
pub fn command_for(exe: &Path) -> String {
    format!("\"{}\" --tray", exe.display())
}

/// Turns "Start with Windows" on (writes the value) or off (removes it).
pub fn set_enabled(enabled: bool, exe: &Path) -> io::Result<()> {
    if enabled {
        set_run_value(VALUE_NAME, &command_for(exe))
    } else {
        remove_run_value(VALUE_NAME)
    }
}

/// True if the Run value exists.
pub fn is_enabled() -> bool {
    get_run_value(VALUE_NAME).ok().flatten().is_some()
}

#[cfg(windows)]
mod sys {
    use super::RUN_KEY;
    use std::io;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn check(code: u32) -> io::Result<()> {
        if code == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(code as i32))
        }
    }

    pub fn set(name: &str, data: &str) -> io::Result<()> {
        let (key, name, data) = (wide(RUN_KEY), wide(name), wide(data));
        let mut hkey: HKEY = std::ptr::null_mut();
        // SAFETY: valid NUL-terminated strings; the key is closed below.
        unsafe {
            check(RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut hkey,
                std::ptr::null_mut(),
            ))?;
            let r = check(RegSetValueExW(
                hkey,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                (data.len() * 2) as u32,
            ));
            RegCloseKey(hkey);
            r
        }
    }

    pub fn remove(name: &str) -> io::Result<()> {
        let (key, name) = (wide(RUN_KEY), wide(name));
        // SAFETY: valid NUL-terminated strings.
        let code = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(()); // already off
        }
        check(code)
    }

    pub fn get(name: &str) -> io::Result<Option<String>> {
        let (key, name) = (wide(RUN_KEY), wide(name));
        let mut buf = vec![0u16; 2048];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: buffer size is passed in bytes.
        let code = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut _,
                &mut bytes,
            )
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(code)?;
        let len = (bytes as usize / 2).min(buf.len());
        let s = String::from_utf16_lossy(&buf[..len]);
        Ok(Some(s.trim_end_matches('\0').to_string()))
    }
}

#[cfg(not(windows))]
mod sys {
    use std::io;
    fn no() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "Start with Windows only works on Windows")
    }
    pub fn set(_name: &str, _data: &str) -> io::Result<()> {
        Err(no())
    }
    pub fn remove(_name: &str) -> io::Result<()> {
        Err(no())
    }
    pub fn get(_name: &str) -> io::Result<Option<String>> {
        Err(no())
    }
}

/// Writes a value under the Run key (tests use their own value name).
pub fn set_run_value(name: &str, command: &str) -> io::Result<()> {
    sys::set(name, command)
}

/// Removes a value under the Run key; a missing value is fine.
pub fn remove_run_value(name: &str) -> io::Result<()> {
    sys::remove(name)
}

/// Reads a value under the Run key (`None` if it does not exist).
pub fn get_run_value(name: &str) -> io::Result<Option<String>> {
    sys::get(name)
}
