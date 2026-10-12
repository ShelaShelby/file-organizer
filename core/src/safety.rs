//! Decides which files may be touched. When in doubt, a file is skipped.

use std::fmt;
use std::fs::{self, Metadata};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Endings of files that are still downloading or are temporary.
pub const TEMP_EXTENSIONS: &[&str] = &["crdownload", "part", "partial", "tmp", "download", "opdownload"];
/// Name prefixes of Office / LibreOffice lock and temp files.
pub const TEMP_PREFIXES: &[&str] = &["~$", ".~lock."];
/// Shortcuts are never moved.
pub const SHORTCUT_EXTENSIONS: &[&str] = &["lnk", "url"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    Link,
    Hidden,
    System,
    CloudOnly,
    DesktopIni,
    Shortcut,
    Temporary,
    Recent,
    NotAFile,
    InUse(String),
    Changed,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::Link => write!(f, "link / junction"),
            SkipReason::Hidden => write!(f, "hidden file"),
            SkipReason::System => write!(f, "system file"),
            SkipReason::CloudOnly => write!(f, "online-only file (not on this PC)"),
            SkipReason::DesktopIni => write!(f, "desktop.ini"),
            SkipReason::Shortcut => write!(f, "shortcut"),
            SkipReason::Temporary => write!(f, "temporary or still downloading"),
            SkipReason::Recent => write!(f, "changed recently"),
            SkipReason::NotAFile => write!(f, "not a normal file"),
            SkipReason::InUse(e) => write!(f, "in use or cannot be opened ({e})"),
            SkipReason::Changed => write!(f, "changed since the scan"),
        }
    }
}

/// Result of checking one entry at the top level of a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// A folder (folders and everything inside them are never touched).
    Folder,
    Skip(SkipReason),
    Ok { size: u64, modified: SystemTime },
}

/// Full check of one top-level entry. `min_age`: files changed more
/// recently than this are skipped.
pub fn check_entry(path: &Path, min_age: Duration, now: SystemTime) -> Check {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => return Check::Skip(SkipReason::InUse(e.to_string())),
    };
    if meta.file_type().is_symlink() {
        return Check::Skip(SkipReason::Link);
    }
    if meta.is_dir() {
        return Check::Folder;
    }
    if !meta.is_file() {
        return Check::Skip(SkipReason::NotAFile);
    }
    let name = match path.file_name() {
        Some(n) => n.to_string_lossy().to_string(),
        None => return Check::Skip(SkipReason::NotAFile),
    };
    if let Some(r) = name_or_attr_reason(&name, &meta) {
        return Check::Skip(r);
    }
    let modified = match meta.modified() {
        Ok(m) => m,
        Err(e) => return Check::Skip(SkipReason::InUse(e.to_string())),
    };
    if is_recent(modified, min_age, now) {
        return Check::Skip(SkipReason::Recent);
    }
    if let Err(e) = open_exclusive(path) {
        return Check::Skip(SkipReason::InUse(e.to_string()));
    }
    Check::Ok { size: meta.len(), modified }
}

fn is_recent(modified: SystemTime, min_age: Duration, now: SystemTime) -> bool {
    match now.duration_since(modified) {
        Ok(age) => age < min_age,
        Err(_) => true, // date in the future: treat as recent, be safe
    }
}

fn name_or_attr_reason(name: &str, meta: &Metadata) -> Option<SkipReason> {
    let lower = name.to_lowercase();
    if lower == "desktop.ini" {
        return Some(SkipReason::DesktopIni);
    }
    if TEMP_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return Some(SkipReason::Temporary);
    }
    let ext = Path::new(&lower).extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    if SHORTCUT_EXTENSIONS.contains(&ext.as_str()) {
        return Some(SkipReason::Shortcut);
    }
    if TEMP_EXTENSIONS.contains(&ext.as_str()) {
        return Some(SkipReason::Temporary);
    }
    attr_reason(name, meta)
}

#[cfg(windows)]
fn attr_reason(_name: &str, meta: &Metadata) -> Option<SkipReason> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_SYSTEM,
    };
    let a = meta.file_attributes();
    if a & FILE_ATTRIBUTE_SYSTEM != 0 {
        Some(SkipReason::System)
    } else if a & FILE_ATTRIBUTE_HIDDEN != 0 {
        Some(SkipReason::Hidden)
    } else if a & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_RECALL_ON_OPEN) != 0 {
        Some(SkipReason::CloudOnly)
    } else {
        None
    }
}

#[cfg(not(windows))]
fn attr_reason(name: &str, _meta: &Metadata) -> Option<SkipReason> {
    name.starts_with('.').then_some(SkipReason::Hidden)
}

/// Opens the file allowing NO sharing: fails if any other program has it
/// open (still downloading, being edited, ...), or if it cannot be read.
#[cfg(windows)]
pub fn open_exclusive(path: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new().read(true).share_mode(0).open(path).map(|_| ())
}

#[cfg(not(windows))]
pub fn open_exclusive(path: &Path) -> std::io::Result<()> {
    fs::File::open(path).map(|_| ())
}
