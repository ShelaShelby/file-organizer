//! Tests for every safety rule, using temporary folders.

use organizer_core::config::Settings;
use organizer_core::fsops::numbered_name;
use organizer_core::paths::{is_protected, FROM_DESKTOP};
use organizer_core::plan::month_folder;
use organizer_core::safety::{Check, SkipReason};
use organizer_core::{execute, scan, Journal, PlannedMove, RootKind, Roots, ScanOptions};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const DAY: u64 = 86_400;

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
        let tmp = tempfile::Builder::new().prefix("fo-test").tempdir_in(parent).unwrap();
        let base = tmp.path().to_path_buf();
        let roots = Roots {
            downloads: base.join("Downloads"),
            desktop: base.join("Desktop"),
            documents: base.join("Documents"),
        };
        for k in RootKind::ALL {
            fs::create_dir_all(roots.get(k)).unwrap();
        }
        let journal = Journal::new(base.join("appdata").join("journal.jsonl"));
        Env { _tmp: tmp, base, roots, journal, settings: Settings::default() }
    }

    fn opts(&self) -> ScanOptions {
        ScanOptions::new(&self.settings, SystemTime::now())
    }

    fn scan(&self) -> organizer_core::Plan {
        scan(&self.roots, &self.settings, &self.opts()).unwrap()
    }

    fn run_all(&self) -> organizer_core::BatchReport {
        let plan = self.scan();
        execute(&self.roots, &plan.moves, &self.journal, &self.opts())
    }
}

/// Create a file whose last-changed time is `age` ago.
fn file(dir: &Path, name: &str, content: &str, age: Duration) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    set_age(&p, age);
    p
}

fn set_age(p: &Path, age: Duration) {
    let f = fs::OpenOptions::new().write(true).open(p).unwrap();
    f.set_modified(SystemTime::now() - age).unwrap();
}

fn days(n: u64) -> Duration {
    Duration::from_secs(n * DAY)
}

fn mtime(p: &Path) -> SystemTime {
    fs::metadata(p).unwrap().modified().unwrap()
}

fn planned_sources(plan: &organizer_core::Plan) -> Vec<String> {
    plan.moves.iter().map(|m| m.source.file_name().unwrap().to_string_lossy().to_string()).collect()
}

fn skip_reason(plan: &organizer_core::Plan, name: &str) -> Option<SkipReason> {
    plan.skipped.iter().find(|(p, _)| p.file_name().unwrap() == name).map(|(_, r)| r.clone())
}

// ---------------------------------------------------------------- layout

#[test]
fn downloads_sorted_by_type_then_month() {
    let env = Env::new();
    let src = file(&env.roots.downloads, "photo.JPG", "img", days(30));
    let month = month_folder(mtime(&src));
    let report = env.run_all();
    assert_eq!(report.moved.len(), 1, "{report:?}");
    let want = env.roots.downloads.join("Images").join(&month).join("photo.JPG");
    assert!(want.is_file());
    assert!(!src.exists());
    assert_eq!(fs::read_to_string(want).unwrap(), "img");
}

#[test]
fn documents_sorted_inside_documents() {
    let env = Env::new();
    let src = file(&env.roots.documents, "notes.txt", "n", days(30));
    let month = month_folder(mtime(&src));
    env.run_all();
    assert!(env.roots.documents.join("Documents").join(month).join("notes.txt").is_file());
}

#[test]
fn desktop_goes_to_documents_from_desktop() {
    let env = Env::new();
    let src = file(&env.roots.desktop, "report.pdf", "r", days(30));
    let month = month_folder(mtime(&src));
    let plan = env.scan();
    let want = env.roots.documents.join(FROM_DESKTOP).join("Documents").join(&month).join("report.pdf");
    assert_eq!(plan.moves[0].target, want);
    let report = execute(&env.roots, &plan.moves, &env.journal, &env.opts());
    assert_eq!(report.moved.len(), 1);
    assert!(want.is_file());
    assert!(!src.exists());
    // Nothing is created inside Desktop itself.
    assert_eq!(fs::read_dir(&env.roots.desktop).unwrap().count(), 0);
}

