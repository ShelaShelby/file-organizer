//! Lists of big files and old files (last changed long ago) in the three
//! roots, with sub-folders. List only: nothing is changed here.

use crate::paths::Roots;
use crate::safety::open_exclusive;
use crate::tree::{walk, Progress, ScannedFile, Skipped};
use crate::Settings;
use std::io;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

/// Rows shown per list.
pub const TOP: usize = 200;

#[derive(Debug, Clone)]
pub struct BigOldOptions {
    /// Files this big or bigger are "big".
    pub big_bytes: u64,
    /// Files not changed for this long are "old".
    pub old_age: Duration,
    pub now: SystemTime,
    pub limit: usize,
}

impl BigOldOptions {
    pub fn new(settings: &Settings, now: SystemTime) -> Self {
        BigOldOptions {
            big_bytes: settings.big_file_mb.saturating_mul(1024 * 1024),
            old_age: Duration::from_secs(u64::from(settings.old_days) * 86_400),
            now,
            limit: TOP,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BigOldScan {
    /// Biggest first.
    pub big: Vec<ScannedFile>,
    /// Oldest first.
    pub old: Vec<ScannedFile>,
    pub skipped: Skipped,
    pub files_checked: u64,
}

pub fn find_big_old(roots: &Roots, opts: &BigOldOptions, progress: &Progress) -> io::Result<BigOldScan> {
    let mut scan = BigOldScan::default();
    let mut big = Vec::new();
    let mut old = Vec::new();
    walk(roots, opts.now, progress, &mut scan.skipped, |f| {
        if f.size >= opts.big_bytes {
            big.push(f.clone());
        }
        if opts.now.duration_since(f.modified).is_ok_and(|age| age >= opts.old_age) {
            old.push(f);
        }
    })?;
    scan.files_checked = progress.files.load(Ordering::Relaxed);
    big.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    old.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| a.path.cmp(&b.path)));
    scan.big = top_unlocked(big, opts.limit, &mut scan.skipped, progress)?;
    scan.old = top_unlocked(old, opts.limit, &mut scan.skipped, progress)?;
    Ok(scan)
}

/// First `limit` files that are not in use by another program.
fn top_unlocked(
    files: Vec<ScannedFile>,
    limit: usize,
    skipped: &mut Skipped,
    progress: &Progress,
) -> io::Result<Vec<ScannedFile>> {
    let mut out = Vec::new();
    for f in files {
        if out.len() >= limit {
            break;
        }
        progress.check()?;
        match open_exclusive(&f.path) {
            Ok(()) => out.push(f),
            Err(e) => skipped.push(f.path, crate::safety::SkipReason::InUse(e.to_string())),
        }
    }
    Ok(out)
}
