//! Stage 4, part 4: extra folders.

use organizer_core::autosort::AutoSorter;
use organizer_core::bigold::{find_big_old, BigOldOptions};
use organizer_core::dupes::{find_duplicates, DupOptions};
use organizer_core::paths::{check_extra, MAX_EXTRA};
use organizer_core::plan::month_folder;
use organizer_core::rename::find_renames;
use organizer_core::{execute, scan, Journal, Progress, RootKind, Roots, ScanOptions, Settings};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

struct Env {
    _tmp: tempfile::TempDir,
    base: PathBuf,
    roots: Roots,
    journal: Journal,
    settings: Settings,
}

impl Env {
    fn new() -> Env {
        let parent = Path::new(env!("CARGO_TARGET_TMPDIR"));
        fs::create_dir_all(parent).unwrap();
        let tmp = tempfile::Builder::new().prefix("fo-extra").tempdir_in(parent).unwrap();
        let base = tmp.path().to_path_buf();
        let roots = Roots::new(base.join("Downloads"), base.join("Desktop"), base.join("Documents"));
        for k in roots.kinds() {
            fs::create_dir_all(roots.get(k)).unwrap();
        }
        let journal = Journal::new(base.join("appdata").join("journal.jsonl"));
        Env { _tmp: tmp, base, roots, journal, settings: Settings::default() }
    }

    /// Roots with the given extra folders (created first); all must pass.
    fn with(&self, names: &[&str]) -> Roots {
        let dirs: Vec<PathBuf> = names.iter().map(|n| self.base.join(n)).collect();
        for d in &dirs {
            fs::create_dir_all(d).unwrap();
        }
        let (r, warnings) = self.roots.clone().with_extra(&dirs);
        assert!(warnings.is_empty(), "{warnings:?}");
        r
    }

    fn opts(&self) -> ScanOptions {
        ScanOptions::new(&self.settings, SystemTime::now())
    }
}

fn days(n: u64) -> Duration {
    Duration::from_secs(n * 86_400)
}

fn file(dir: &Path, name: &str, content: &[u8], age: Duration) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    fs::OpenOptions::new().write(true).open(&p).unwrap().set_times(ft(SystemTime::now() - age)).unwrap();
    p
}

fn refused(roots: &Roots, p: &Path, says: &str) {
    match check_extra(roots, p) {
        Ok(()) => panic!("{} was accepted", p.display()),
        Err(e) => {
            assert!(e.to_lowercase().contains(&says.to_lowercase()), "{}: {e}", p.display());
            assert!(e.ends_with('.'), "one plain sentence: {e}");
        }
    }
    let mut list = roots.extra.clone();
    list.push(p.to_path_buf());
    let (r, warnings) = roots.clone().with_extra(&list);
    assert!(r.extra == roots.extra && warnings.len() == 1, "{}", p.display());
}

// ---------------------------------------------------------------- refusals

#[test]
fn refuses_relative_network_device_and_verbatim_paths() {
    let env = Env::new();
    refused(&env.roots, Path::new("Projects"), "not a full folder path");
    refused(&env.roots, &env.base.join("x").join("..").join("Projects"), "not a full folder path");
    refused(&env.roots, Path::new(r"\\server\share\Projects"), "network");
    refused(&env.roots, Path::new(r"\\.\C:\Projects"), "device");
    refused(&env.roots, Path::new(r"\\?\C:\Projects"), "network, device");
    refused(&env.roots, Path::new("//server/share"), "network");
}

#[test]
fn refuses_missing_folders_and_files() {
    let env = Env::new();
    refused(&env.roots, &env.base.join("nope"), "does not exist");
    let f = file(&env.base, "a.txt", b"a", days(1));
    refused(&env.roots, &f, "not a folder");
}

#[test]
fn refuses_links_and_junctions() {
    let env = Env::new();
    let real = env.base.join("real");
    fs::create_dir_all(&real).unwrap();
    let link = env.base.join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).unwrap();
    #[cfg(windows)]
    {
        let st = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&real).status().unwrap();
        assert!(st.success());
    }
    refused(&env.roots, &link, "link or junction");
    assert!(check_extra(&env.roots, &real).is_ok());
    #[cfg(windows)]
    fs::remove_dir(&link).unwrap();
}

#[test]
fn refuses_hidden_and_system_folders() {
    let env = Env::new();
    #[cfg(unix)]
    {
        let h = env.base.join(".hidden");
        fs::create_dir_all(&h).unwrap();
        refused(&env.roots, &h, "hidden or system");
    }
    #[cfg(windows)]
    for flag in ["+h", "+s"] {
        let d = env.base.join(format!("attr{}", &flag[1..]));
        fs::create_dir_all(&d).unwrap();
        assert!(std::process::Command::new("attrib").arg(flag).arg(&d).status().unwrap().success());
        refused(&env.roots, &d, "hidden or system");
        let minus = flag.replace('+', "-");
        assert!(std::process::Command::new("attrib").arg(minus).arg(&d).status().unwrap().success());
    }
}

