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
    /// Each root is also resolved to its final path (links and junctions
    /// followed) and that path is checked too, so a junction cannot hide a
    /// protected place.
    pub fn validate(&self) -> io::Result<()> {
        for kind in RootKind::ALL {
            let p = self.get(kind);
            if !p.is_absolute() {
                return Err(io::Error::other(format!(
                    "{} folder path is not absolute: {}",
                    kind.label(),
                    p.display()
                )));
            }
            let resolved = match final_path(p) {
                Ok(r) => Some(r),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => {
                    return Err(io::Error::other(format!(
                        "{} folder cannot be opened ({e}), refusing to touch it: {}",
                        kind.label(),
                        p.display()
                    )))
                }
            };
            if is_protected(p) || resolved.as_deref().is_some_and(is_protected) {
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

/// The user profile folder (e.g. `C:\Users\me`).
pub fn profile_dir() -> io::Result<PathBuf> {
    sys::known_folder(sys::Folder::Profile)
}

/// The Windows folder (e.g. `C:\Windows`), asked from Windows itself rather
/// than from an environment variable.
pub fn windows_dir() -> io::Result<PathBuf> {
    sys::known_folder(sys::Folder::Windows)
}

/// The final path of an existing file or folder: links and junctions are
/// followed and the `\\?\` prefix is removed (`\\?\UNC\` becomes `\\`).
pub fn final_path(p: &Path) -> io::Result<PathBuf> {
    Ok(strip_verbatim(&std::fs::canonicalize(p)?))
}

fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        p.to_path_buf()
    }
}

/// True for C:\Windows, Program Files, ProgramData, AppData and anything
/// inside them, for drive roots, for the user profile folder itself, and for
/// any folder that CONTAINS one of those (so a root like `C:\` is refused
/// too). The places are asked from Windows (Known Folder API), never read
/// from environment variables, which a parent process could change.
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
    let places = protected_places();
    let inside_or_containing = places.dirs.iter().any(|d| {
        let d = norm(d);
        !d.is_empty() && (is_within(&p, &d) || is_within(&d, &p))
    });
    // The profile itself (or a folder containing it) is refused; Downloads
    // and the like inside it are fine.
    let profile = places.profile.as_ref().is_some_and(|d| {
        let d = norm(d);
        !d.is_empty() && is_within(&d, &p)
    });
    inside_or_containing || profile
}

struct Protected {
    dirs: Vec<PathBuf>,
    profile: Option<PathBuf>,
}

fn protected_places() -> &'static Protected {
    static PLACES: std::sync::OnceLock<Protected> = std::sync::OnceLock::new();
    PLACES.get_or_init(|| {
        use sys::Folder::*;
        let mut dirs: Vec<PathBuf> = Vec::new();
        for f in [
            Windows,
            System,
            ProgramFiles,
            ProgramFilesX86,
            ProgramFilesCommon,
            ProgramData,
            RoamingAppData,
            LocalAppData,
        ] {
            if let Ok(d) = sys::known_folder(f) {
                if !d.as_os_str().is_empty() {
                    dirs.push(d);
                }
            }
        }
        if cfg!(windows) {
            for p in [
                r"C:\Windows",
                r"C:\Program Files",
                r"C:\Program Files (x86)",
                r"C:\ProgramData",
            ] {
                dirs.push(PathBuf::from(p));
            }
        }
        let profile = sys::known_folder(Profile)
            .ok()
            .filter(|p| !p.as_os_str().is_empty());
        Protected { dirs, profile }
    })
}

/// The running program, so that no scan or batch ever touches it.
struct SelfExe {
    name: String,
    len: u64,
    key: String,
}

fn self_exe() -> Option<&'static SelfExe> {
    static EXE: std::sync::OnceLock<Option<SelfExe>> = std::sync::OnceLock::new();
    EXE.get_or_init(|| {
        let exe = final_path(&std::env::current_exe().ok()?).ok()?;
        let name = exe.file_name()?.to_string_lossy().to_lowercase();
        let len = std::fs::metadata(&exe).ok()?.len();
        Some(SelfExe {
            name,
            len,
            key: crate::fsops::path_key(&exe),
        })
    })
    .as_ref()
}

/// True if `path` is the running program itself (compared on final paths;
/// name and size are checked first so this stays cheap).
pub fn is_self_exe(path: &Path, len: u64) -> bool {
    let Some(me) = self_exe() else {
        return false;
    };
    let same_name = path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().to_lowercase() == me.name);
    same_name
        && len == me.len
        && final_path(path).is_ok_and(|p| crate::fsops::path_key(&p) == me.key)
}

/// Lower-cased path with `\` separators and no trailing separator.
pub(crate) fn norm(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    s.trim_end_matches('\\').to_string()
}

pub(crate) fn is_within(child: &str, parent: &str) -> bool {
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
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_LocalAppData,
        FOLDERID_Profile, FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesCommon,
        FOLDERID_ProgramFilesX86, FOLDERID_RoamingAppData, FOLDERID_System, FOLDERID_Windows,
        SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };

    pub enum Folder {
        Downloads,
        Desktop,
        Documents,
        RoamingAppData,
        LocalAppData,
        Windows,
        System,
        ProgramFiles,
        ProgramFilesX86,
        ProgramFilesCommon,
        ProgramData,
        Profile,
    }

    pub fn known_folder(f: Folder) -> io::Result<PathBuf> {
        let id: GUID = match f {
            Folder::Downloads => FOLDERID_Downloads,
            Folder::Desktop => FOLDERID_Desktop,
            Folder::Documents => FOLDERID_Documents,
            Folder::RoamingAppData => FOLDERID_RoamingAppData,
            Folder::LocalAppData => FOLDERID_LocalAppData,
            Folder::Windows => FOLDERID_Windows,
            Folder::System => FOLDERID_System,
            Folder::ProgramFiles => FOLDERID_ProgramFiles,
            Folder::ProgramFilesX86 => FOLDERID_ProgramFilesX86,
            Folder::ProgramFilesCommon => FOLDERID_ProgramFilesCommon,
            Folder::ProgramData => FOLDERID_ProgramData,
            Folder::Profile => FOLDERID_Profile,
        };
        let mut out: PWSTR = std::ptr::null_mut();
        // SAFETY: documented API; `out` is freed with CoTaskMemFree in all cases.
        let hr = unsafe {
            SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT as u32, std::ptr::null_mut(), &mut out)
        };
        let result = if hr >= 0 && !out.is_null() {
            let mut len = 0usize;
            // SAFETY: on success `out` is a valid NUL-terminated wide string.
            unsafe {
                while *out.add(len) != 0 {
                    len += 1;
                }
                Ok(PathBuf::from(OsString::from_wide(
                    std::slice::from_raw_parts(out, len),
                )))
            }
        } else {
            Err(io::Error::other(format!(
                "Windows could not find a known folder (error 0x{hr:08X})"
            )))
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
        LocalAppData,
        Windows,
        System,
        ProgramFiles,
        ProgramFilesX86,
        ProgramFilesCommon,
        ProgramData,
        Profile,
    }

    pub fn known_folder(f: Folder) -> io::Result<PathBuf> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("HOME not set"))?;
        match f {
            Folder::Downloads => Ok(home.join("Downloads")),
            Folder::Desktop => Ok(home.join("Desktop")),
            Folder::Documents => Ok(home.join("Documents")),
            Folder::RoamingAppData => Ok(home.join(".config")),
            Folder::LocalAppData => Ok(home.join(".local")),
            Folder::Profile => Ok(home),
            _ => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "not on this system",
            )),
        }
    }
}
