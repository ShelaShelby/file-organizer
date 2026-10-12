//! Settings (stored in `%APPDATA%\file-organizer\settings.json`) and the
//! extension -> type-folder map.

use crate::paths::MAX_EXTRA;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Name of the folder used for files whose extension is not in the map.
pub const OTHER: &str = "Other";

/// One type folder and the file extensions that belong in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Desktop and Documents: only files NOT changed in this many days are sorted.
    pub recent_days: u32,
    /// Type folders, checked in order. Anything not listed goes to "Other".
    pub categories: Vec<Category>,
    /// "Big and old" tab: files of at least this many MB are big.
    pub big_file_mb: u64,
    /// "Big and old" tab: files not changed for this many days are old.
    pub old_days: u32,
    /// Duplicates: files smaller than this many KB are ignored.
    pub min_duplicate_kb: u64,
    /// Background sorting of Downloads (`--tray`) is paused.
    pub background_paused: bool,
    /// "Old installers" list: installers not changed for this many days.
    pub installer_days: u32,
    /// Extra folders handled like Documents (at most 10; each one is
    /// checked with `paths::check_extra` before it is used).
    pub extra_folders: Vec<PathBuf>,
}

/// Allowed ranges; values outside fall back to the default.
pub const RECENT_DAYS_MAX: u32 = 3650;
pub const BIG_FILE_MB_RANGE: (u64, u64) = (1, 1_048_576);
pub const OLD_DAYS_RANGE: (u32, u32) = (1, 36_500);
pub const MIN_DUPLICATE_KB_MAX: u64 = 1_048_576;
pub const INSTALLER_DAYS_RANGE: (u32, u32) = (1, 3650);

/// THE extension map. Edit here to change the built-in defaults
/// (users can also edit settings.json).
const DEFAULT_CATEGORIES: &[(&str, &[&str])] = &[
    ("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp", "heic", "tiff", "svg"]),
    ("Videos", &["mp4", "mkv", "mov", "avi", "webm", "wmv"]),
    ("Music", &["mp3", "wav", "flac", "aac", "ogg", "m4a"]),
    ("Documents", &["pdf", "doc", "docx", "txt", "rtf", "odt", "xls", "xlsx", "csv", "ppt", "pptx", "md"]),
    ("Archives", &["zip", "rar", "7z", "tar", "gz"]),
    ("Installers", &["exe", "msi", "msix"]),
];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            recent_days: 7,
            categories: DEFAULT_CATEGORIES
                .iter()
                .map(|(name, exts)| Category {
                    name: name.to_string(),
                    extensions: exts.iter().map(|e| e.to_string()).collect(),
                })
                .collect(),
            big_file_mb: 500,
            old_days: 365,
            min_duplicate_kb: 1,
            background_paused: false,
            installer_days: 30,
            extra_folders: Vec::new(),
        }
    }
}

impl Settings {
    /// Type folder for a file name, e.g. "photo.JPG" -> "Images".
    pub fn category_for(&self, file_name: &str) -> String {
        let ext = match Path::new(file_name).extension() {
            Some(e) => e.to_string_lossy().to_lowercase(),
            None => return OTHER.to_string(),
        };
        for c in &self.categories {
            if c.extensions.iter().any(|e| normalize_ext(e) == ext) {
                return safe_folder_name(&c.name);
            }
        }
        OTHER.to_string()
    }

    /// Load settings. A missing file is created with defaults.
    /// A file that cannot be read is left untouched and defaults are used.
    /// Single broken values fall back to their default. The returned
    /// warning explains what happened.
    pub fn load_or_create(path: &Path) -> (Settings, Option<String>) {
        match fs::read_to_string(path) {
            Ok(text) => Settings::from_json(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // Another process may create it at the same moment: update()
                // keeps whatever is there by then.
                match Settings::update(path, |_| {}) {
                    Ok(s) => (s, None),
                    Err(e) => (Settings::default(), Some(format!("could not save settings.json: {e}"))),
                }
            }
            Err(e) => (Settings::default(), Some(format!("settings.json could not be read ({e}); using defaults"))),
        }
    }

