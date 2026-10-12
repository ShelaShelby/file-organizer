//! The three roots (found with the Windows Known Folder API), the app-data
//! folder and the list of places that must never be touched.

use std::io;
use std::path::{Component, Path, PathBuf};

/// Desktop files are sent to `Documents\<this>\<Type>\<YYYY-MM>\`.
pub const FROM_DESKTOP: &str = "From Desktop";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RootKind {
    Downloads,
    Desktop,
    Documents,
}

impl RootKind {
    pub const ALL: [RootKind; 3] = [RootKind::Downloads, RootKind::Desktop, RootKind::Documents];

    pub fn label(self) -> &'static str {
        match self {
            RootKind::Downloads => "Downloads",
            RootKind::Desktop => "Desktop",
            RootKind::Documents => "Documents",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    pub downloads: PathBuf,
    pub desktop: PathBuf,
    pub documents: PathBuf,
}

impl Roots {
    /// The current user's Downloads, Desktop and Documents, asked from Windows
    /// (works when they are redirected, e.g. to OneDrive).
    pub fn from_system() -> io::Result<Roots> {
        Ok(Roots {
            downloads: sys::known_folder(sys::Folder::Downloads)?,
            desktop: sys::known_folder(sys::Folder::Desktop)?,
            documents: sys::known_folder(sys::Folder::Documents)?,
        })
    }

    pub fn get(&self, kind: RootKind) -> &Path {
        match kind {
            RootKind::Downloads => &self.downloads,
            RootKind::Desktop => &self.desktop,
            RootKind::Documents => &self.documents,
        }
    }

    /// Where the type folders for files from `kind` are created.
    pub fn target_base(&self, kind: RootKind) -> PathBuf {
        match kind {
            RootKind::Downloads => self.downloads.clone(),
            RootKind::Documents => self.documents.clone(),
            RootKind::Desktop => self.documents.join(FROM_DESKTOP),
        }
    }

    /// Refuse to work if any root is (inside) a protected system place.
    pub fn validate(&self) -> io::Result<()> {
        for kind in RootKind::ALL {
            let p = self.get(kind);
            if !p.is_absolute() {
                return Err(io::Error::other(format!("{} folder path is not absolute: {}", kind.label(), p.display())));
            }
            if is_protected(p) {
                return Err(io::Error::other(format!(
                    "{} folder is in a protected system location, refusing to touch it: {}",
                    kind.label(),
                    p.display()
                )));
            }
        }
        Ok(())
    }
}

/// `%APPDATA%\file-organizer` (settings and journal live here).
pub fn app_data_dir() -> io::Result<PathBuf> {
    Ok(sys::known_folder(sys::Folder::RoamingAppData)?.join("file-organizer"))
}

/// True for C:\Windows, Program Files, ProgramData, AppData and anything
/// inside them, for drive roots, and for any folder that CONTAINS one of
/// those (so a root like `C:\` is refused too).
pub fn is_protected(path: &Path) -> bool {
    let p = norm(path);
    if p.is_empty() || path.parent().is_none() {
        return true;
    }
    if path
        .components()
        .any(|c| matches!(c, Component::Normal(n) if n.to_string_lossy().eq_ignore_ascii_case("appdata")))
    {
        return true;
    }
    protected_dirs().iter().any(|d| {
        let d = norm(d);
        !d.is_empty() && (is_within(&p, &d) || is_within(&d, &p))
    })
}

fn protected_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    for var in [
        "SystemRoot",
        "windir",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "ProgramData",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        if let Some(val) = std::env::var_os(var) {
            if !val.is_empty() {
                v.push(PathBuf::from(val));
            }
        }
    }
    if cfg!(windows) {
        for p in [r"C:\Windows", r"C:\Program Files", r"C:\Program Files (x86)", r"C:\ProgramData"] {
            v.push(PathBuf::from(p));
        }
    }
    v
}

/// Lower-cased path with `\` separators and no trailing separator.
fn norm(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    s.trim_end_matches('\\').to_string()
}

fn is_within(child: &str, parent: &str) -> bool {
    child == parent || (child.starts_with(parent) && child[parent.len()..].starts_with('\\'))
}

#[cfg(windows)]
mod sys {
    use std::ffi::OsString;
    use std::io;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use windows_sys::core::{GUID, PWSTR};
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        SHGetKnownFolderPath, FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_RoamingAppData,
        KF_FLAG_DEFAULT,
    };

    pub enum Folder {
        Downloads,
        Desktop,
        Documents,
        RoamingAppData,
    }

    pub fn known_folder(f: Folder) -> io::Result<PathBuf> {
        let id: GUID = match f {
            Folder::Downloads => FOLDERID_Downloads,
            Folder::Desktop => FOLDERID_Desktop,
            Folder::Documents => FOLDERID_Documents,
            Folder::RoamingAppData => FOLDERID_RoamingAppData,
        };
        let mut out: PWSTR = std::ptr::null_mut();
        // SAFETY: documented API; `out` is freed with CoTaskMemFree in all cases.
        let hr = unsafe { SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT as u32, std::ptr::null_mut(), &mut out) };
        let result = if hr >= 0 && !out.is_null() {
            let mut len = 0usize;
            // SAFETY: on success `out` is a valid NUL-terminated wide string.
            unsafe {
                while *out.add(len) != 0 {
                    len += 1;
                }
                Ok(PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(out, len))))
            }
        } else {
            Err(io::Error::other(format!("Windows could not find a known folder (error 0x{hr:08X})")))
        };
        // SAFETY: freeing null is allowed.
        unsafe { CoTaskMemFree(out as *const _) };
        result
    }
}

#[cfg(not(windows))]
mod sys {
    //! Development fallback for non-Windows builds.
    use std::io;
    use std::path::PathBuf;

    pub enum Folder {
        Downloads,
        Desktop,
        Documents,
        RoamingAppData,
    }

    pub fn known_folder(f: Folder) -> io::Result<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| io::Error::other("HOME not set"))?;
        Ok(match f {
            Folder::Downloads => home.join("Downloads"),
            Folder::Desktop => home.join("Desktop"),
            Folder::Documents => home.join("Documents"),
            Folder::RoamingAppData => home.join(".config"),
        })
    }
}
