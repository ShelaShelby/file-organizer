//! Old installers: setup files (`.exe`, `.msi`, ...) last changed more than
//! `installer_days` ago. Looked for ONLY at the top of Downloads and in the
//! `Downloads\<Installers>\<YYYY-MM>\` folders this app creates (portable
//! programs elsewhere are never listed). List only; removal goes through
//! [`crate::remove::recycle_files`] like the big / old lists.

use crate::config::{safe_folder_name, Settings};
use crate::fsops::path_key;
use crate::paths::{RootKind, Roots};
use crate::safety::{check_meta, folder_skip_reason, open_exclusive, program_folder_on_disk, Check, SkipReason};
use crate::tree::{Progress, ScannedFile, Skipped, GRACE};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

/// Name of the type folder for installers in the default settings.
pub const INSTALLERS: &str = "Installers";
/// Always counted as installers, on top of the "Installers" type folder's list.
pub const EXTRA_EXTENSIONS: &[&str] = &["appx", "msixbundle", "appxbundle"];

#[derive(Debug, Clone)]
pub struct InstallerOptions {
    pub extensions: Vec<String>,
    /// The type folder in Downloads that holds sorted installers.
    pub folder: String,
    pub min_age: Duration,
    pub now: SystemTime,
}

impl InstallerOptions {
    pub fn new(settings: &Settings, now: SystemTime) -> InstallerOptions {
        // The "Installers" type folder, or (if the user renamed it) the one
        // that holds .exe / .msi files.
        let has = |c: &&crate::config::Category, e: &str| c.extensions.iter().any(|x| x.trim().eq_ignore_ascii_case(e));
        let cat = settings
            .categories
            .iter()
            .find(|c| c.name.trim().eq_ignore_ascii_case(INSTALLERS))
            .or_else(|| settings.categories.iter().find(|c| has(c, "exe") || has(c, "msi")));
        let mut extensions: Vec<String> = match cat {
            Some(c) => c.extensions.iter().map(|e| e.trim().trim_start_matches('.').to_lowercase()).collect(),
            None => vec!["exe".into(), "msi".into(), "msix".into()],
        };
        extensions.extend(EXTRA_EXTENSIONS.iter().map(|e| e.to_string()));
        extensions.retain(|e| !e.is_empty());
        let folder = cat.map(|c| safe_folder_name(&c.name)).unwrap_or_else(|| INSTALLERS.to_string());
        InstallerOptions {
            extensions,
            folder,
            min_age: Duration::from_secs(u64::from(settings.installer_days) * 86_400),
            now,
        }
    }

    fn is_installer(&self, p: &Path) -> bool {
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        !ext.is_empty() && self.extensions.contains(&ext)
    }
}

#[derive(Debug, Clone, Default)]
pub struct InstallerScan {
    /// Oldest first.
    pub files: Vec<ScannedFile>,
    pub skipped: Skipped,
    pub files_checked: u64,
}

/// True for a "YYYY-MM" folder name.
pub fn is_month_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 7 && b[4] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || c.is_ascii_digit())
}

/// Finds old installers. Nothing on disk is changed.
pub fn find_installers(roots: &Roots, opts: &InstallerOptions, progress: &Progress) -> io::Result<InstallerScan> {
    roots.validate()?;
    let mut scan = InstallerScan::default();
    let downloads = roots.get(RootKind::Downloads).to_path_buf();
    let mut dirs = vec![downloads.clone()];
    let type_dir = downloads.join(&opts.folder);
    if let Some(ok) = enterable(&type_dir, &mut scan.skipped) {
        if ok {
            for month in list(&type_dir, &mut scan.skipped) {
                let name = month.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                if is_month_name(&name) && enterable(&month, &mut scan.skipped) == Some(true) {
                    dirs.push(month);
                }
            }
        }
    }
    let mut found = Vec::new();
    for dir in dirs {
        for path in list(&dir, &mut scan.skipped) {
            progress.check()?;
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            match check_meta(&path, &meta, GRACE, opts.now) {
                Check::Folder => {}
                Check::Skip(r) => {
                    if opts.is_installer(&path) {
                        scan.skipped.push(path, r);
                    }
                }
                Check::Ok { size, modified } => {
                    progress.files.fetch_add(1, Ordering::Relaxed);
                    let old = opts.now.duration_since(modified).is_ok_and(|age| age > opts.min_age);
                    if opts.is_installer(&path) && old {
                        found.push(ScannedFile { root: RootKind::Downloads, path, size, modified });
                    }
                }
            }
        }
    }
    found.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| path_key(&a.path).cmp(&path_key(&b.path))));
    for f in found {
        progress.check()?;
        match open_exclusive(&f.path) {
            Ok(()) => scan.files.push(f),
            Err(e) => scan.skipped.push(f.path, SkipReason::InUse(e.to_string())),
        }
    }
    scan.files_checked = progress.files.load(Ordering::Relaxed);
    Ok(scan)
}

/// `None` if the folder does not exist, `Some(true)` if it may be entered,
/// `Some(false)` (reason recorded) if not.
fn enterable(dir: &Path, skipped: &mut Skipped) -> Option<bool> {
    let meta = fs::symlink_metadata(dir).ok()?;
    let reason = folder_skip_reason(dir, &meta).or_else(|| program_folder_on_disk(dir));
    match reason {
        None => Some(true),
        Some(r) => {
            skipped.push(dir.to_path_buf(), r);
            Some(false)
        }
    }
}

fn list(dir: &Path, skipped: &mut Skipped) -> Vec<PathBuf> {
    match fs::read_dir(dir) {
        Ok(rd) => {
            let mut v: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            v.sort_by_key(|p| path_key(p));
            v
        }
        Err(e) => {
            skipped.errors.push((dir.to_path_buf(), e.to_string()));
            Vec::new()
        }
    }
}
