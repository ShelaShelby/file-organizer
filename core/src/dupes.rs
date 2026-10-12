//! Duplicate finder: files with identical content in the three roots
//! (with sub-folders). Grouped by size, then a hash of the first block,
//! then a full BLAKE3 hash. Removal keeps at least one copy of every group.

use crate::fsops::path_key;
use crate::journal::{BatchReport, Journal};
use crate::paths::Roots;
use crate::recycle::Recycler;
use crate::remove::{recycle_checked, still_same};
use crate::safety::open_exclusive_file;
use crate::tree::{walk, Progress, ScannedFile, Skipped};
use std::collections::{HashMap, HashSet};
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

/// Size of the first block used for the quick hash.
pub const QUICK_BLOCK: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub struct DupOptions {
    /// Files smaller than this are ignored (empty files always are).
    pub min_size: u64,
    pub now: SystemTime,
}

impl DupOptions {
    pub fn new(settings: &crate::Settings, now: SystemTime) -> Self {
        DupOptions { min_size: settings.min_duplicate_kb.saturating_mul(1024), now }
    }
}

/// Files with the same content. `files` is sorted oldest last-changed first.
#[derive(Debug, Clone)]
pub struct DupGroup {
    pub size: u64,
    pub files: Vec<ScannedFile>,
}