#[test]
fn every_type_folder_and_other() {
    let env = Env::new();
    let cases = [
        ("a.png", "Images"),
        ("b.mkv", "Videos"),
        ("c.flac", "Music"),
        ("d.xlsx", "Documents"),
        ("e.7z", "Archives"),
        ("f.msi", "Installers"),
        ("g.xyz", "Other"),
        ("noext", "Other"),
    ];
    for (n, _) in cases {
        file(&env.roots.downloads, n, "x", days(1));
    }
    let plan = env.scan();
    for (n, cat) in cases {
        let m = plan.moves.iter().find(|m| m.source.file_name().unwrap() == n).unwrap();
        assert_eq!(m.category, cat, "{n}");
        assert!(m.target.starts_with(env.roots.downloads.join(cat)));
    }
}

#[test]
fn month_folder_uses_last_changed_date() {
    use chrono::{Local, TimeZone};
    let env = Env::new();
    let p = file(&env.roots.downloads, "old.zip", "z", days(1));
    let t: SystemTime = Local.with_ymd_and_hms(2024, 3, 15, 12, 0, 0).unwrap().into();
    fs::OpenOptions::new().write(true).open(&p).unwrap().set_modified(t).unwrap();
    let plan = env.scan();
    assert_eq!(plan.moves[0].target, env.roots.downloads.join("Archives").join("2024-03").join("old.zip"));
}

// ---------------------------------------------------------------- age rules

#[test]
fn age_rule_desktop_and_documents_seven_days() {
    let env = Env::new();
    file(&env.roots.desktop, "new_desk.txt", "x", days(3));
    file(&env.roots.desktop, "old_desk.txt", "x", days(8));
    file(&env.roots.documents, "new_doc.txt", "x", days(6));
    file(&env.roots.documents, "old_doc.txt", "x", days(10));
    let plan = env.scan();
    let s = planned_sources(&plan);
    assert!(s.contains(&"old_desk.txt".into()) && s.contains(&"old_doc.txt".into()));
    assert!(!s.contains(&"new_desk.txt".into()) && !s.contains(&"new_doc.txt".into()));
    assert_eq!(skip_reason(&plan, "new_desk.txt"), Some(SkipReason::Recent));
}

#[test]
fn age_rule_uses_setting() {
    let mut env = Env::new();
    env.settings.recent_days = 2;
    file(&env.roots.desktop, "three_days.txt", "x", days(3));
    assert_eq!(planned_sources(&env.scan()), vec!["three_days.txt".to_string()]);
}

#[test]
fn age_rule_downloads_two_minutes() {
    let env = Env::new();
    file(&env.roots.downloads, "just_now.pdf", "x", Duration::from_secs(30));
    file(&env.roots.downloads, "five_min.pdf", "x", Duration::from_secs(300));
    let plan = env.scan();
    assert_eq!(planned_sources(&plan), vec!["five_min.pdf".to_string()]);
    assert_eq!(skip_reason(&plan, "just_now.pdf"), Some(SkipReason::Recent));
}

#[test]
fn future_dates_are_treated_as_recent() {
    let env = Env::new();
    let p = file(&env.roots.downloads, "future.txt", "x", days(0));
    fs::OpenOptions::new().write(true).open(&p).unwrap().set_modified(SystemTime::now() + days(5)).unwrap();
    assert!(env.scan().moves.is_empty());
}

// ---------------------------------------------------------------- never touch

#[test]
fn skips_temp_downloading_shortcut_and_desktop_ini() {
    let env = Env::new();
    let names = [
        "movie.mp4.crdownload",
        "big.iso.part",
        "x.partial",
        "y.tmp",
        "~$report.docx",
        "Chrome.lnk",
        "site.url",
        "desktop.ini",
        "Desktop.INI",
    ];
    for n in names {
        file(&env.roots.downloads, n, "x", days(30));
    }
    let report = env.run_all();
    assert!(report.moved.is_empty(), "{report:?}");
    for n in names {
        assert!(env.roots.downloads.join(n).is_file(), "{n} must stay");
    }
    let plan = env.scan();
    assert_eq!(skip_reason(&plan, "Chrome.lnk"), Some(SkipReason::Shortcut));
    assert_eq!(skip_reason(&plan, "y.tmp"), Some(SkipReason::Temporary));
    assert_eq!(skip_reason(&plan, "~$report.docx"), Some(SkipReason::Temporary));
}

