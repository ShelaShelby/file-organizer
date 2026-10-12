//! Scan the roots, build the preview list, and carry out confirmed moves.

use crate::config::{safe_folder_name, Settings};
use crate::fsops::{free_target, has_dot_parts, move_into, path_key};
use crate::journal::{BatchReport, Entry, Journal, Record};
use crate::paths::{RootKind, Roots};
use crate::safety::{check_entry, check_entry_unlocked, Check, SkipReason};
use chrono::{DateTime, Local};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Downloads: files changed in the last 2 minutes are left alone.
pub const DOWNLOADS_GRACE: Duration = Duration::from_secs(2 * 60);

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub now: SystemTime,
    /// Desktop and Documents: skip files changed in the last N days.
    pub recent_days: u32,
    pub downloads_grace: Duration,
}

impl ScanOptions {
    pub fn new(settings: &Settings, now: SystemTime) -> Self {
        ScanOptions {
            now,
            recent_days: settings.recent_days,
            downloads_grace: DOWNLOADS_GRACE,
        }
    }

    pub fn min_age(&self, kind: RootKind) -> Duration {
        match kind {
            RootKind::Downloads => self.downloads_grace,
            RootKind::Desktop | RootKind::Documents => {
                Duration::from_secs(u64::from(self.recent_days) * 86_400)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedMove {
    pub root: RootKind,
    pub source: PathBuf,
    /// Planned destination (a free name at scan time; re-checked on Confirm).
    pub target: PathBuf,
    pub category: String,
    pub size: u64,
    pub modified: SystemTime,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub moves: Vec<PlannedMove>,
    pub skipped: Vec<(PathBuf, SkipReason)>,
    /// Roots that could not be read, etc.
    pub errors: Vec<(PathBuf, String)>,
}

/// "YYYY-MM" of a time, in local time.
pub fn month_folder(t: SystemTime) -> String {
    DateTime::<Local>::from(t).format("%Y-%m").to_string()
}

/// Look at the LOOSE files at the top level of each root and plan where
/// each one should go. Nothing on disk is changed.
pub fn scan(roots: &Roots, settings: &Settings, opts: &ScanOptions) -> io::Result<Plan> {
    roots.validate()?;
    let mut plan = Plan::default();
    let mut taken: HashSet<String> = HashSet::new();
    let mut seen_roots: HashSet<String> = HashSet::new();
    for kind in RootKind::ALL {
        if !seen_roots.insert(path_key(roots.get(kind))) {
            continue; // two roots pointing at the same folder: scan once
        }
        scan_root(roots, settings, opts, kind, &mut taken, &mut plan);
    }
    Ok(plan)
}

/// Plans the loose files at the top level of one root.
pub fn scan_root(
    roots: &Roots,
    settings: &Settings,
    opts: &ScanOptions,
    kind: RootKind,
    taken: &mut HashSet<String>,
    plan: &mut Plan,
) {
    scan_root_with(roots, settings, opts, kind, taken, plan, true)
}

/// [`scan_root`]; with `open_check` false files are NOT opened (only their
/// metadata is read; background sorting opens just the few files it is
/// about to move, and every move re-checks anyway).
pub fn scan_root_with(
    roots: &Roots,
    settings: &Settings,
    opts: &ScanOptions,
    kind: RootKind,
    taken: &mut HashSet<String>,
    plan: &mut Plan,
    open_check: bool,
) {
    let root = roots.get(kind);
    let entries = match fs::read_dir(root) {
        Ok(rd) => rd,
        Err(e) => {
            plan.errors.push((root.to_path_buf(), e.to_string()));
            return;
        }
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        match entry {
            Ok(e) => paths.push(e.path()),
            Err(e) => plan.errors.push((root.to_path_buf(), e.to_string())),
        }
    }
    paths.sort_by_key(|p| path_key(p));
    for path in paths {
        let check = if open_check {
            check_entry(&path, opts.min_age(kind), opts.now)
        } else {
            check_entry_unlocked(&path, opts.min_age(kind), opts.now)
        };
        match check {
            Check::Folder => {}
            Check::Skip(r) => plan.skipped.push((path, r)),
            Check::Ok { size, modified } => {
                let name = file_name(&path);
                let category = settings.category_for(&name);
                let dir = target_dir(roots, kind, &category, modified);
                let target = free_target(&dir, &name, taken);
                taken.insert(path_key(&target));
                plan.moves.push(PlannedMove {
                    root: kind,
                    source: path,
                    target,
                    category,
                    size,
                    modified,
                });
            }
        }
    }
}

fn target_dir(roots: &Roots, kind: RootKind, category: &str, modified: SystemTime) -> PathBuf {
    roots
        .target_base(kind)
        .join(category)
        .join(month_folder(modified))
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// The folder a planned move may go to. Nothing in `m` is trusted: the
/// source must be a loose file directly inside its root, the folder is
/// recomputed from root + type folder + month, and the planned target must
/// lie in exactly that folder (compared ignoring case, no `..` or `.`).
fn checked_target_dir(roots: &Roots, m: &PlannedMove) -> Result<PathBuf, &'static str> {
    if has_dot_parts(&m.source) || has_dot_parts(&m.target) {
        return Err("path contains '..' or '.'");
    }
    if m.source.parent().map(path_key) != Some(path_key(roots.get(m.root))) {
        return Err("not a loose file in a chosen folder");
    }
    if safe_folder_name(&m.category) != m.category {
        return Err("unexpected type folder");
    }
    let dir = target_dir(roots, m.root, &m.category, m.modified);
    if m.target.parent().map(path_key) != Some(path_key(&dir)) {
        return Err("unexpected target folder");
    }
    Ok(dir)
}

/// Carry out the moves the user confirmed. Each file is checked again just
/// before it is moved; a problem with one file never stops the batch.
pub fn execute(
    roots: &Roots,
    moves: &[PlannedMove],
    journal: &Journal,
    opts: &ScanOptions,
) -> BatchReport {
    execute_tagged(roots, moves, journal, opts, "")
}

/// [`execute`], with every journal line tagged (e.g. "auto" for background
/// sorting). Holds the journal lock for the whole batch.
pub fn execute_tagged(
    roots: &Roots,
    moves: &[PlannedMove],
    journal: &Journal,
    opts: &ScanOptions,
    tag: &str,
) -> BatchReport {
    let _lock = match journal.lock() {
        Ok(l) => l,
        Err(e) => {
            let mut report = BatchReport::default();
            report.errors.push((
                journal.path().to_path_buf(),
                format!("journal is not available: {e}"),
            ));
            return report;
        }
    };
    let mut report = BatchReport {
        batch: journal.next_batch_id(),
        ..Default::default()
    };
    if let Err(e) = roots.validate() {
        report.errors.push((PathBuf::new(), e.to_string()));
        return report;
    }
    for m in moves {
        let dir = match checked_target_dir(roots, m) {
            Ok(d) => d,
            Err(e) => {
                report.errors.push((m.source.clone(), e.into()));
                continue;
            }
        };
        match check_entry(&m.source, opts.min_age(m.root), opts.now) {
            Check::Ok { size, modified } if size == m.size && modified == m.modified => {}
            Check::Ok { .. } => {
                report
                    .skipped
                    .push((m.source.clone(), SkipReason::Changed.to_string()));
                continue;
            }
            Check::Folder => {
                report
                    .skipped
                    .push((m.source.clone(), SkipReason::NotAFile.to_string()));
                continue;
            }
            Check::Skip(r) => {
                report.skipped.push((m.source.clone(), r.to_string()));
                continue;
            }
        }
        let name = file_name(&m.source);
        let new = match move_into(&m.source, &dir, &name) {
            Ok(p) => p,
            Err(e) => {
                report.errors.push((m.source.clone(), e.to_string()));
                continue;
            }
        };
        let entry = Entry::new(report.batch, &m.source, &new, m.size, m.modified).tagged(tag);
        if let Err(e) = journal.append(&Record::Move(entry)) {
            // Without a journal line the move could not be undone: put it back.
            let msg = match crate::fsops::move_no_replace(&new, &m.source) {
                Ok(()) => format!("journal could not be written ({e}); file left in place"),
                Err(e2) => format!(
                    "journal could not be written ({e}); file is now at {} ({e2})",
                    new.display()
                ),
            };
            report.errors.push((m.source.clone(), msg));
            continue;
        }
        report.moved.push((m.source.clone(), new));
    }
    report
}