#[test]
fn refuses_protected_places() {
    let env = Env::new();
    let home = organizer_core::paths::profile_dir().unwrap();
    refused(&env.roots, &home, "protected");
    #[cfg(windows)]
    {
        refused(&env.roots, Path::new(r"C:\"), "protected");
        refused(&env.roots, &organizer_core::paths::windows_dir().unwrap(), "protected");
        refused(&env.roots, Path::new(r"C:\Program Files"), "protected");
        refused(&env.roots, Path::new(r"C:\ProgramData"), "protected");
        let roaming = organizer_core::paths::app_data_dir().unwrap().parent().unwrap().to_path_buf();
        refused(&env.roots, &roaming, "protected");
    }
    #[cfg(unix)]
    refused(&env.roots, Path::new("/"), "protected");
}

#[test]
fn refuses_the_app_data_folder() {
    let env = Env::new();
    let app = organizer_core::paths::app_data_dir().unwrap();
    fs::create_dir_all(&app).unwrap();
    // On Windows it lies in AppData (protected); elsewhere the own rule says so.
    let e = check_extra(&env.roots, &app).unwrap_err();
    assert!(e.contains("protected") || e.contains("own settings"), "{e}");
}

#[test]
fn refuses_the_folder_the_app_runs_from() {
    let env = Env::new();
    let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    refused(&env.roots, &exe_dir, "runs from");
    // Its parent is fine (the exe is not directly in it).
    assert!(check_extra(&env.roots, exe_dir.parent().unwrap()).is_ok());
}

#[test]
fn refuses_overlapping_folders() {
    let env = Env::new();
    refused(&env.roots, &env.roots.downloads, "already in the list");
    let inside = env.roots.documents.join("Work");
    fs::create_dir_all(&inside).unwrap();
    refused(&env.roots, &inside, "inside Documents");
    refused(&env.roots, &env.base, "contains");
    let roots = env.with(&["Projects"]);
    refused(&roots, &env.base.join("Projects"), "already in the list (Projects)");
    let sub = env.base.join("Projects").join("Rust");
    fs::create_dir_all(&sub).unwrap();
    refused(&roots, &sub, "inside Projects");
    #[cfg(windows)]
    {
        // Same folder, other spelling: compared on resolved paths, ignoring case.
        let upper = PathBuf::from(env.base.join("PROJECTS").to_string_lossy().to_string());
        refused(&roots, &upper, "already in the list");
    }
}

#[test]
fn refuses_more_than_ten() {
    let env = Env::new();
    let names: Vec<String> = (0..MAX_EXTRA).map(|i| format!("x{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let roots = env.with(&refs);
    assert_eq!(roots.kinds().count(), 3 + MAX_EXTRA);
    let one_more = env.base.join("x10");
    fs::create_dir_all(&one_more).unwrap();
    refused(&roots, &one_more, "at most 10");
    let mut too_many = roots.clone();
    too_many.extra.push(one_more);
    assert!(too_many.validate().is_err());
}

#[test]
fn every_batch_rechecks_extra_folders() {
    let env = Env::new();
    let roots = env.with(&["Projects"]);
    assert!(roots.validate().is_ok());
    // Folder replaced by something that is no longer allowed.
    let mut bad = roots.clone();
    bad.extra.push(env.roots.documents.clone());
    assert!(bad.validate().is_err());
    assert!(scan(&bad, &env.settings, &env.opts()).is_err());
    let r = execute(&bad, &[], &env.journal, &env.opts());
    assert!(!r.errors.is_empty());
    fs::remove_dir(env.base.join("Projects")).unwrap();
    assert!(roots.validate().is_err(), "a folder that is gone is refused");
}

// ---------------------------------------------------------------- settings

#[test]
fn settings_round_trip_and_bad_entries_dropped() {
    let env = Env::new();
    let good = env.base.join("Projects");
    fs::create_dir_all(&good).unwrap();
    let path = env.base.join("appdata").join("settings.json");
    let bad = env.roots.downloads.clone();
    let missing = env.base.join("gone");
    Settings::update(&path, |s| s.extra_folders = vec![good.clone(), bad.clone(), missing.clone()]).unwrap();
    let (s, warn) = Settings::load_or_create(&path);
    assert!(warn.is_none());
    assert_eq!(s.extra_folders, vec![good.clone(), bad, missing]);
    let (roots, warnings) = env.roots.clone().with_extra(&s.extra_folders);
    assert_eq!(roots.extra, vec![good]);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings.iter().all(|w| w.starts_with("Extra folder left out")));
}

#[test]
fn settings_keep_at_most_ten_extra_folders() {
    let list: Vec<String> = (0..12).map(|i| format!("\"C:\\\\f{i}\"")).collect();
    let (s, warn) = Settings::from_json(&format!("{{\"extra_folders\": [{}]}}", list.join(",")));
    assert_eq!(s.extra_folders.len(), MAX_EXTRA);
    assert!(warn.is_some());
    let (s, _) = Settings::from_json("{\"extra_folders\": 5}");
    assert!(s.extra_folders.is_empty());
}

// ---------------------------------------------------------------- behaviour inside extra folders

#[test]
fn extra_folder_is_sorted_like_documents() {
    let env = Env::new();
    let roots = env.with(&["Projects"]);
    let dir = env.base.join("Projects");
    let old = file(&dir, "plan.pdf", b"p", days(30));
    let recent = file(&dir, "today.pdf", b"t", days(1));
    let inner = file(&dir.join("sub"), "deep.pdf", b"d", days(30));
    let plan = scan(&roots, &env.settings, &env.opts()).unwrap();
    let m = plan.moves.iter().find(|m| m.source == old).unwrap();
    assert_eq!(m.root, RootKind::Extra(0));
    assert_eq!(roots.label(m.root), "Projects");
    let month = month_folder(fs::metadata(&old).unwrap().modified().unwrap());
    let want = dir.join("Documents").join(&month).join("plan.pdf");
    assert_eq!(m.target, want);
    assert!(plan.moves.iter().all(|m| m.source != recent && m.source != inner));
    let r = execute(&roots, &plan.moves, &env.journal, &env.opts());
    assert!(r.errors.is_empty(), "{r:?}");
    assert!(want.is_file() && recent.is_file() && inner.is_file());
    // Undo works while the folder is in the list.
    let u = env.journal.undo_last(&roots).unwrap().unwrap();
    assert_eq!(u.moved.len(), 1);
    assert!(old.is_file());
}

#[test]
fn undo_refused_after_the_folder_was_removed_from_the_list() {
    let env = Env::new();
    let roots = env.with(&["Projects"]);
    let old = file(&env.base.join("Projects"), "plan.pdf", b"p", days(30));
    let plan = scan(&roots, &env.settings, &env.opts()).unwrap();
    assert_eq!(execute(&roots, &plan.moves, &env.journal, &env.opts()).moved.len(), 1);
    let u = env.journal.undo_last(&env.roots).unwrap().unwrap();
    assert!(u.moved.is_empty() && !old.exists());
    assert!(u.skipped[0].1.contains("add the folder back in Settings to undo this"), "{u:?}");
    // Back in the list: undo works again.
    assert_eq!(env.journal.undo_last(&roots).unwrap().unwrap().moved.len(), 1);
    assert!(old.is_file());
}

#[test]
fn duplicates_big_old_and_rename_include_extra_folders() {
    let mut env = Env::new();
    let roots = env.with(&["Projects"]);
    let dir = env.base.join("Projects");
    let data = vec![3u8; 4096];
    let a = file(&env.roots.downloads, "a.bin", &data, days(20));
    let b = file(&dir.join("deep"), "b.bin", &data, days(10));
    let dup =
        find_duplicates(&roots, &DupOptions::new(&env.settings, SystemTime::now()), &Progress::default()).unwrap();
    assert_eq!(dup.groups.len(), 1);
    let paths: Vec<_> = dup.groups[0].files.iter().map(|f| f.path.clone()).collect();
    assert_eq!(paths, vec![a, b.clone()]);
    assert_eq!(dup.groups[0].files[1].root, RootKind::Extra(0));

    env.settings.old_days = 5;
    let bo = find_big_old(&roots, &BigOldOptions::new(&env.settings, SystemTime::now()), &Progress::default()).unwrap();
    assert!(bo.old.iter().any(|f| f.path == b));

    let messy = file(&dir, "Copy of notes.txt", b"n", days(10));
    let r = find_renames(&roots, SystemTime::now(), &Progress::default()).unwrap();
    let item = r.items.iter().find(|i| i.file.path == messy).unwrap();
    let rep = organizer_core::rename::rename_files(&roots, std::slice::from_ref(item), &env.journal, SystemTime::now());
    assert_eq!(rep.moved.len(), 1, "{rep:?}");
    assert!(dir.join("notes.txt").is_file());
}

#[test]
fn background_sorting_never_touches_extra_folders() {
    let env = Env::new();
    let roots = env.with(&["Projects"]);
    let p = file(&env.base.join("Projects"), "song.mp3", b"m", days(30));
    let mut s = AutoSorter::new();
    for _ in 0..3 {
        assert!(s.tick(&roots, &env.settings, &env.journal, false, SystemTime::now()).is_none());
    }
    assert!(p.is_file());
}

/// Last-changed AND created time `t`. On Windows a file counts as new by the
/// newer of the two (a copied file keeps its old last-changed time).
fn ft(t: SystemTime) -> fs::FileTimes {
    let times = fs::FileTimes::new().set_modified(t);
    #[cfg(windows)]
    let times = {
        use std::os::windows::fs::FileTimesExt;
        times.set_created(t)
    };
    times
}