#[test]
fn folders_and_their_contents_are_never_touched() {
    let env = Env::new();
    let sub = env.roots.documents.join("My Project");
    fs::create_dir_all(sub.join("deeper")).unwrap();
    let inner = file(&sub, "inside.pdf", "x", days(100));
    let deep = file(&sub.join("deeper"), "deep.jpg", "x", days(100));
    let report = env.run_all();
    assert!(report.moved.is_empty());
    assert!(inner.is_file() && deep.is_file());
}

#[test]
fn only_loose_files_are_accepted_by_execute() {
    let env = Env::new();
    let sub = env.roots.downloads.join("folder");
    fs::create_dir_all(&sub).unwrap();
    let inner = file(&sub, "x.jpg", "x", days(30));
    let fake = PlannedMove {
        root: RootKind::Downloads,
        source: inner.clone(),
        target: env.roots.downloads.join("Images").join("2020-01").join("x.jpg"),
        category: "Images".into(),
        size: 1,
        modified: mtime(&inner),
    };
    let report = execute(&env.roots, &[fake], &env.journal, &env.opts());
    assert_eq!(report.errors.len(), 1);
    assert!(inner.is_file());
}

#[test]
fn execute_refuses_target_outside_root() {
    let env = Env::new();
    let src = file(&env.roots.downloads, "x.jpg", "x", days(30));
    let fake = PlannedMove {
        root: RootKind::Downloads,
        source: src.clone(),
        target: env.base.join("elsewhere").join("x.jpg"),
        category: "Images".into(),
        size: 1,
        modified: mtime(&src),
    };
    let report = execute(&env.roots, &[fake], &env.journal, &env.opts());
    assert_eq!(report.errors.len(), 1);
    assert!(src.is_file());
}

#[cfg(unix)]
#[test]
fn hidden_files_skipped_unix() {
    let env = Env::new();
    file(&env.roots.downloads, ".secret.txt", "x", days(30));
    let plan = env.scan();
    assert!(plan.moves.is_empty());
    assert_eq!(skip_reason(&plan, ".secret.txt"), Some(SkipReason::Hidden));
}

#[cfg(windows)]
fn attrib(flag: &str, p: &Path) {
    let st = std::process::Command::new("attrib").arg(flag).arg(p).status().unwrap();
    assert!(st.success());
}

#[cfg(windows)]
#[test]
fn hidden_and_system_files_skipped_windows() {
    let env = Env::new();
    let h = file(&env.roots.downloads, "hidden.txt", "x", days(30));
    let s = file(&env.roots.downloads, "system.txt", "x", days(30));
    attrib("+h", &h);
    attrib("+s", &s);
    let report = env.run_all();
    assert!(report.moved.is_empty(), "{report:?}");
    let plan = env.scan();
    assert_eq!(skip_reason(&plan, "hidden.txt"), Some(SkipReason::Hidden));
    assert_eq!(skip_reason(&plan, "system.txt"), Some(SkipReason::System));
    attrib("-h", &h);
    attrib("-s", &s);
}

#[cfg(windows)]
#[test]
fn locked_files_skipped_windows() {
    let env = Env::new();
    let p = file(&env.roots.downloads, "open.docx", "x", days(30));
    let handle = fs::File::open(&p).unwrap();
    let plan = env.scan();
    assert!(plan.moves.is_empty());
    assert!(matches!(skip_reason(&plan, "open.docx"), Some(SkipReason::InUse(_))));
    drop(handle);
    assert_eq!(env.scan().moves.len(), 1);
}

#[cfg(windows)]
#[test]
fn file_locked_after_scan_is_skipped_on_confirm() {
    let env = Env::new();
    let p = file(&env.roots.downloads, "busy.pdf", "x", days(30));
    file(&env.roots.downloads, "free.pdf", "x", days(30));
    let plan = env.scan();
    assert_eq!(plan.moves.len(), 2);
    let handle = fs::File::open(&p).unwrap();
    let report = execute(&env.roots, &plan.moves, &env.journal, &env.opts());
    drop(handle);
    assert_eq!(report.moved.len(), 1);
    assert_eq!(report.skipped.len(), 1);
    assert!(p.is_file());
}

