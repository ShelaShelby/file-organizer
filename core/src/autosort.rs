//! Stage 3: background sorting of Downloads (`--tray`). Uses the stage 1
//! sort engine (same rules, `<Type>\<YYYY-MM>` layout and journal); entries
//! are tagged "auto". A file is only moved when it is clearly finished:
//! last changed more than 60 s ago (and the stage 1 two-minute rule), same
//! size and last-changed time at two checks in a row, not a temp /
//! still-downloading file, and it can be opened with no sharing violation.

use crate::config::Settings;
use crate::fsops::path_key;
use crate::journal::{BatchReport, Journal, TAG_AUTO};
use crate::paths::{RootKind, Roots};
use crate::plan::{execute_tagged, scan_root, Plan, PlannedMove, ScanOptions};
use crate::safety::TEMP_EXTENSIONS;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Files changed more recently than this are never picked up.
pub const SETTLE_TIME: Duration = Duration::from_secs(60);
/// How often the background sorter looks at Downloads.
pub const CHECK_EVERY: Duration = Duration::from_secs(15);
/// Endings that always mean "still downloading" (checked on top of the
/// stage 1 temp list).
pub const DOWNLOADING_EXTENSIONS: &[&str] = &["crdownload", "part", "partial", "tmp", "download"];

/// Remembers what each file looked like at the previous check.
#[derive(Debug, Default)]
pub struct AutoSorter {
    seen: HashMap<String, (u64, SystemTime)>,
}

fn is_downloading_name(p: &Path) -> bool {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    DOWNLOADING_EXTENSIONS.contains(&ext.as_str()) || TEMP_EXTENSIONS.contains(&ext.as_str())
}

fn settled(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified).is_ok_and(|age| age >= SETTLE_TIME)
}

impl AutoSorter {
    pub fn new() -> AutoSorter {
        AutoSorter::default()
    }

    /// One check: returns the Downloads files that may be moved now. Uses
    /// the stage 1 scan (skip rules, lock check, two-minute rule) and adds
    /// the background rules. Remembers sizes for the next check.
    pub fn eligible(&mut self, roots: &Roots, settings: &Settings, now: SystemTime) -> Vec<PlannedMove> {
        let opts = ScanOptions::new(settings, now);
        let mut plan = Plan::default();
        scan_root(roots, settings, &opts, RootKind::Downloads, &mut HashSet::new(), &mut plan);
        let mut seen = HashMap::new();
        let mut out = Vec::new();
        for m in plan.moves {
            let key = path_key(&m.source);
            let now_state = (m.size, m.modified);
            let stable = self.seen.get(&key) == Some(&now_state);
            seen.insert(key, now_state);
            if stable && settled(m.modified, now) && !is_downloading_name(&m.source) {
                out.push(m);
            }
        }
        // Files skipped by the scan (too new, locked, ...) start over.
        self.seen = seen;
        out
    }

    /// One timer tick: nothing happens while paused; otherwise eligible
    /// files are moved as one "auto" batch. `None` if nothing was moved.
    pub fn tick(
        &mut self,
        roots: &Roots,
        settings: &Settings,
        journal: &Journal,
        paused: bool,
        now: SystemTime,
    ) -> Option<BatchReport> {
        if paused {
            self.seen.clear();
            return None;
        }
        let moves = self.eligible(roots, settings, now);
        if moves.is_empty() {
            return None;
        }
        let report = execute_tagged(roots, &moves, journal, &ScanOptions::new(settings, now), TAG_AUTO);
        (!report.moved.is_empty() || !report.errors.is_empty()).then_some(report)
    }
}

/// Text for the small note after a batch, e.g.
/// "Moved 3 files to Downloads\Images\2026-10".
pub fn note_text(roots: &Roots, moved: &[(std::path::PathBuf, std::path::PathBuf)]) -> String {
    let mut dirs: Vec<String> = Vec::new();
    for (_, new) in moved {
        let dir = new.parent().unwrap_or(new);
        let label = match dir.strip_prefix(&roots.downloads) {
            Ok(rest) => Path::new("Downloads").join(rest).display().to_string(),
            Err(_) => dir.display().to_string(),
        };
        if !dirs.contains(&label) {
            dirs.push(label);
        }
    }
    let files = if moved.len() == 1 { "1 file".to_string() } else { format!("{} files", moved.len()) };
    match dirs.len() {
        0 => format!("Moved {files}"),
        1 => format!("Moved {files} to {}", dirs[0]),
        n => format!("Moved {files} to {} and {} other folder{}", dirs[0], n - 1, if n == 2 { "" } else { "s" }),
    }
}