impl DupGroup {
    /// Default choice: keep only the copy with the oldest last-changed date.
    pub fn default_keep(&self) -> Vec<bool> {
        (0..self.files.len()).map(|i| i == 0).collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct DupScan {
    /// Biggest space to win first.
    pub groups: Vec<DupGroup>,
    pub skipped: Skipped,
    pub files_checked: u64,
}

/// Finds duplicate groups. Nothing on disk is changed. Returns an
/// `Interrupted` error if `progress.cancel()` was called.
pub fn find_duplicates(roots: &Roots, opts: &DupOptions, progress: &Progress) -> io::Result<DupScan> {
    let min = opts.min_size.max(1);
    let mut scan = DupScan::default();
    let mut by_size: HashMap<u64, Vec<ScannedFile>> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    walk(roots, opts.now, progress, &mut scan.skipped, |f| {
        if f.size >= min && seen.insert(path_key(&f.path)) {
            by_size.entry(f.size).or_default().push(f);
        }
    })?;
    scan.files_checked = progress.files.load(Ordering::Relaxed);

    let mut sizes: Vec<u64> = by_size.iter().filter(|(_, v)| v.len() > 1).map(|(s, _)| *s).collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    for size in sizes {
        let files = by_size.remove(&size).unwrap_or_default();
        for quick in split_by(files, &mut scan.skipped, progress, |p| hash_file(p, Some(QUICK_BLOCK), progress))? {
            let full_needed = size > QUICK_BLOCK as u64;
            let groups = if full_needed {
                split_by(quick, &mut scan.skipped, progress, |p| hash_file(p, None, progress))?
            } else {
                vec![quick] // the quick hash already covered the whole file
            };
            for mut files in groups {
                files.sort_by(|a, b| {
                    a.modified.cmp(&b.modified).then_with(|| path_key(&a.path).cmp(&path_key(&b.path)))
                });
                scan.groups.push(DupGroup { size, files });
            }
        }
    }
    scan.groups.sort_by(|a, b| {
        let wa = a.size * (a.files.len() as u64 - 1);
        let wb = b.size * (b.files.len() as u64 - 1);
        wb.cmp(&wa)
    });
    Ok(scan)
}

/// Splits files into groups (of 2 or more) with equal key. Files that cannot
/// be read (locked, gone) are reported as skipped.
fn split_by(
    files: Vec<ScannedFile>,
    skipped: &mut Skipped,
    progress: &Progress,
    key: impl Fn(&std::path::Path) -> io::Result<[u8; 32]>,
) -> io::Result<Vec<Vec<ScannedFile>>> {
    let mut map: HashMap<[u8; 32], Vec<ScannedFile>> = HashMap::new();
    let mut order: Vec<[u8; 32]> = Vec::new();
    for f in files {
        progress.check()?;
        match key(&f.path) {
            Ok(k) => {
                let v = map.entry(k).or_default();
                if v.is_empty() {
                    order.push(k);
                }
                v.push(f);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(e),
            Err(e) => skipped.push(f.path, crate::safety::SkipReason::InUse(e.to_string())),
        }
    }
    Ok(order.into_iter().filter_map(|k| map.remove(&k)).filter(|v| v.len() > 1).collect())
}

/// BLAKE3 of the first `limit` bytes (or the whole file). The file is opened
/// without sharing, so locked files fail here and are skipped.
pub fn hash_file(path: &std::path::Path, limit: Option<usize>, progress: &Progress) -> io::Result<[u8; 32]> {
    let mut f = open_exclusive_file(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut left = limit.unwrap_or(usize::MAX);
    while left > 0 {
        progress.check()?;
        let want = buf.len().min(left);
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        left -= n;
    }
    if limit.is_none() {
        progress.hashed.fetch_add(1, Ordering::Relaxed);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// The user's choice for one group: which copies to keep.
#[derive(Debug, Clone)]
pub struct GroupChoice {
    pub files: Vec<ScannedFile>,
    pub keep: Vec<bool>,
}

impl GroupChoice {
    pub fn is_valid(&self) -> bool {
        self.keep.len() == self.files.len() && self.keep.iter().any(|k| *k)
    }

    pub fn to_remove(&self) -> impl Iterator<Item = &ScannedFile> {
        self.files.iter().zip(&self.keep).filter(|(_, k)| !**k).map(|(f, _)| f)
    }
}

/// Error text if any group would lose all of its copies (Confirm must then
/// be blocked), `None` if the choice is fine.
pub fn check_choices(choices: &[GroupChoice]) -> Option<String> {
    let bad = choices.iter().filter(|c| !c.is_valid()).count();
    (bad > 0).then(|| {
        if bad == 1 {
            "1 group would lose every copy: tick \"Keep\" on one.".to_string()
        } else {
            format!("{bad} groups would lose every copy: tick \"Keep\" on one in each.")
        }
    })
}

/// Bytes freed if the choice is carried out.
pub fn space_freed(choices: &[GroupChoice]) -> u64 {
    choices.iter().flat_map(|c| c.to_remove()).map(|f| f.size).sum()
}

/// Sends the copies NOT marked "keep" to the Recycle Bin. A whole group is
/// left alone if it has no kept copy, or if a kept copy is gone or changed
/// since the scan, so at least one copy always stays.
pub fn remove_duplicates(
    roots: &Roots,
    choices: &[GroupChoice],
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
    for c in choices {
        let group = c.files.first().map(|f| f.path.clone()).unwrap_or_default();
        if !c.is_valid() {
            report.errors.push((group, "group would lose every copy; nothing removed from it".into()));
            continue;
        }
        let kept: Vec<&ScannedFile> = c.files.iter().zip(&c.keep).filter(|(_, k)| **k).map(|(f, _)| f).collect();
        let kept_keys: HashSet<String> = kept.iter().map(|f| path_key(&f.path)).collect();
        if c.to_remove().any(|f| kept_keys.contains(&path_key(&f.path))) {
            report.errors.push((group, "the same file is both kept and removed; nothing removed from it".into()));
            continue;
        }
        let mut check = BatchReport::default();
        if !kept.iter().all(|f| still_same(f, now, &mut check)) {
            for f in c.to_remove() {
                report.skipped.push((f.path.clone(), "the copy to keep is gone or changed since the scan".into()));
            }
            continue;
        }
        // Compare contents again right now (full hash, no sharing), once for
        // the kept copy and once for each copy that goes.
        let progress = Progress::default();
        let kept_hash = match hash_file(&kept[0].path, None, &progress) {
            Ok(h) => h,
            Err(e) => {
                for f in c.to_remove() {
                    report.skipped.push((f.path.clone(), format!("the copy to keep cannot be read now ({e})")));
                }
                continue;
            }
        };
        for f in c.to_remove() {
            match hash_file(&f.path, None, &progress) {
                Ok(h) if h == kept_hash => {
                    recycle_checked(roots, f, journal, recycler, now, &mut report);
                }
                Ok(_) => report.skipped.push((f.path.clone(), "content is no longer the same as the kept copy".into())),
                Err(e) => report.skipped.push((f.path.clone(), format!("cannot be read to compare again ({e})"))),
            }
        }
    }
    report
}