#[cfg(windows)]
#[test]
fn junctions_are_skipped_and_untouched_windows() {
    let env = Env::new();
    let real = env.base.join("real");
    fs::create_dir_all(&real).unwrap();
    let inside = file(&real, "keep.jpg", "x", days(30));
    let j = env.roots.documents.join("My Pictures");
    let st = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&j).arg(&real).status().unwrap();
    assert!(st.success());
    let report = env.run_all();
    assert!(report.moved.is_empty());
    assert!(inside.is_file());
    let plan = env.scan();
    assert_eq!(skip_reason(&plan, "My Pictures"), Some(SkipReason::Link));
    fs::remove_dir(&j).unwrap(); // removes only the junction itself
    assert!(inside.is_file());
}

#[test]
fn symlinked_files_are_skipped() {
    let env = Env::new();
    let real = file(&env.base, "real.jpg", "x", days(30));
    let link = env.roots.downloads.join("link.jpg");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&real, &link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(&real, &link);
    if made.is_err() {
        eprintln!("symlinks not permitted here; skipping");
        return;
    }
    let report = env.run_all();
    assert!(report.moved.is_empty());
    assert!(link.symlink_metadata().is_ok() && real.is_file());
    assert_eq!(skip_reason(&env.scan(), "link.jpg"), Some(SkipReason::Link));
}

#[test]
fn protected_locations_are_refused() {
    assert!(is_protected(Path::new("/")));
    let env = Env::new();
    assert!(!is_protected(&env.roots.downloads));
    #[cfg(windows)]
    {
        for p in [
            r"C:\",
            r"C:\Windows",
            r"c:\windows\System32",
            r"C:\Program Files\App",
            r"C:\Program Files (x86)",
            r"C:\ProgramData\x",
            r"C:\Users\someone\AppData\Roaming",
        ] {
            assert!(is_protected(Path::new(p)), "{p}");
        }
        let appdata = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("fo-test-root");
        let roots = Roots { downloads: appdata, ..env.roots.clone() };
        assert!(scan(&roots, &env.settings, &env.opts()).is_err());
        assert!(!execute(&roots, &[], &env.journal, &env.opts()).errors.is_empty());
    }
    let mut roots = env.roots.clone();
    roots.desktop = PathBuf::from("relative");
    assert!(scan(&roots, &env.settings, &env.opts()).is_err());
}

// ---------------------------------------------------------------- no overwrite

#[test]
fn numbered_names() {
    assert_eq!(numbered_name("photo.jpg", 1), "photo.jpg");
    assert_eq!(numbered_name("photo.jpg", 2), "photo (2).jpg");
    assert_eq!(numbered_name("README", 3), "README (3)");
    assert_eq!(numbered_name(".env", 2), ".env (2)");
}

#[test]
fn name_collision_in_scan_and_on_disk() {
    let env = Env::new();
    let a = file(&env.roots.downloads, "photo.jpg", "new", days(30));
    let month = month_folder(mtime(&a));
    let dir = env.roots.downloads.join("Images").join(&month);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("photo.jpg"), "existing").unwrap();
    let plan = env.scan();
    assert_eq!(plan.moves[0].target, dir.join("photo (2).jpg"));
    let report = execute(&env.roots, &plan.moves, &env.journal, &env.opts());
    assert_eq!(report.moved.len(), 1);
    assert_eq!(fs::read_to_string(dir.join("photo.jpg")).unwrap(), "existing");
    assert_eq!(fs::read_to_string(dir.join("photo (2).jpg")).unwrap(), "new");
}

#[test]
fn never_overwrites_file_created_after_scan() {
    let env = Env::new();
    file(&env.roots.downloads, "song.mp3", "mine", days(30));
    let plan = env.scan();
    let target = plan.moves[0].target.clone();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "appeared later").unwrap();
    let report = execute(&env.roots, &plan.moves, &env.journal, &env.opts());
    assert_eq!(report.moved.len(), 1);
    assert_eq!(fs::read_to_string(&target).unwrap(), "appeared later");
    let new = &report.moved[0].1;
    assert_eq!(new.file_name().unwrap(), "song (2).mp3");
    assert_eq!(fs::read_to_string(new).unwrap(), "mine");
}

