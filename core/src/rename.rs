//! Stage 3: tidy up junk in file names. Only three patterns are touched:
//! stacked copy counters ("photo (1) (2).jpg"), "Copy of " / " - Copy", and
//! repeated or surrounding spaces. The extension never changes, files stay
//! in their folder, nothing is ever replaced and no numbers are invented:
//! if the new name is taken the file is simply left alone.

use crate::fsops::{move_no_replace, path_key};
use crate::journal::{BatchReport, Entry, Journal, Record, TAG_RENAME};
use crate::paths::Roots;
use crate::remove::{root_of, still_same};
use crate::tree::{walk, Progress, ScannedFile, Skipped};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Longest file name Windows accepts.
pub const MAX_NAME_LEN: usize = 255;

/// The cleaned-up name for `name`, or `None` if none of the three patterns
/// applies or the result would not be a valid, non-empty Windows name.
pub fn proposed_name(name: &str) -> Option<String> {
    let (stem, ext) = split_ext(name);
    let mut s = squeeze_spaces(stem);
    loop {
        let before = s.clone();
        if let Some(rest) = strip_prefix_ci(&s, "copy of ") {
            s = rest.to_string();
        }
        if let Some(rest) = strip_suffix_ci(&s, " - copy") {
            s = rest.to_string();
        }
        s = strip_stacked_counters(&s);
        s = squeeze_spaces(&s);
        if s == before {
            break;
        }
    }
    if s == stem {
        return None;
    }
    let new = match ext {
        Some(ext) => format!("{s}.{ext}"),
        None => s,
    };
    (is_valid_windows_name(&new) && !new.eq_ignore_ascii_case(name) && split_ext(&new).1 == ext)
        .then_some(new)
}

/// ("name", Some("ext")) — the extension is what follows the LAST dot, and
/// only if something comes before that dot.
fn split_ext(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], Some(&name[i + 1..])),
        _ => (name, None),
    }
}

/// Runs of spaces become one space; spaces at both ends are removed.
fn squeeze_spaces(s: &str) -> String {
    s.split(' ')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = s.len().checked_sub(suffix.len())?;
    let tail = s.get(cut..)?;
    tail.eq_ignore_ascii_case(suffix).then(|| &s[..cut])
}

/// Removes trailing " (n)" counters, but only when there are two or more
/// of them in a row ("photo (1) (2)" -> "photo"; "photo (1)" stays).
fn strip_stacked_counters(s: &str) -> String {
    let mut rest = s;
    let mut count = 0;
    while let Some(r) = strip_one_counter(rest) {
        rest = r;
        count += 1;
    }
    if count >= 2 {
        rest.to_string()
    } else {
        s.to_string()
    }
}

/// "name (12)" -> "name"; `None` if `s` does not end in " (digits)".
fn strip_one_counter(s: &str) -> Option<&str> {
    let inner = s.strip_suffix(')')?;
    let open = inner.rfind(" (")?;
    let digits = &inner[open + 2..];
    let ok = !digits.is_empty() && digits.len() <= 9 && digits.bytes().all(|b| b.is_ascii_digit());
    (ok && open > 0).then(|| &s[..open])
}

/// True if Windows accepts `name` as a file name: not empty, no forbidden
/// characters, not ending in a space or dot, not a reserved device name.
pub fn is_valid_windows_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN || name.ends_with(' ') || name.ends_with('.') {
        return false;
    }
    if name.chars().any(|c| {
        matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control()
    }) {
        return false;
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit()
            && base.as_bytes()[3] != b'0');
    !reserved && !split_ext(name).0.is_empty()
}

/// One proposed rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedRename {
    pub file: ScannedFile,
    pub new_path: PathBuf,
}

