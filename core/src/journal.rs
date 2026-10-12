//! Journal of every move and rename (JSON lines in
//! `%APPDATA%\file-organizer\journal.jsonl`) and undo. Every batch holds
//! [`Journal::lock`] (a lock file next to the journal) while it runs.

use crate::fsops::{move_no_replace, path_key};
use crate::paths::{Roots, FROM_DESKTOP};
use crate::remove::{journal_path_ok, root_of};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const JOURNAL_FILE: &str = "journal.jsonl";
/// Tag of moves made by background sorting (`--tray`).
pub const TAG_AUTO: &str = "auto";
/// Tag of renames made in the Rename tab.
pub const TAG_RENAME: &str = "rename";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub batch: u64,
    pub time: String,
    pub old: PathBuf,
    pub new: PathBuf,
    pub size: u64,
    /// Last-changed time of the file, in nanoseconds since 1970.
    pub modified_ns: i64,
    /// Who made the change: empty = Sort tab, [`TAG_AUTO`] = background
    /// sorting, [`TAG_RENAME`] = Rename tab.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
}

impl Entry {
    pub fn new(batch: u64, old: &Path, new: &Path, size: u64, modified: SystemTime) -> Entry {
        Entry {
            batch,
            time: Local::now().to_rfc3339(),
            old: old.to_path_buf(),
            new: new.to_path_buf(),
            size,
            modified_ns: to_ns(modified),
            tag: String::new(),
        }
    }