#[test]
fn move_no_replace_refuses_existing_target() {
    let env = Env::new();
    let a = file(&env.base, "a.txt", "A", days(1));
    let b = file(&env.base, "b.txt", "B", days(1));
    assert!(organizer_core::fsops::move_no_replace(&a, &b).is_err());
    assert_eq!(fs::read_to_string(&a).unwrap(), "A");
    assert_eq!(fs::read_to_string(&b).unwrap(), "B");
}

// ---------------------------------------------------------------- batch errors

#[test]
fn one_failure_does_not_stop_the_batch() {
    let env = Env::new();
    let gone = file(&env.roots.downloads, "gone.txt", "x", days(30));
    file(&env.roots.downloads, "stays.txt", "x", days(30));
    file(&env.roots.downloads, "zzz.txt", "x", days(30));
    let plan = env.scan();
    fs::remove_file(&gone).unwrap(); // vanished between scan and confirm
    let report = execute(&env.roots, &plan.moves, &env.journal, &env.opts());
    assert_eq!(report.moved.len(), 2);
    assert_eq!(report.skipped.len() + report.errors.len(), 1);
}

#[test]
fn file_changed_after_scan_is_skipped() {
    let env = Env::new();
    let p = file(&env.roots.downloads, "edit.txt", "x", days(30));
    let plan = env.scan();
    fs::write(&p, "changed!").unwrap();
    set_age(&p, days(30));
    let report = execute(&env.roots, &plan.moves, &env.journal, &env.opts());
    assert!(report.moved.is_empty());
    assert_eq!(report.skipped[0].1, SkipReason::Changed.to_string());
    assert!(p.is_file());
}

#[test]
fn unchecked_rows_are_not_moved() {
    let env = Env::new();
    let a = file(&env.roots.downloads, "a.txt", "x", days(30));
    let b = file(&env.roots.downloads, "b.txt", "x", days(30));
    let plan = env.scan();
    let chosen: Vec<_> = plan.moves.iter().filter(|m| m.source == a).cloned().collect();
    execute(&env.roots, &chosen, &env.journal, &env.opts());
    assert!(!a.exists() && b.is_file());
}

// ---------------------------------------------------------------- journal + undo

#[test]
fn journal_records_old_new_and_time() {
    let env = Env::new();
    let src = file(&env.roots.downloads, "a.pdf", "x", days(30));
    let report = env.run_all();
    let text = fs::read_to_string(env.journal.path()).unwrap();
    let line: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(line["kind"], "move");
    assert_eq!(PathBuf::from(line["old"].as_str().unwrap()), src);
    assert_eq!(PathBuf::from(line["new"].as_str().unwrap()), report.moved[0].1);
    assert!(line["time"].as_str().unwrap().len() > 10);
}

#[test]
fn undo_restores_everything_exactly() {
    let env = Env::new();
    let files = [
        file(&env.roots.downloads, "a.jpg", "A", days(30)),
        file(&env.roots.desktop, "b.pdf", "B", days(30)),
        file(&env.roots.documents, "c.mp3", "C", days(40)),
    ];
    let before: Vec<_> = files.iter().map(|p| (fs::read(p).unwrap(), mtime(p))).collect();
    let report = env.run_all();
    assert_eq!(report.moved.len(), 3);
    assert!(files.iter().all(|p| !p.exists()));

    let undo = env.journal.undo_last(&env.roots).unwrap().unwrap();
    assert_eq!(undo.moved.len(), 3, "{undo:?}");
    assert!(undo.skipped.is_empty() && undo.errors.is_empty());
    for (p, (content, t)) in files.iter().zip(before) {
        assert_eq!(fs::read(p).unwrap(), content);
        assert_eq!(mtime(p), t);
    }
    // Created (now empty) folders are gone again; roots look as before.
    assert!(!env.roots.downloads.join("Images").exists());
    assert!(!env.roots.documents.join(FROM_DESKTOP).exists());
    assert!(!env.roots.documents.join("Music").exists());
    assert!(env.journal.history().unwrap()[0].remaining.is_empty());
    assert!(env.journal.undo_last(&env.roots).unwrap().is_none());
}

