//! Settings (stored in `%APPDATA%\file-organizer\settings.json`) and the
//! extension -> type-folder map.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

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
}

/// THE extension map. Edit here to change the built-in defaults
/// (users can also edit settings.json).
const DEFAULT_CATEGORIES: &[(&str, &[&str])] = &[
    ("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp", "heic", "tiff", "svg"]),
    ("Videos", &["mp4", "mkv", "mov", "avi", "webm", "wmv"]),
    ("Music", &["mp3", "wav", "flac", "aac", "ogg", "m4a"]),
    (
        "Documents",
        &["pdf", "doc", "docx", "txt", "rtf", "odt", "xls", "xlsx", "csv", "ppt", "pptx", "md"],
    ),
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
    /// A broken file is left untouched and defaults are used; the returned
    /// warning explains that.
    pub fn load_or_create(path: &Path) -> (Settings, Option<String>) {
        match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(s) => (s, None),
                Err(e) => (
                    Settings::default(),
                    Some(format!("settings.json could not be read ({e}); using defaults")),
                ),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let s = Settings::default();
                let warn = s.save(path).err().map(|e| format!("could not save settings.json: {e}"));
                (s, warn)
            }
            Err(e) => (Settings::default(), Some(format!("settings.json could not be read ({e}); using defaults"))),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, text)
    }
}

fn normalize_ext(e: &str) -> String {
    e.trim().trim_start_matches('.').to_lowercase()
}

/// Category names come from a user-editable file; never let them escape
/// the target folder or contain characters Windows rejects.
fn safe_folder_name(name: &str) -> String {
    let n = name.trim();
    let bad = n.is_empty()
        || n == "."
        || n == ".."
        || n.ends_with('.')
        || n.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control());
    if bad {
        OTHER.to_string()
    } else {
        n.to_string()
    }
}