    pub fn tagged(mut self, tag: &str) -> Entry {
        self.tag = tag.to_string();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Record {
    /// A file was moved from `old` to `new`.
    Move(Entry),
    /// That move was reversed.
    Undo(Entry),
    /// The file at `old` was sent to the Recycle Bin (`new` = its place in
    /// the Recycle Bin if Windows reported it, else empty).
    Recycle(Entry),
}

/// Result of one batch (organise or undo).
#[derive(Debug, Clone, Default)]
pub struct BatchReport {
    pub batch: u64,
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// Files sent to the Recycle Bin.
    pub recycled: Vec<PathBuf>,
    pub skipped: Vec<(PathBuf, String)>,
    pub errors: Vec<(PathBuf, String)>,
}

impl BatchReport {
    pub fn summary(&self) -> String {
        format!(
            "{} moved, {} skipped, {} errors",
            self.moved.len(),
            self.skipped.len(),
            self.errors.len()
        )
    }

    pub fn recycle_summary(&self) -> String {
        format!(
            "{} sent to the Recycle Bin, {} skipped, {} errors",
            self.recycled.len(),
            self.skipped.len(),
            self.errors.len()
        )
    }
}

/// One batch in the history list.
#[derive(Debug, Clone)]
pub struct BatchSummary {
    pub batch: u64,
    pub time: String,
    /// Tag of the batch's entries ("", "auto" or "rename").
    pub tag: String,
    pub total: usize,
    /// Moves of this batch that have not been undone yet.
    pub remaining: Vec<Entry>,
    /// Files this batch sent to the Recycle Bin (restore them from there).
    pub recycled: Vec<Entry>,
}

pub struct Journal {
    path: PathBuf,
}

/// Held while a batch checks, moves and journals files. The main window and
/// the background sorter are separate processes; the lock makes sure they
/// never work at the same time, so a file cannot be moved twice and journal
/// lines never mix. Released when dropped (or when the process ends).
pub struct JournalLock {
    _file: fs::File,
}

impl Journal {
    pub fn new(path: impl Into<PathBuf>) -> Journal {
        Journal { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("lock")
    }

    fn open_lock_file(&self) -> io::Result<fs::File> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.lock_path())
    }

    /// Waits until no other batch (in any process) is running, then holds
    /// the lock until the returned guard is dropped.
    pub fn lock(&self) -> io::Result<JournalLock> {
        let file = self.open_lock_file()?;
        file.lock()?;
        Ok(JournalLock { _file: file })
    }

    /// Like [`Journal::lock`] but returns `None` at once if another batch
    /// is running.
    pub fn try_lock(&self) -> io::Result<Option<JournalLock>> {
        let file = self.open_lock_file()?;
        match file.try_lock() {
            Ok(()) => Ok(Some(JournalLock { _file: file })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(e)) => Err(e),
        }
    }

    /// Writes one line and flushes it to disk. If the file does not end
    /// with a line break (a crash in the middle of a line), one is written
    /// first so the broken line never swallows this record.
    pub fn append(&self, rec: &Record) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut line = serde_json::to_string(rec).map_err(io::Error::other)?;
        line.push('\n');
        let mut f = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)?;
        let len = f.metadata()?.len();
        if len > 0 {
            let mut last = [0u8; 1];
            f.seek(SeekFrom::Start(len - 1))?;
            f.read_exact(&mut last)?;
            if last[0] != b'\n' {
                line.insert(0, '\n');
            }
        }
        f.write_all(line.as_bytes())?;
        f.sync_data()
    }

    /// All readable records (unreadable lines are ignored).
    pub fn records(&self) -> io::Result<Vec<Record>> {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        Ok(text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect())
    }

    /// A new batch id, always larger than any already in the journal.
    pub fn next_batch_id(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last = self
            .records()
            .unwrap_or_default()
            .iter()
            .map(|r| match r {
                Record::Move(e) | Record::Undo(e) | Record::Recycle(e) => e.batch,
            })
            .max()
            .unwrap_or(0);
        now.max(last + 1)
    }

    /// Batches, newest first.
    pub fn history(&self) -> io::Result<Vec<BatchSummary>> {
        let records = self.records()?;
        let undone: HashSet<(u64, String)> = records
            .iter()
            .filter_map(|r| match r {
                Record::Undo(e) => Some((e.batch, path_key(&e.new))),
                _ => None,
            })
            .collect();
        let mut map: BTreeMap<u64, BatchSummary> = BTreeMap::new();
        for r in records {
            let (e, recycled) = match r {
                Record::Move(e) => (e, false),
                Record::Recycle(e) => (e, true),
                Record::Undo(_) => continue,
            };
            let s = map.entry(e.batch).or_insert_with(|| BatchSummary {
                batch: e.batch,
                time: e.time.clone(),
                tag: e.tag.clone(),
                total: 0,
                remaining: Vec::new(),
                recycled: Vec::new(),
            });
            s.total += 1;
            if recycled {
                s.recycled.push(e);
            } else if !undone.contains(&(e.batch, path_key(&e.new))) {
                s.remaining.push(e);
            }
        }
        Ok(map.into_values().rev().collect())
    }

    /// Undo the newest batch that still has moves left to undo.
    pub fn undo_last(&self, roots: &Roots) -> io::Result<Option<BatchReport>> {
        self.undo_last_where(roots, |_| true)
    }

    /// Undo the newest batch with this tag that still has moves left.
    pub fn undo_last_tagged(&self, tag: &str, roots: &Roots) -> io::Result<Option<BatchReport>> {
        self.undo_last_where(roots, |b| b.tag == tag)
    }

    fn undo_last_where(
        &self,
        roots: &Roots,
        pick: impl Fn(&BatchSummary) -> bool,
    ) -> io::Result<Option<BatchReport>> {
        let last = self
            .history()?
            .into_iter()
            .find(|b| !b.remaining.is_empty() && pick(b));
        match last {
            Some(b) => self.undo(b.batch, roots).map(Some),
            None => Ok(None),
        }
    }

    /// Why a journal entry must not be undone (both paths are checked: no
    /// `..` or `.`, inside a current root and not the root itself, every
    /// folder on the way normal, never a protected or app-data place).
    fn entry_problem(&self, e: &Entry, roots: &Roots) -> Option<String> {
        let app_dir = self.path.parent();
        for p in [&e.new, &e.old] {
            if let Err(why) = journal_path_ok(roots, p, app_dir) {
                return Some(format!("journal entry looks wrong ({why})"));
            }
        }
        None
    }

    /// Move every file of `batch` back to where it was (renames get their
    /// old name back). A file is skipped (and reported) if it is gone, was
    /// changed or renamed again, or its old spot / old name is taken, and
    /// if the journal entry itself looks wrong (it is never trusted blindly).
    pub fn undo(&self, batch: u64, roots: &Roots) -> io::Result<BatchReport> {
        let _lock = self.lock()?;
        let summary = self.history()?.into_iter().find(|b| b.batch == batch);
        let mut report = BatchReport {
            batch,
            ..Default::default()
        };
        let Some(summary) = summary else {
            return Ok(report);
        };
        if let Err(err) = roots.validate() {
            report.errors.push((PathBuf::new(), err.to_string()));
            return Ok(report);
        }
        for e in summary.remaining.iter().rev() {
            if let Some(why) = self.entry_problem(e, roots) {
                report.skipped.push((e.new.clone(), why));
                continue;
            }
            let meta = match fs::symlink_metadata(&e.new) {
                Ok(m) if m.is_file() => m,
                _ => {
                    report.skipped.push((
                        e.new.clone(),
                        "file is no longer where it was moved to".into(),
                    ));
                    continue;
                }
            };
            let modified = meta.modified().map(to_ns).ok();
            if meta.len() != e.size || modified != Some(e.modified_ns) {
                report
                    .skipped
                    .push((e.new.clone(), "file was changed after it was moved".into()));
                continue;
            }
            if fs::symlink_metadata(&e.old).is_ok() {
                report.skipped.push((
                    e.new.clone(),
                    format!("original place is taken: {}", e.old.display()),
                ));
                continue;
            }
            if let Some(dir) = e.old.parent() {
                if !dir.is_dir() {
                    report.skipped.push((
                        e.new.clone(),
                        format!("original folder is gone: {}", dir.display()),
                    ));
                    continue;
                }
            }
            if crate::paths::is_self_exe(&e.new, meta.len()) {
                report.skipped.push((
                    e.new.clone(),
                    crate::safety::SkipReason::SelfApp.to_string(),
                ));
                continue;
            }
            // Every folder on the way must still be a normal folder.
            if let Err(why) = root_of(roots, &e.new).and_then(|_| root_of(roots, &e.old)) {
                report
                    .skipped
                    .push((e.new.clone(), format!("journal entry looks wrong ({why})")));
                continue;
            }
            if let Err(err) = move_no_replace(&e.new, &e.old) {
                report.errors.push((e.new.clone(), err.to_string()));
                continue;
            }
            let mut rec = e.clone();
            rec.time = Local::now().to_rfc3339();
            if let Err(err) = self.append(&Record::Undo(rec)) {
                report.errors.push((
                    e.old.clone(),
                    format!("moved back, but journal could not be written: {err}"),
                ));
            }
            if e.tag != TAG_RENAME {
                remove_empty_created_dirs(&e.new);
            }
            report.moved.push((e.new.clone(), e.old.clone()));
        }
        Ok(report)
    }
}

/// After undo, remove the `<Type>\<YYYY-MM>` (and `From Desktop`) folders
/// again if they are now EMPTY. `remove_dir` refuses non-empty folders, so
/// no file can be lost.
fn remove_empty_created_dirs(new_path: &Path) {
    let Some(month) = new_path.parent() else {
        return;
    };
    if !is_month_name(month) || fs::remove_dir(month).is_err() {
        return;
    }
    let Some(type_dir) = month.parent() else {
        return;
    };
    if fs::remove_dir(type_dir).is_err() {
        return;
    }
    if let Some(fd) = type_dir.parent() {
        if fd.file_name().is_some_and(|n| n == FROM_DESKTOP) {
            let _ = fs::remove_dir(fd);
        }
    }
}

fn is_month_name(p: &Path) -> bool {
    let n = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let b = n.as_bytes();
    b.len() == 7
        && b[4] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || c.is_ascii_digit())
}

fn to_ns(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos().min(i64::MAX as u128) as i64,
        Err(e) => -(e.duration().as_nanos().min(i64::MAX as u128) as i64),
    }
}