    /// Settings from JSON text, value by value: anything missing, of the
    /// wrong type or out of range is replaced by its default.
    pub fn from_json(text: &str) -> (Settings, Option<String>) {
        let mut s = Settings::default();
        let v: serde_json::Value = match serde_json::from_str(text) {
            Ok(v @ serde_json::Value::Object(_)) => v,
            Ok(_) => return (s, Some("settings.json could not be read (not an object); using defaults".into())),
            Err(e) => return (s, Some(format!("settings.json could not be read ({e}); using defaults"))),
        };
        let mut bad = Vec::new();
        fn field<T: serde::de::DeserializeOwned>(
            v: &serde_json::Value,
            name: &str,
            ok: impl Fn(&T) -> bool,
            bad: &mut Vec<String>,
        ) -> Option<T> {
            let raw = v.get(name)?;
            match serde_json::from_value::<T>(raw.clone()) {
                Ok(x) if ok(&x) => Some(x),
                _ => {
                    bad.push(name.to_string());
                    None
                }
            }
        }
        if let Some(x) = field(&v, "recent_days", |d: &u32| *d <= RECENT_DAYS_MAX, &mut bad) {
            s.recent_days = x;
        }
        if let Some(x) = field(&v, "categories", |_: &Vec<Category>| true, &mut bad) {
            s.categories = x;
        }
        if let Some(x) =
            field(&v, "big_file_mb", |m: &u64| (BIG_FILE_MB_RANGE.0..=BIG_FILE_MB_RANGE.1).contains(m), &mut bad)
        {
            s.big_file_mb = x;
        }
        if let Some(x) = field(&v, "old_days", |d: &u32| (OLD_DAYS_RANGE.0..=OLD_DAYS_RANGE.1).contains(d), &mut bad) {
            s.old_days = x;
        }
        if let Some(x) = field(&v, "min_duplicate_kb", |k: &u64| *k <= MIN_DUPLICATE_KB_MAX, &mut bad) {
            s.min_duplicate_kb = x;
        }
        if let Some(x) = field(&v, "background_paused", |_: &bool| true, &mut bad) {
            s.background_paused = x;
        }
        if let Some(x) = field(
            &v,
            "installer_days",
            |d: &u32| (INSTALLER_DAYS_RANGE.0..=INSTALLER_DAYS_RANGE.1).contains(d),
            &mut bad,
        ) {
            s.installer_days = x;
        }
        if let Some(mut x) = field(&v, "extra_folders", |_: &Vec<PathBuf>| true, &mut bad) {
            if x.len() > MAX_EXTRA {
                x.truncate(MAX_EXTRA);
                bad.push(format!("extra_folders (only the first {MAX_EXTRA} are used)"));
            }
            s.extra_folders = x;
        }
        let warn =
            (!bad.is_empty()).then(|| format!("settings.json: bad value for {}; using the default", bad.join(", ")));
        (s, warn)
    }

    /// The same settings with every out-of-range value replaced by its default.
    pub fn sanitized(&self) -> Settings {
        let text = serde_json::to_string(self).unwrap_or_default();
        Settings::from_json(&text).0
    }

    /// Reads settings.json as it is on disk NOW, lets `change` edit it and
    /// saves it, all while holding `settings.lock`. Used when two processes
    /// (window and background sorter) may both write settings, so neither
    /// loses the other's change.
    pub fn update(path: &Path, change: impl FnOnce(&mut Settings)) -> io::Result<Settings> {
        let _lock = lock(path)?;
        let mut s = match fs::read_to_string(path) {
            Ok(text) => Settings::from_json(&text).0,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Settings::default(),
            Err(e) => return Err(io::Error::other(format!("settings.json cannot be read ({e}); not overwriting it"))),
        };
        change(&mut s);
        s.save_locked(path)?;
        Ok(s)
    }

    /// Writes settings.json (holding `settings.lock`). Refuses to replace a
    /// file that exists but cannot be read as settings (the user's file is
    /// never lost).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let _lock = lock(path)?;
        self.save_locked(path)
    }

    /// Writes `settings.json.tmp` in the same folder, flushes it, then puts
    /// it in place in one step, so a crash never leaves a half-written file.
    fn save_locked(&self, path: &Path) -> io::Result<()> {
        match fs::read_to_string(path) {
            Ok(text) => {
                if !matches!(serde_json::from_str::<serde_json::Value>(&text), Ok(serde_json::Value::Object(_))) {
                    return Err(io::Error::other("settings.json exists but cannot be read; not overwriting it"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io::Error::other(format!("settings.json cannot be read ({e}); not overwriting it"))),
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&self.sanitized()).map_err(io::Error::other)?;
        let tmp = side_file(path, "json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        replace_own_file(&tmp, path)
    }
}

/// `settings.json` -> `settings.<ext>` in the same folder.
fn side_file(path: &Path, ext: &str) -> PathBuf {
    path.with_extension(ext)
}

/// Held while settings.json is read, changed and written.
struct SettingsLock {
    _file: fs::File,
}

fn lock(path: &Path) -> io::Result<SettingsLock> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file =
        fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(side_file(path, "lock"))?;
    file.lock()?;
    Ok(SettingsLock { _file: file })
}

/// Replaces the app's OWN settings file with the freshly written temp file.
/// This is the only place in the app where a file is replaced.
#[cfg(windows)]
fn replace_own_file(tmp: &Path, dst: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }
    let (s, d) = (wide(tmp), wide(dst));
    // SAFETY: both strings are NUL-terminated and live for the call.
    let ok = unsafe { MoveFileExW(s.as_ptr(), d.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_own_file(tmp: &Path, dst: &Path) -> io::Result<()> {
    fs::rename(tmp, dst)
}

fn normalize_ext(e: &str) -> String {
    e.trim().trim_start_matches('.').to_lowercase()
}

/// Category names come from a user-editable file; never let them escape
/// the target folder, contain characters Windows rejects or be a device
/// name (CON, NUL, COM1, ...). Anything else becomes "Other".
pub fn safe_folder_name(name: &str) -> String {
    let n = name.trim();
    if n != "." && n != ".." && crate::rename::is_valid_windows_name(n) {
        n.to_string()
    } else {
        OTHER.to_string()
    }
}
