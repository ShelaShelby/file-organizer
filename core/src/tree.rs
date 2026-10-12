//! Walks the three roots INCLUDING sub-folders (used by the duplicate
//! finder and the big/old lists). Applies the same skip rules as stage 1;
//! links, junctions, hidden/system folders and protected places are never
//! entered.

use crate::fsops::path_key;
use crate::paths::{RootKind, Roots};
use crate::safety::{check_meta, folder_skip_reason, Check, SkipReason};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Files changed in the last 2 minutes may still be downloading: skipped.
pub const GRACE: Duration = Duration::from_secs(2 * 60);

/// At most this many skipped entries are kept for display (all are counted).
pub const MAX_LISTED_SKIPS: usize = 1000;

/// One normal file found while walking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    pub root: RootKind,
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

/// Shared between the background scan and the window: counters for the
/// progress line and the Cancel flag.
#[derive(Debug, Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub hashed: AtomicU64,
    cancel: AtomicBool,
}

impl Progress {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn check(&self) -> io::Result<()> {
        if self.is_cancelled() {
            Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
        } else {
            Ok(())
        }
    }
}

/// What was left alone while walking.
#[derive(Debug, Clone, Default)]
pub struct Skipped {
    pub count: usize,
    pub items: Vec<(PathBuf, SkipReason)>,
    pub errors: Vec<(PathBuf, String)>,
}

impl Skipped {
    pub fn push(&mut self, p: PathBuf, r: SkipReason) {
        self.count += 1;
        if self.items.len() < MAX_LISTED_SKIPS {
            self.items.push((p, r));
        }
    }
}

/// Calls `found` for every file that passes the skip rules (no lock check
/// yet). Returns `Interrupted` if cancelled.
pub fn walk(
    roots: &Roots,
    now: SystemTime,
    progress: &Progress,
    skipped: &mut Skipped,
    mut found: impl FnMut(ScannedFile),
) -> io::Result<()> {
    roots.validate()?;
    let mut seen_dirs: HashSet<String> = HashSet::new();
    for kind in RootKind::ALL {
        let mut stack = vec![roots.get(kind).to_path_buf()];
        while let Some(dir) = stack.pop() {
            progress.check()?;
            if !seen_dirs.insert(path_key(&dir)) {
                continue; // overlapping roots: each folder once
            }
            let entries = match fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(e) => {
                    skipped.errors.push((dir, e.to_string()));
                    continue;
                }
            };
            let mut paths = Vec::new();
            for e in entries {
                match e {
                    Ok(e) => paths.push(e.path()),
                    Err(e) => skipped.errors.push((dir.clone(), e.to_string())),
                }
            }
            paths.sort_by_key(|p| path_key(p));
            let mut subdirs = Vec::new();
            for path in paths {
                let meta = match fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(e) => {
                        skipped.push(path, SkipReason::InUse(e.to_string()));
                        continue;
                    }
                };
                match check_meta(&path, &meta, GRACE, now) {
                    Check::Folder => match folder_skip_reason(&path, &meta) {
                        None => subdirs.push(path),
                        Some(r) => skipped.push(path, r),
                    },
                    Check::Skip(r) => skipped.push(path, r),
                    Check::Ok { size, modified } => {
                        progress.files.fetch_add(1, Ordering::Relaxed);
                        found(ScannedFile { root: kind, path, size, modified });
                    }
                }
            }
            // Reverse so folders are visited in name order.
            stack.extend(subdirs.into_iter().rev());
        }
    }
    Ok(())
}

/// "2026-10-12" (local time) for a last-changed time.
pub fn date_text(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d").to_string()
}