#[test]
fn undo_skips_changed_file_and_taken_spot() {
    let env = Env::new();
    let a = file(&env.roots.downloads, "a.txt", "A", days(30));
    let b = file(&env.roots.downloads, "b.txt", "B", days(30));
    let c = file(&env.roots.downloads, "c.txt", "C", days(30));
    let report = env.run_all();
    let new_of = |p: &Path| report.moved.iter().find(|(o, _)| o == p).unwrap().1.clone();
    fs::write(new_of(&a), "edited after move").unwrap();
    fs::write(&b, "someone put a new file here").unwrap();

    let undo = env.journal.undo_last(&env.roots).unwrap().unwrap();
    assert_eq!(undo.moved.len(), 1);
    assert_eq!(undo.skipped.len(), 2);
    assert_eq!(fs::read_to_string(new_of(&a)).unwrap(), "edited after move");
    assert_eq!(fs::read_to_string(&b).unwrap(), "someone put a new file here");
    assert_eq!(fs::read_to_string(new_of(&b)).unwrap(), "B");
    assert_eq!(fs::read_to_string(&c).unwrap(), "C");
    assert_eq!(env.journal.history().unwrap()[0].remaining.len(), 2);
}

#[test]
fn undo_older_batch_from_history() {
    let env = Env::new();
    let first = file(&env.roots.downloads, "first.txt", "1", days(30));
    env.run_all();
    let second = file(&env.roots.downloads, "second.txt", "2", days(30));
    env.run_all();
    let hist = env.journal.history().unwrap();
    assert_eq!(hist.len(), 2);
    assert!(hist[0].batch > hist[1].batch, "newest first");
    let older = hist[1].batch;
    let undo = env.journal.undo(older, &env.roots).unwrap();
    assert_eq!(undo.moved.len(), 1);
    assert!(first.is_file());
    assert!(!second.exists());
    // Undoing the same batch again does nothing.
    assert!(env.journal.undo(older, &env.roots).unwrap().moved.is_empty());
}

#[test]
fn undo_skips_file_that_is_gone() {
    let env = Env::new();
    file(&env.roots.downloads, "a.txt", "A", days(30));
    let report = env.run_all();
    let new = report.moved[0].1.clone();
    let elsewhere = env.base.join("moved-by-user.txt");
    fs::rename(&new, &elsewhere).unwrap();
    let undo = env.journal.undo_last(&env.roots).unwrap().unwrap();
    assert!(undo.moved.is_empty());
    assert_eq!(undo.skipped.len(), 1);
    assert!(elsewhere.is_file());
}

// ---------------------------------------------------------------- settings

#[test]
fn settings_created_with_defaults_and_editable() {
    let env = Env::new();
    let path = env.base.join("appdata").join("settings.json");
    let (s, warn) = Settings::load_or_create(&path);
    assert!(warn.is_none());
    assert_eq!(s.recent_days, 7);
    assert!(path.is_file());
    assert_eq!(s.category_for("X.JpEg"), "Images");
    assert_eq!(s.category_for("setup.exe"), "Installers");

    let mut edited = s.clone();
    edited.recent_days = 14;
    edited
        .categories
        .push(organizer_core::config::Category { name: "Ebooks".into(), extensions: vec![".EPUB".into()] });
    edited.save(&path).unwrap();
    let (s2, _) = Settings::load_or_create(&path);
    assert_eq!(s2.recent_days, 14);
    assert_eq!(s2.category_for("book.epub"), "Ebooks");
}

#[test]
fn broken_settings_file_is_not_overwritten() {
    let env = Env::new();
    let path = env.base.join("settings.json");
    fs::write(&path, "{ not json").unwrap();
    let (s, warn) = Settings::load_or_create(&path);
    assert!(warn.is_some());
    assert_eq!(s, Settings::default());
    assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
}

#[test]
fn unsafe_category_names_fall_back_to_other() {
    let mut s = Settings::default();
    s.categories
        .insert(0, organizer_core::config::Category { name: "..\\..\\Windows".into(), extensions: vec!["jpg".into()] });
    assert_eq!(s.category_for("a.jpg"), "Other");
}

#[test]
fn check_entry_reports_folder() {
    let env = Env::new();
    let c = organizer_core::safety::check_entry(&env.roots.downloads, Duration::ZERO, SystemTime::now());
    assert_eq!(c, Check::Folder);
}
