//! Decides which files may be touched. When in doubt, a file is skipped.

use std::fmt;
use std::fs::{self, Metadata};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Endings of files that are still downloading or are temporary.
pub const TEMP_EXTENSIONS: &[&str] = &[
    "crdownload",
    "part",
    "partial",
    "tmp",
    "download",
    "opdownload",
];
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
    /// The running File Organizer program.
    SelfApp,
    /// A program or project folder (see [`program_folder_reason`]).
    ProgramFolder,
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
            SkipReason::SelfApp => write!(f, "File Organizer itself"),
            SkipReason::ProgramFolder => write!(f, "program or project folder"),
        }
    }
}

/// Result of checking one entry at the top level of a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// A folder (folders and everything inside them are never touched).
    Folder,
    Skip(SkipReason),
    Ok {
        size: u64,
        modified: SystemTime,
    },
}

/// Full check of one top-level entry. `min_age`: files changed more
/// recently than this are skipped.
pub fn check_entry(path: &Path, min_age: Duration, now: SystemTime) -> Check {
    match check_entry_unlocked(path, min_age, now) {
        Check::Ok { size, modified } => match open_exclusive(path) {
            Ok(()) => Check::Ok { size, modified },
            Err(e) => Check::Skip(SkipReason::InUse(e.to_string())),
        },
        other => other,
    }
}

/// Same as [`check_entry`] but without trying to open the file (fast, used
/// while walking big folder trees; the open check is done later).
pub fn check_entry_unlocked(path: &Path, min_age: Duration, now: SystemTime) -> Check {
    match fs::symlink_metadata(path) {
        Ok(meta) => check_meta(path, &meta, min_age, now),
        Err(e) => Check::Skip(SkipReason::InUse(e.to_string())),
    }
}

/// Checks an entry from metadata already read (no lock check).
pub fn check_meta(path: &Path, meta: &Metadata, min_age: Duration, now: SystemTime) -> Check {
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
    if crate::paths::is_self_exe(path, meta.len()) {
        return Check::Skip(SkipReason::SelfApp);
    }
    if let Some(r) = name_or_attr_reason(&name, meta) {
        return Check::Skip(r);
    }
    let modified = match meta.modified() {
        Ok(m) => m,
        Err(e) => return Check::Skip(SkipReason::InUse(e.to_string())),
    };
    if is_recent(modified, min_age, now) {
        return Check::Skip(SkipReason::Recent);
    }
    Check::Ok {
        size: meta.len(),
        modified,
    }
}

/// Why a sub-folder must not be entered (links / junctions, hidden or
/// system folders, protected places), or `None` if it may be scanned.
pub fn folder_skip_reason(path: &Path, meta: &Metadata) -> Option<SkipReason> {
    if meta.file_type().is_symlink() {
        return Some(SkipReason::Link);
    }
    if !meta.is_dir() {
        return Some(SkipReason::NotAFile);
    }
    if crate::paths::is_protected(path) {
        return Some(SkipReason::System);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if is_program_folder_name(&name) {
        return Some(SkipReason::ProgramFolder);
    }
    match attr_reason(&name, meta) {
        Some(SkipReason::CloudOnly) | None => None, // online-only folders: files inside are checked one by one
        r => r,
    }
}

/// Folders that belong to programs, version control or developer tools
/// (never entered).
pub const PROGRAM_FOLDER_NAMES: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "site-packages",
    ".gradle",
    "steamapps",
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
];
/// A folder holding any of these is a project folder (never entered).
pub const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "requirements.txt",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "CMakeLists.txt",
];
/// A folder holding a file with one of these endings is a project folder.
pub const PROJECT_MARKER_EXTENSIONS: &[&str] = &["sln", "csproj"];

pub fn is_program_folder_name(name: &str) -> bool {
    PROGRAM_FOLDER_NAMES
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
}

/// `Some(ProgramFolder)` if a folder with these entry names is a project
/// folder (a marker file such as `package.json` or `*.sln`) or a program
/// folder (an `.exe` and a `.dll` directly inside).
pub fn program_folder_reason<S: AsRef<str>>(entry_names: &[S]) -> Option<SkipReason> {
    let (mut exe, mut dll) = (false, false);
    for n in entry_names {
        let n = n.as_ref();
        if PROJECT_MARKERS.iter().any(|m| m.eq_ignore_ascii_case(n)) {
            return Some(SkipReason::ProgramFolder);
        }
        let ext = Path::new(n)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if PROJECT_MARKER_EXTENSIONS.contains(&ext.as_str()) {
            return Some(SkipReason::ProgramFolder);
        }
        exe |= ext == "exe";
        dll |= ext == "dll";
    }
    (exe && dll).then_some(SkipReason::ProgramFolder)
}

/// [`program_folder_reason`] for a folder on disk (by name, then by what
/// it holds). A folder that cannot be listed counts as not safe.
pub fn program_folder_on_disk(dir: &Path) -> Option<SkipReason> {
    if dir
        .file_name()
        .is_some_and(|n| is_program_folder_name(&n.to_string_lossy()))
    {
        return Some(SkipReason::ProgramFolder);
    }
    let names: Vec<String> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect(),
        Err(e) => return Some(SkipReason::InUse(e.to_string())),
    };
    program_folder_reason(&names)
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
    let ext = Path::new(&lower)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
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
    } else if a
        & (FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
            | FILE_ATTRIBUTE_RECALL_ON_OPEN)
        != 0
    {
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
pub fn open_exclusive(path: &Path) -> std::io::Result<()> {
    open_exclusive_file(path).map(|_| ())
}

/// Opens the file for reading allowing NO sharing (see [`open_exclusive`]).
#[cfg(windows)]
pub fn open_exclusive_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new().read(true).share_mode(0).open(path)
}

#[cfg(not(windows))]
pub fn open_exclusive_file(path: &Path) -> std::io::Result<fs::File> {
    fs::File::open(path)
}