impl ProposedRename {
    pub fn old_name(&self) -> String {
        name_of(&self.file.path)
    }
    pub fn new_name(&self) -> String {
        name_of(&self.new_path)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenameScan {
    pub items: Vec<ProposedRename>,
    /// Files whose name could be tidied but are left alone (and why).
    pub conflicts: Vec<(PathBuf, String)>,
    pub skipped: Skipped,
    pub files_checked: u64,
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Lower-cased names of everything in `dir` (files and folders).
fn names_in(dir: &Path) -> io::Result<HashSet<String>> {
    let mut set = HashSet::new();
    for e in fs::read_dir(dir)? {
        set.insert(e?.file_name().to_string_lossy().to_lowercase());
    }
    Ok(set)
}

/// True if `dir` already holds `name` (ignoring case, like Windows).
fn name_taken(dir: &Path, name: &str) -> io::Result<bool> {
    if dir.join(name).symlink_metadata().is_ok() {
        return Ok(true);
    }
    Ok(names_in(dir)?.contains(&name.to_lowercase()))
}

/// Walks the three roots with all sub-folders (same skip rules as stage 2)
/// and lists the renames. Nothing on disk is changed.
pub fn find_renames(roots: &Roots, now: SystemTime, progress: &Progress) -> io::Result<RenameScan> {
    let mut scan = RenameScan::default();
    let mut found: Vec<(ScannedFile, String)> = Vec::new();
    walk(roots, now, progress, &mut scan.skipped, |f| {
        if let Some(new) = proposed_name(&name_of(&f.path)) {
            found.push((f, new));
        }
    })?;
    scan.files_checked = progress.files.load(std::sync::atomic::Ordering::Relaxed);
    // Two proposals with the same target: neither is done.
    let mut targets: HashMap<String, usize> = HashMap::new();
    for (f, new) in &found {
        let dir = f.path.parent().unwrap_or(Path::new(""));
        *targets.entry(path_key(&dir.join(new))).or_default() += 1;
    }
    let mut dir_names: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    for (f, new) in found {
        let Some(dir) = f.path.parent().map(Path::to_path_buf) else {
            continue;
        };
        let new_path = dir.join(&new);
        if targets.get(&path_key(&new_path)).copied().unwrap_or(0) > 1 {
            scan.conflicts
                .push((f.path, format!("another file would also become \"{new}\"")));
            continue;
        }
        let names = match dir_names.get(&dir) {
            Some(n) => n,
            None => match names_in(&dir) {
                Ok(n) => dir_names.entry(dir.clone()).or_insert(n),
                Err(e) => {
                    scan.conflicts
                        .push((f.path, format!("folder cannot be read: {e}")));
                    continue;
                }
            },
        };
        if names.contains(&new.to_lowercase()) {
            scan.conflicts
                .push((f.path, format!("\"{new}\" already exists")));
            continue;
        }
        scan.items.push(ProposedRename { file: f, new_path });
    }
    Ok(scan)
}

/// Renames the files the user ticked. Each one is checked again right
/// before (still there, same size and last-changed time, not locked, new
/// name still free and still exactly what the rules give). Never replaces.
pub fn rename_files(
    roots: &Roots,
    items: &[ProposedRename],
    journal: &Journal,
    now: SystemTime,
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
    for item in items {
        let src = &item.file.path;
        if let Err(e) = rename_one(roots, item, journal, now, &mut report) {
            report.errors.push((src.clone(), e));
        }
    }
    report
}

fn rename_one(
    roots: &Roots,
    item: &ProposedRename,
    journal: &Journal,
    now: SystemTime,
    report: &mut BatchReport,
) -> Result<(), String> {
    let src = &item.file.path;
    root_of(roots, src)?;
    let dir = src.parent().ok_or("file has no folder")?;
    if item.new_path.parent() != Some(dir) {
        return Err("new name must stay in the same folder".into());
    }
    let new_name = item.new_name();
    if proposed_name(&item.old_name()).as_deref() != Some(new_name.as_str()) {
        return Err("not a rename these rules would make".into());
    }
    if !still_same(&item.file, now, report) {
        return Ok(());
    }
    match name_taken(dir, &new_name) {
        Ok(false) => {}
        Ok(true) => {
            report
                .skipped
                .push((src.clone(), format!("\"{new_name}\" already exists")));
            return Ok(());
        }
        Err(e) => return Err(format!("folder cannot be read: {e}")),
    }
    move_no_replace(src, &item.new_path).map_err(|e| e.to_string())?;
    let entry = Entry::new(
        report.batch,
        src,
        &item.new_path,
        item.file.size,
        item.file.modified,
    )
    .tagged(TAG_RENAME);
    if let Err(e) = journal.append(&Record::Move(entry)) {
        // Without a journal line the rename could not be undone: put it back.
        return Err(match move_no_replace(&item.new_path, src) {
            Ok(()) => format!("journal could not be written ({e}); name left as it was"),
            Err(e2) => format!(
                "journal could not be written ({e}); file is now {} ({e2})",
                item.new_path.display()
            ),
        });
    }
    report.moved.push((src.clone(), item.new_path.clone()));
    Ok(())
}
