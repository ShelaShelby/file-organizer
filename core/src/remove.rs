//! Sending chosen files to the Recycle Bin, with every file checked again
//! right before it goes, and a journal line for each one.

use crate::fsops::path_key;
use crate::journal::{BatchReport, Entry, Journal, Record};
use crate::paths::{RootKind, Roots};
use crate::recycle::Recycler;
use crate::safety::{check_entry, folder_skip_reason, Check, SkipReason};
use crate::tree::{ScannedFile, GRACE};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

/// The root `path` lies in, if every folder between that root and the file
/// is a normal folder (no link, junction, hidden/system or protected
/// folder). Anything else is refused.
pub fn root_of(roots: &Roots, path: &Path) -> Result<RootKind, String> {
    if path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err("path contains '..' or '.'".into());
    }
    for kind in RootKind::ALL {
        let root = roots.get(kind);
        let Ok(rest) = strip_prefix_ci(path, root) else {
            continue;
        };
        let mut dir = root.to_path_buf();
        let parts: Vec<_> = rest.components().collect();
        if parts.is_empty() {
            return Err("is a chosen folder itself".into());
        }
        for c in &parts[..parts.len() - 1] {
            dir.push(c);
            let ok = fs::symlink_metadata(&dir).ok().filter(|m| folder_skip_reason(&dir, m).is_none());
            if ok.is_none() {
                return Err(format!("folder is not safe to touch: {}", dir.display()));
            }
        }
        return Ok(kind);
    }
    Err("not inside Downloads, Desktop or Documents".into())
}

fn strip_prefix_ci<'a>(path: &'a Path, base: &Path) -> Result<&'a Path, ()> {
    let base_parts = base.components().count();
    let mut it = path.components();
    let head: PathBuf = it.by_ref().take(base_parts).collect();
    if path_key(&head) == path_key(base) {
        Ok(it.as_path())
    } else {
        Err(())
    }
}

/// Re-checks one file (skip rules, lock, same size and last-changed time as
/// at the scan) and sends it to the Recycle Bin. Result goes into `report`.
pub fn recycle_checked(
    roots: &Roots,
    f: &ScannedFile,
    journal: &Journal,
    recycler: &dyn Recycler,
    now: SystemTime,
    report: &mut BatchReport,
) -> bool {
    if let Err(e) = root_of(roots, &f.path) {
        report.errors.push((f.path.clone(), e));
        return false;
    }
    if !still_same(f, now, report) {
        return false;
    }
    match recycler.recycle(&f.path) {
        Ok(place) => {
            let entry = Entry::new(report.batch, &f.path, &place.unwrap_or_default(), f.size, f.modified);
            if let Err(e) = journal.append(&Record::Recycle(entry)) {
                report.errors.push((
                    f.path.clone(),
                    format!("sent to the Recycle Bin, but the journal could not be written: {e}"),
                ));
            }
            report.recycled.push(f.path.clone());
            true
        }
        Err(e) => {
            if f.path.symlink_metadata().is_ok() {
                report.errors.push((f.path.clone(), format!("not sent to the Recycle Bin, file left in place: {e}")));
            } else {
                report.errors.push((f.path.clone(), format!("Recycle Bin reported an error: {e}")));
            }
            false
        }
    }
}

/// True if the file passes every skip rule and is unchanged since the scan;
/// otherwise the reason is added to `report.skipped`.
pub fn still_same(f: &ScannedFile, now: SystemTime, report: &mut BatchReport) -> bool {
    match check_entry(&f.path, GRACE, now) {
        Check::Ok { size, modified } if size == f.size && modified == f.modified => true,
        Check::Ok { .. } => {
            report.skipped.push((f.path.clone(), SkipReason::Changed.to_string()));
            false
        }
        Check::Folder => {
            report.skipped.push((f.path.clone(), SkipReason::NotAFile.to_string()));
            false
        }
        Check::Skip(r) => {
            report.skipped.push((f.path.clone(), r.to_string()));
            false
        }
    }
}

/// Sends the given files to the Recycle Bin as one batch (used by the
/// big / old lists). Each file is checked again first.
pub fn recycle_files(
    roots: &Roots,
    files: &[ScannedFile],
    journal: &Journal,
    recycler: &dyn Recycler,
    now: SystemTime,
) -> BatchReport {
    let _lock = match journal.lock() {
        Ok(l) => l,
        Err(e) => {
            let mut report = BatchReport::default();
            report.errors.push((journal.path().to_path_buf(), format!("journal is not available: {e}")));
            return report;
        }
    };
    let mut report = BatchReport { batch: journal.next_batch_id(), ..Default::default() };
    if let Err(e) = roots.validate() {
        report.errors.push((PathBuf::new(), e.to_string()));
        return report;
    }
    for f in files {
        recycle_checked(roots, f, journal, recycler, now, &mut report);
    }
    report
}
