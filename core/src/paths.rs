//! The chosen folders ("roots"): Downloads, Desktop and Documents (found
//! with the Windows Known Folder API) plus up to [`MAX_EXTRA`] extra folders
//! from the settings; the app-data folder and the list of places that must
//! never be touched.

use std::io;
use std::path::{Component, Path, PathBuf};

/// Desktop files are sent to `Documents\<this>\<Type>\<YYYY-MM>\`.
pub const FROM_DESKTOP: &str = "From Desktop";
/// Most extra folders a user can add.
pub const MAX_EXTRA: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RootKind {
    Downloads,
    Desktop,
    Documents,
    /// The n-th extra folder from the settings (0-based).
    Extra(u8),
}

impl RootKind {
    /// Short name for the three standard folders; extra folders are named
    /// by [`Roots::label`].
    pub fn label(self) -> &'static str {
        match self {
            RootKind::Downloads => "Downloads",
            RootKind::Desktop => "Desktop",
            RootKind::Documents => "Documents",
            RootKind::Extra(_) => "Extra folder",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    pub downloads: PathBuf,
    pub desktop: PathBuf,
    pub documents: PathBuf,
    /// Extra folders (checked with [`check_extra`] before they get here,
    /// and again by [`Roots::validate`] before every batch).
    pub extra: Vec<PathBuf>,
}

impl Roots {
    pub fn new(downloads: PathBuf, desktop: PathBuf, documents: PathBuf) -> Roots {
        Roots { downloads, desktop, documents, extra: Vec::new() }
    }

    /// The current user's Downloads, Desktop and Documents, asked from Windows
    /// (works when they are redirected, e.g. to OneDrive).
    pub fn from_system() -> io::Result<Roots> {
        Ok(Roots::new(
            sys::known_folder(sys::Folder::Downloads)?,
            sys::known_folder(sys::Folder::Desktop)?,
            sys::known_folder(sys::Folder::Documents)?,
        ))
    }

    /// Adds the extra folders that pass [`check_extra`]; the others are
    /// dropped and one plain sentence per dropped folder is returned.
    pub fn with_extra(mut self, folders: &[PathBuf]) -> (Roots, Vec<String>) {
        let mut warnings = Vec::new();
        self.extra.clear();
        for f in folders {
            match check_extra(&self, f) {
                Ok(()) => self.extra.push(f.clone()),
                Err(e) => warnings.push(format!("Extra folder left out: {e}")),
            }
        }
        (self, warnings)
    }

    /// Every chosen folder: the three standard ones, then the extra ones.
    pub fn kinds(&self) -> impl Iterator<Item = RootKind> + '_ {
        [RootKind::Downloads, RootKind::Desktop, RootKind::Documents]
            .into_iter()
            .chain((0..self.extra.len().min(MAX_EXTRA)).map(|i| RootKind::Extra(i as u8)))
    }

    /// The folder of `kind` (an empty path for an extra folder that is not
    /// in the list, which matches no file).
    pub fn get(&self, kind: RootKind) -> &Path {
        match kind {
            RootKind::Downloads => &self.downloads,
            RootKind::Desktop => &self.desktop,
            RootKind::Documents => &self.documents,
            RootKind::Extra(i) => self.extra.get(i as usize).map(PathBuf::as_path).unwrap_or(Path::new("")),
        }
    }

    /// Name shown to the user: "Downloads", ... or the extra folder's name.
    pub fn label(&self, kind: RootKind) -> String {
        match kind {
            RootKind::Extra(_) => folder_label(self.get(kind)),
            k => k.label().to_string(),
        }
    }

    /// Where the type folders for files from `kind` are created.
    pub fn target_base(&self, kind: RootKind) -> PathBuf {
        match kind {
            RootKind::Desktop => self.documents.join(FROM_DESKTOP),
            k => self.get(k).to_path_buf(),
        }
    }

    /// Refuse to work if any root is (inside) a protected system place, or
    /// if an extra folder no longer passes [`check_extra`]. Each root is
    /// also resolved to its final path (links and junctions followed) and
    /// that path is checked too, so a junction cannot hide a protected place.
    pub fn validate(&self) -> io::Result<()> {
        if self.extra.len() > MAX_EXTRA {
            return Err(io::Error::other(format!("More than {MAX_EXTRA} extra folders.")));
        }
        for kind in self.kinds() {
            let p = self.get(kind);
            let label = self.label(kind);
            if !p.is_absolute() {
                return Err(io::Error::other(format!("{label} folder path is not absolute: {}", p.display())));
            }
            let resolved = match final_path(p) {
                Ok(r) => Some(r),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => {
                    return Err(io::Error::other(format!(
                        "{label} folder cannot be opened ({e}), refusing to touch it: {}",
                        p.display()
                    )))
                }
            };
            if is_protected(p) || resolved.as_deref().is_some_and(is_protected) {
                return Err(io::Error::other(format!(
                    "{label} folder is in a protected system location, refusing to touch it: {}",
                    p.display()
                )));
            }
        }
        for (i, f) in self.extra.iter().enumerate() {
            let mut others = self.clone();
            others.extra.truncate(i);
            check_extra(&others, f).map_err(io::Error::other)?;
            // Later extras are compared with this one when their turn comes.
        }
        Ok(())
    }
}

