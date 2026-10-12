//! Walks the three roots INCLUDING sub-folders (used by the duplicate
//! finder and the big/old lists). Applies the same skip rules as stage 1;
//! links, junctions, hidden/system folders and protected places are never
//! entered, and neither are program or project folders.

use crate::fsops::path_key;
use crate::paths::{RootKind, Roots};
use crate::safety::{check_meta, folder_skip_reason, program_folder_reason, Check, SkipReason};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
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
        let root = roots.get(kind).to_path_buf();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            progress.check()?;
            if !seen_dirs.insert(path_key(&dir)) {
                continue; // overlapping roots: each folder once
            }
            if let Err(e) = walk_dir(
                &dir,
                dir != root,
                kind,
                now,
                progress,
                skipped,
                &mut stack,
                &mut found,
            ) {
                skipped.errors.push((dir, e.to_string()));
            }
        }
    }
    Ok(())
}

/// Lists one folder: files that pass go to `found`, sub-folders that may be
/// entered go on `stack`. A program or project folder (not a root) is left
/// alone as a whole.
#[allow(clippy::too_many_arguments)]
fn walk_dir(
    dir: &Path,
    is_sub: bool,
    kind: RootKind,
    now: SystemTime,
    progress: &Progress,
    skipped: &mut Skipped,
    stack: &mut Vec<PathBuf>,
    found: &mut impl FnMut(ScannedFile),
) -> io::Result<()> {
    let mut paths = Vec::new();
    for e in fs::read_dir(dir)? {
        match e {
            Ok(e) => paths.push(e.path()),
            Err(e) => skipped.errors.push((dir.to_path_buf(), e.to_string())),
        }
    }
    if is_sub {
        let names: Vec<String> = paths
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .collect();
        if let Some(r) = program_folder_reason(&names) {
            skipped.push(dir.to_path_buf(), r);
            return Ok(());
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
                found(ScannedFile {
                    root: kind,
                    path,
                    size,
                    modified,
                });
            }
        }
    }
    // Reverse so folders are visited in name order.
    stack.extend(subdirs.into_iter().rev());
    Ok(())
}

/// "2026-10-12" (local time) for a last-changed time.
pub fn date_text(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%Y-%m-%d")
        .to_string()
}