/// "C:\Users\me\Projects" -> "Projects".
pub fn folder_label(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

/// Why `folder` cannot be an extra folder next to `roots` (one plain
/// sentence), or `Ok`. Checked when adding, when loading settings.json and
/// before every batch.
pub fn check_extra(roots: &Roots, folder: &Path) -> Result<(), String> {
    let shown = folder.display();
    if roots.extra.len() >= MAX_EXTRA {
        return Err(format!("{shown}: you can add at most {MAX_EXTRA} extra folders."));
    }
    let text = folder.to_string_lossy();
    if text.starts_with(r"\\") || text.starts_with("//") {
        return Err(format!("{shown} is a network, device or \\\\?\\ path; only folders on this PC can be added."));
    }
    if !folder.is_absolute() || crate::fsops::has_dot_parts(folder) {
        return Err(format!("{shown} is not a full folder path (like C:\\Users\\me\\Projects)."));
    }
    if is_protected(folder) {
        return Err(format!(
            "{shown} is a protected place (Windows, Program Files, AppData, your user folder itself or a whole drive)."
        ));
    }
    if sys::is_network_drive(folder) {
        return Err(format!("{shown} is on a network drive; only folders on this PC can be added."));
    }
    let meta = match std::fs::symlink_metadata(folder) {
        Ok(m) => m,
        Err(_) => return Err(format!("{shown} does not exist.")),
    };
    if meta.file_type().is_symlink() {
        return Err(format!("{shown} is a link or junction to another place; add the real folder instead."));
    }
    if !meta.is_dir() {
        return Err(format!("{shown} is not a folder."));
    }
    if crate::safety::is_hidden_or_system(folder, &meta) {
        return Err(format!("{shown} is a hidden or system folder."));
    }
    let resolved = final_path(folder).map_err(|e| format!("{shown} cannot be opened ({e})."))?;
    if is_protected(&resolved) {
        return Err(format!(
            "{shown} is a protected place (Windows, Program Files, AppData, your user folder itself or a whole drive)."
        ));
    }
    let key = norm(&resolved);
    if let Ok(app) = app_data_dir() {
        let a = norm(&final_path(&app).unwrap_or(app));
        if is_within(&key, &a) || is_within(&a, &key) {
            return Err(format!("{shown} holds File Organizer's own settings and journal."));
        }
    }
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| final_path(&e).ok()) {
        if exe.parent().is_some_and(|d| norm(d) == key) {
            return Err(format!("{shown} is the folder File Organizer itself runs from."));
        }
    }
    for kind in roots.kinds() {
        let other = roots.get(kind);
        let o = norm(&final_path(other).unwrap_or_else(|_| other.to_path_buf()));
        if o.is_empty() {
            continue;
        }
        if o == key {
            return Err(format!("{shown} is already in the list ({}).", roots.label(kind)));
        }
        if is_within(&key, &o) {
            return Err(format!("{shown} is inside {}, which is already in the list.", roots.label(kind)));
        }
        if is_within(&o, &key) {
            return Err(format!("{shown} contains {}, which is already in the list.", roots.label(kind)));
        }
    }
    Ok(())
}

/// `%APPDATA%\file-organizer` (settings and journal live here).
pub fn app_data_dir() -> io::Result<PathBuf> {
    Ok(sys::known_folder(sys::Folder::RoamingAppData)?.join("file-organizer"))
}

/// The user profile folder (e.g. `C:\Users\me`).
pub fn profile_dir() -> io::Result<PathBuf> {
    sys::known_folder(sys::Folder::Profile)
}

/// The Windows fonts folder (Known Folder API).
pub fn fonts_dir() -> io::Result<PathBuf> {
    sys::known_folder(sys::Folder::Fonts)
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
            for p in [r"C:\Windows", r"C:\Program Files", r"C:\Program Files (x86)", r"C:\ProgramData"] {
                dirs.push(PathBuf::from(p));
            }
        }
        let profile = sys::known_folder(Profile).ok().filter(|p| !p.as_os_str().is_empty());
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
        Some(SelfExe { name, len, key: crate::fsops::path_key(&exe) })
    })
    .as_ref()
}

/// True if `path` is the running program itself (compared on final paths;
/// name and size are checked first so this stays cheap).
pub fn is_self_exe(path: &Path, len: u64) -> bool {
    let Some(me) = self_exe() else {
        return false;
    };
    let same_name = path.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == me.name);
    same_name && len == me.len && final_path(path).is_ok_and(|p| crate::fsops::path_key(&p) == me.key)
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
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Fonts, FOLDERID_LocalAppData,
        FOLDERID_Profile, FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesCommon,
        FOLDERID_ProgramFilesX86, FOLDERID_RoamingAppData, FOLDERID_System, FOLDERID_Windows, SHGetKnownFolderPath,
        KF_FLAG_DEFAULT,
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
        Fonts,
    }

    /// True if the drive of `p` is a mapped network drive.
    pub fn is_network_drive(p: &std::path::Path) -> bool {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        const DRIVE_REMOTE: u32 = 4;
        let Some(std::path::Component::Prefix(prefix)) = p.components().next() else {
            return false;
        };
        let mut root: Vec<u16> = prefix.as_os_str().encode_wide().collect();
        root.extend("\\\0".encode_utf16());
        // SAFETY: NUL-terminated root path such as "Z:\".
        unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
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
            Folder::Fonts => FOLDERID_Fonts,
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
        LocalAppData,
        Windows,
        System,
        ProgramFiles,
        ProgramFilesX86,
        ProgramFilesCommon,
        ProgramData,
        Profile,
        Fonts,
    }

    pub fn is_network_drive(_p: &std::path::Path) -> bool {
        false
    }

    pub fn known_folder(f: Folder) -> io::Result<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| io::Error::other("HOME not set"))?;
        match f {
            Folder::Downloads => Ok(home.join("Downloads")),
            Folder::Desktop => Ok(home.join("Desktop")),
            Folder::Documents => Ok(home.join("Documents")),
            Folder::RoamingAppData => Ok(home.join(".config")),
            Folder::LocalAppData => Ok(home.join(".local")),
            Folder::Profile => Ok(home),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "not on this system")),
        }
    }
}
