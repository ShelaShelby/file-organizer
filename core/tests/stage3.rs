//! Stage 3 tests: junk-name renaming, background sorting of Downloads,
//! journal lock between processes, "Start with Windows".

use organizer_core::autosort::{note_text, AutoSorter};
use organizer_core::journal::{Record, TAG_AUTO, TAG_RENAME};
use organizer_core::rename::{find_renames, is_valid_windows_name, proposed_name, rename_files, RenameScan};
use organizer_core::{execute, Journal, Progress, RootKind, Roots, ScanOptions, Settings};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const DAY: u64 = 86_400;

struct Env {
    _tmp: tempfile::TempDir,
    roots: Roots,
    journal: Journal,
    settings: Settings,
}

impl Env {
    fn new() -> Env {
        let parent = Path::new(env!("CARGO_TARGET_TMPDIR"));
        fs::create_dir_all(parent).unwrap();
        let tmp = tempfile::Builder::new().prefix("fo-s3").tempdir_in(parent).unwrap();
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
        Env { _tmp: tmp, roots, journal, settings: Settings::default() }
    }

    fn renames(&self) -> RenameScan {
        find_renames(&self.roots, SystemTime::now(), &Progress::default()).unwrap()
    }

    fn rename_all(&self, scan: &RenameScan) -> organizer_core::BatchReport {
        rename_files(&self.roots, &scan.items, &self.journal, SystemTime::now())
    }

    fn tick(&self, sorter: &mut AutoSorter, paused: bool) -> Option<organizer_core::BatchReport> {
        sorter.tick(&self.roots, &self.settings, &self.journal, paused, SystemTime::now())
    }
}

fn file(dir: &Path, name: &str, content: &str, age: Duration) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
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

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    v.sort();
    v
}

fn proposals(scan: &RenameScan) -> Vec<(String, String)> {
    let mut v: Vec<_> = scan.items.iter().map(|i| (i.old_name(), i.new_name())).collect();
    v.sort();
    v
}

fn p(name: &str) -> Option<String> {
    proposed_name(name)
}

fn s(x: &str) -> Option<String> {
    Some(x.to_string())
}

// ================================================================ rename: patterns

#[test]
fn rename_stacked_copy_counters() {
    assert_eq!(p("photo (1) (2).jpg"), s("photo.jpg"));
    assert_eq!(p("photo (3) (4) (5).png"), s("photo.png"));
    assert_eq!(p("photo (1) (12).jpg"), s("photo.jpg"));
    // A single counter is left alone.
    assert_eq!(p("photo (1).jpg"), None);
    assert_eq!(p("photo (2).jpg"), None);
    // Not counters: no space, letters, nothing before them.
    assert_eq!(p("photo(1)(2).jpg"), None);
    assert_eq!(p("photo (a) (b).jpg"), None);
    assert_eq!(p("(1) (2).jpg"), None);
}

#[test]
fn rename_copy_of_and_dash_copy() {
    assert_eq!(p("Copy of report.docx"), s("report.docx"));
    assert_eq!(p("copy of report.docx"), s("report.docx"));
    assert_eq!(p("COPY OF report.docx"), s("report.docx"));
    assert_eq!(p("report - Copy.docx"), s("report.docx"));
    assert_eq!(p("report - copy.docx"), s("report.docx"));
    assert_eq!(p("Copy of Copy of report.docx"), s("report.docx"));
    // Only as prefix / suffix before the extension.
    assert_eq!(p("my Copy of report.docx"), None);
    assert_eq!(p("report - Copy 2.docx"), None);
    assert_eq!(p("Copyof report.docx"), None);
}

#[test]
fn rename_spaces() {
    assert_eq!(p("my  file .txt"), s("my file.txt"));
    assert_eq!(p("  lead.txt"), s("lead.txt"));
    assert_eq!(p("a   b    c.txt"), s("a b c.txt"));
    assert_eq!(p("clean name.txt"), None);
    assert_eq!(p("Copy of  my   notes .txt"), s("my notes.txt"));
}

#[test]
fn rename_keeps_extension_exactly() {
    assert_eq!(p("photo (1) (2).JPG"), s("photo.JPG"));
    assert_eq!(p("Copy of archive.tar.gz"), s("archive.tar.gz"));
    assert_eq!(p("Copy of notes"), s("notes")); // no extension stays none
                                                // Spaces in the extension are not touched (it never changes).
    assert_eq!(p("my  file.t x t"), s("my file.t x t"));
    for (old, new) in [("Copy of a.pdf", "a.pdf"), ("b - Copy.mp3", "b.mp3"), ("c (1) (2).zip", "c.zip")] {
        let got = p(old).unwrap();
        assert_eq!(got, new);
        assert_eq!(Path::new(&got).extension(), Path::new(old).extension());
    }
}

#[test]
fn rename_never_invents_numbers_or_empty_names() {
    for n in ["photo (1) (2).jpg", "Copy of report.docx", "a  b.txt", "x - Copy.txt"] {
        let new = p(n).unwrap();
        assert!(!new.contains('('), "{new}");
        assert!(new.chars().filter(|c| c.is_ascii_digit()).count() <= n.chars().filter(|c| c.is_ascii_digit()).count());
    }
    assert_eq!(p("   .txt"), None); // would leave no name
    assert_eq!(p("Copy of .txt"), s("Copy of.txt")); // only the space goes; never ".txt"
}

#[test]
fn rename_refuses_invalid_windows_names() {
    for bad in ["", "CON.txt", "con", "NUL.pdf", "COM1.doc", "lpt9", "a:b.txt", "a?.txt", "name.", "name "] {
        assert!(!is_valid_windows_name(bad), "{bad:?} should be invalid");
    }
    let long = format!("{}.txt", "x".repeat(260));
    assert!(!is_valid_windows_name(&long));
    for ok in ["report.docx", "COM0.txt", "console.txt", "a b.txt", "notes"] {
        assert!(is_valid_windows_name(ok), "{ok:?} should be valid");
    }
    // A cleanup that would give a reserved name is not proposed.
    assert_eq!(p("Copy of CON.txt"), None);
    assert_eq!(p("nul - Copy"), None);
}

// ================================================================ rename: scan and rename

#[test]
fn rename_scan_finds_only_patterns_in_all_roots_and_subfolders() {
    let env = Env::new();
    file(&env.roots.downloads, "photo (1) (2).jpg", "a", days(3));
    file(&env.roots.downloads, "photo (1).jpg", "b", days(3));
    file(&env.roots.desktop, "Copy of report.docx", "c", days(3));
    file(&env.roots.documents.join("Sub").join("Deeper"), "my  file .txt", "d", days(3));
    file(&env.roots.documents, "fine.txt", "e", days(3));
    let scan = env.renames();
    assert_eq!(
        proposals(&scan),
        vec![
            ("Copy of report.docx".into(), "report.docx".into()),
            ("my  file .txt".into(), "my file.txt".into()),
            ("photo (1) (2).jpg".into(), "photo.jpg".into()),
        ]
    );
    for i in &scan.items {
        assert_eq!(i.new_path.parent(), i.file.path.parent(), "same folder only");
    }
}

#[test]
fn rename_skips_when_target_exists_case_insensitive() {
    let env = Env::new();
    file(&env.roots.downloads, "Copy of report.docx", "copy", days(3));
    file(&env.roots.downloads, "REPORT.DOCX", "original", days(3));
    file(&env.roots.downloads, "x - Copy.txt", "copy", days(3));
    file(&env.roots.downloads, "x.txt", "original", days(3));
    let scan = env.renames();
    assert!(scan.items.is_empty(), "{:?}", proposals(&scan));
    assert_eq!(scan.conflicts.len(), 2);
    let r = env.rename_all(&scan);
    assert!(r.moved.is_empty());
    assert_eq!(fs::read_to_string(env.roots.downloads.join("REPORT.DOCX")).unwrap(), "original");
    assert_eq!(fs::read_to_string(env.roots.downloads.join("x.txt")).unwrap(), "original");
}

#[test]
fn rename_skips_two_proposals_with_same_target() {
    let env = Env::new();
    file(&env.roots.downloads, "Copy of b.txt", "1", days(3));
    file(&env.roots.downloads, "b - Copy.txt", "2", days(3));
    file(&env.roots.downloads, "B (1) (2).TXT", "3", days(3)); // different extension case: own target "B.TXT" = same name
    let scan = env.renames();
    assert!(scan.items.is_empty(), "{:?}", proposals(&scan));
    assert_eq!(scan.conflicts.len(), 3);
    // Same target name in DIFFERENT folders is fine.
    let env = Env::new();
    file(&env.roots.downloads, "Copy of b.txt", "1", days(3));
    file(&env.roots.documents, "Copy of b.txt", "2", days(3));
    assert_eq!(env.renames().items.len(), 2);
}

#[test]
fn rename_renames_in_place_keeps_content_and_journals() {
    let env = Env::new();
    let old = file(&env.roots.downloads, "photo (1) (2).jpg", "pixels", days(3));
    let scan = env.renames();
    let r = env.rename_all(&scan);
    assert_eq!(r.moved.len(), 1, "{r:?}");
    assert!(!old.exists());
    let new = env.roots.downloads.join("photo.jpg");
    assert_eq!(fs::read_to_string(&new).unwrap(), "pixels");
    let recs = env.journal.records().unwrap();
    assert!(matches!(&recs[0], Record::Move(e) if e.tag == TAG_RENAME && e.old == old && e.new == new));
    let h = env.journal.history().unwrap();
    assert_eq!(h[0].tag, TAG_RENAME);
    assert_eq!(h[0].remaining.len(), 1);
}

#[test]
fn rename_never_overwrites_file_created_after_scan() {
    let env = Env::new();
    let old = file(&env.roots.downloads, "Copy of report.docx", "copy", days(3));
    let scan = env.renames();
    assert_eq!(scan.items.len(), 1);
    file(&env.roots.downloads, "Report.docx", "made later", days(3)); // other case too
    let r = env.rename_all(&scan);
    assert!(r.moved.is_empty());
    assert_eq!(r.skipped.len() + r.errors.len(), 1);
    assert_eq!(fs::read_to_string(&old).unwrap(), "copy");
    assert_eq!(fs::read_to_string(env.roots.downloads.join("Report.docx")).unwrap(), "made later");
    assert!(env.journal.records().unwrap().is_empty());
}

#[test]
fn rename_skips_file_changed_or_gone_after_scan() {
    let env = Env::new();
    let a = file(&env.roots.downloads, "Copy of a.txt", "a", days(3));
    let b = file(&env.roots.downloads, "Copy of b.txt", "b", days(3));
    let scan = env.renames();
    fs::write(&a, "changed!").unwrap();
    set_age(&a, days(3));
    fs::remove_file(&b).unwrap();
    let r = env.rename_all(&scan);
    assert!(r.moved.is_empty(), "{r:?}");
    assert_eq!(r.skipped.len(), 2);
    assert_eq!(fs::read_to_string(&a).unwrap(), "changed!");
}

#[test]
fn rename_refuses_tampered_proposals() {
    let env = Env::new();
    file(&env.roots.downloads, "Copy of a.txt", "a", days(3));
    let mut scan = env.renames();
    let mut other_folder = scan.items[0].clone();
    other_folder.new_path = env.roots.documents.join("a.txt");
    let mut other_ext = scan.items[0].clone();
    other_ext.new_path = env.roots.downloads.join("a.exe");
    scan.items = vec![other_folder, other_ext];
    let r = env.rename_all(&scan);
    assert!(r.moved.is_empty());
    assert_eq!(r.errors.len(), 2);
    assert_eq!(names_in(&env.roots.downloads), vec!["Copy of a.txt"]);
}

#[test]
fn rename_skips_hidden_temp_shortcut_and_recent() {
    let env = Env::new();
    file(&env.roots.downloads, "Copy of link.lnk", "l", days(3));
    file(&env.roots.downloads, "Copy of x.tmp", "t", days(3));
    file(&env.roots.downloads, "Copy of x.crdownload", "t", days(3));
    file(&env.roots.downloads, "Copy of new.txt", "n", secs(5));
    #[cfg(windows)]
    {
        let h = file(&env.roots.downloads, "Copy of hidden.txt", "h", days(3));
        assert!(std::process::Command::new("attrib").arg("+h").arg(&h).status().unwrap().success());
    }
    #[cfg(not(windows))]
    file(&env.roots.downloads, ".Copy of  hidden.txt", "h", days(3));
    assert!(env.renames().items.is_empty(), "{:?}", proposals(&env.renames()));
}

#[cfg(windows)]
#[test]
fn rename_skips_locked_file_windows() {
    let env = Env::new();
    let a = file(&env.roots.downloads, "Copy of a.txt", "a", days(3));
    let scan = env.renames();
    let h = fs::File::open(&a).unwrap();
    let r = env.rename_all(&scan);
    assert!(r.moved.is_empty());
    assert_eq!(r.skipped.len(), 1);
    drop(h);
    assert!(a.exists());
}

#[test]
fn rename_undo_restores_old_name() {
    let env = Env::new();
    let old = file(&env.roots.documents.join("Sub"), "a  b.txt", "x", days(3));
    let r = env.rename_all(&env.renames());
    assert_eq!(r.moved.len(), 1);
    let u = env.journal.undo_last().unwrap().unwrap();
    assert_eq!(u.moved.len(), 1, "{u:?}");
    assert_eq!(fs::read_to_string(&old).unwrap(), "x");
    assert_eq!(names_in(&env.roots.documents.join("Sub")), vec!["a  b.txt"]);
    assert!(env.journal.history().unwrap()[0].remaining.is_empty());
}

#[test]
fn rename_undo_only_if_still_new_name_and_old_name_free() {
    let env = Env::new();
    let old_a = file(&env.roots.downloads, "Copy of a.txt", "a", days(3));
    let old_b = file(&env.roots.downloads, "Copy of b.txt", "b", days(3));
    let r = env.rename_all(&env.renames());
    assert_eq!(r.moved.len(), 2);
    // a: user renamed it again; b: something new took the old name.
    fs::rename(env.roots.downloads.join("a.txt"), env.roots.downloads.join("mine.txt")).unwrap();
    file(&env.roots.downloads, "Copy of b.txt", "new file", days(3));
    let u = env.journal.undo(r.batch).unwrap();
    assert!(u.moved.is_empty(), "{u:?}");
    assert_eq!(u.skipped.len(), 2);
    assert!(!old_a.exists());
    assert_eq!(fs::read_to_string(env.roots.downloads.join("mine.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(&old_b).unwrap(), "new file");
    assert_eq!(fs::read_to_string(env.roots.downloads.join("b.txt")).unwrap(), "b");
}

// ================================================================ background sorting

#[test]
fn auto_moves_finished_download_after_two_checks() {
    let env = Env::new();
    let f = file(&env.roots.downloads, "photo.jpg", "img", days(2));
    let mut sorter = AutoSorter::new();
    assert!(env.tick(&mut sorter, false).is_none(), "first sight: not yet");
    assert!(f.exists());
    let r = env.tick(&mut sorter, false).expect("second check moves it");
    assert_eq!(r.moved.len(), 1);
    let month = organizer_core::plan::month_folder(fs::metadata(&r.moved[0].1).unwrap().modified().unwrap());
    assert_eq!(r.moved[0].1, env.roots.downloads.join("Images").join(&month).join("photo.jpg"));
    let recs = env.journal.records().unwrap();
    assert!(matches!(&recs[0], Record::Move(e) if e.tag == TAG_AUTO));
    assert_eq!(env.journal.history().unwrap()[0].tag, TAG_AUTO);
    assert_eq!(
        note_text(&env.roots, &r.moved),
        format!("Moved 1 file to {}", Path::new("Downloads").join("Images").join(&month).display())
    );
}

#[test]
fn auto_skips_too_new_files() {
    let env = Env::new();
    let a = file(&env.roots.downloads, "a.pdf", "a", secs(5));
    let b = file(&env.roots.downloads, "b.pdf", "b", secs(90)); // > 60 s but < the 2-minute rule
    let mut sorter = AutoSorter::new();
    for _ in 0..3 {
        assert!(env.tick(&mut sorter, false).is_none());
    }
    assert!(a.exists() && b.exists());
}

#[test]
fn auto_waits_while_size_is_still_changing() {
    let env = Env::new();
    let f = file(&env.roots.downloads, "big.zip", "part1", days(1));
    let mut sorter = AutoSorter::new();
    assert!(env.tick(&mut sorter, false).is_none());
    fs::write(&f, "part1part2").unwrap(); // grew between checks
    set_age(&f, days(1));
    assert!(env.tick(&mut sorter, false).is_none(), "size changed: wait");
    assert!(f.exists());
    let r = env.tick(&mut sorter, false).expect("stable now");
    assert_eq!(r.moved.len(), 1);
}

#[test]
fn auto_never_moves_temp_or_downloading_files() {
    let env = Env::new();
    let mut files = Vec::new();
    for ext in ["crdownload", "part", "partial", "tmp", "download", "CRDOWNLOAD"] {
        files.push(file(&env.roots.downloads, &format!("movie.{ext}"), "x", days(1)));
    }
    let mut sorter = AutoSorter::new();
    for _ in 0..3 {
        assert!(env.tick(&mut sorter, false).is_none());
    }
    assert!(files.iter().all(|f| f.exists()));
}

#[test]
fn auto_does_nothing_while_paused() {
    let env = Env::new();
    let f = file(&env.roots.downloads, "song.mp3", "x", days(1));
    let mut sorter = AutoSorter::new();
    for _ in 0..3 {
        assert!(env.tick(&mut sorter, true).is_none());
    }
    assert!(f.exists());
    assert!(env.journal.records().unwrap().is_empty());
    // Resumed: needs two fresh checks again.
    assert!(env.tick(&mut sorter, false).is_none());
    assert!(env.tick(&mut sorter, false).is_some());
    assert!(!f.exists());
}

#[test]
fn auto_pause_is_saved_in_settings() {
    let env = Env::new();
    let path = env.journal.path().with_file_name("settings.json");
    Settings::load_or_create(&path);
    assert!(!Settings::load_or_create(&path).0.background_paused, "default is running");
    Settings::update(&path, |s| s.background_paused = true).unwrap();
    assert!(Settings::load_or_create(&path).0.background_paused);
    // The window saving its own settings keeps the tray's switch.
    let other = Settings { big_file_mb: 42, ..Settings::default() };
    Settings::update(&path, |s| {
        let paused = s.background_paused;
        *s = other.clone();
        s.background_paused = paused;
    })
    .unwrap();
    let back = Settings::load_or_create(&path).0;
    assert!(back.background_paused);
    assert_eq!(back.big_file_mb, 42);
}

#[test]
fn auto_only_top_level_of_downloads() {
    let env = Env::new();
    let sub = file(&env.roots.downloads.join("Folder"), "inner.jpg", "x", days(1));
    let desk = file(&env.roots.desktop, "desk.jpg", "x", days(30));
    let mut sorter = AutoSorter::new();
    for _ in 0..3 {
        assert!(env.tick(&mut sorter, false).is_none());
    }
    assert!(sub.exists() && desk.exists());
}

#[cfg(windows)]
#[test]
fn auto_skips_locked_file_windows() {
    let env = Env::new();
    let f = file(&env.roots.downloads, "doc.pdf", "x", days(1));
    let mut sorter = AutoSorter::new();
    let h = fs::File::open(&f).unwrap();
    for _ in 0..3 {
        assert!(env.tick(&mut sorter, false).is_none());
    }
    assert!(f.exists());
    drop(h);
    assert!(env.tick(&mut sorter, false).is_none());
    assert!(env.tick(&mut sorter, false).is_some());
    assert!(!f.exists());
}

#[test]
fn auto_undo_last_auto_sort_only() {
    let env = Env::new();
    let f = file(&env.roots.downloads, "a.pdf", "x", days(1));
    let mut sorter = AutoSorter::new();
    env.tick(&mut sorter, false);
    assert_eq!(env.tick(&mut sorter, false).unwrap().moved.len(), 1);
    // A later manual rename must not be what "Undo last auto-sort" undoes.
    let g = file(&env.roots.documents, "Copy of z.txt", "z", days(3));
    assert_eq!(env.rename_all(&env.renames()).moved.len(), 1);
    let u = env.journal.undo_last_tagged(TAG_AUTO).unwrap().unwrap();
    assert_eq!(u.moved.len(), 1);
    assert!(f.exists(), "auto move undone");
    assert!(!g.exists(), "rename left as it is");
    assert!(env.journal.undo_last_tagged(TAG_AUTO).unwrap().is_none());
}

#[test]
fn note_text_for_several_folders() {
    let env = Env::new();
    let d = &env.roots.downloads;
    let moved = vec![
        (d.join("a.jpg"), d.join("Images").join("2026-10").join("a.jpg")),
        (d.join("b.jpg"), d.join("Images").join("2026-10").join("b.jpg")),
        (d.join("c.pdf"), d.join("Documents").join("2026-09").join("c.pdf")),
    ];
    let t = note_text(&env.roots, &moved);
    assert!(t.starts_with("Moved 3 files to "), "{t}");
    assert!(t.ends_with("and 1 other folder"), "{t}");
}

// ================================================================ two processes

#[test]
fn journal_lock_is_exclusive() {
    let env = Env::new();
    let held = env.journal.lock().unwrap();
    assert!(env.journal.try_lock().unwrap().is_none(), "second holder must wait");
    drop(held);
    assert!(env.journal.try_lock().unwrap().is_some());
}

#[test]
fn same_file_is_never_moved_twice_by_parallel_batches() {
    let env = Env::new();
    for i in 0..20 {
        file(&env.roots.downloads, &format!("f{i}.pdf"), "x", days(1));
    }
    let opts = ScanOptions::new(&env.settings, SystemTime::now());
    let plan = organizer_core::scan(&env.roots, &env.settings, &opts).unwrap();
    assert_eq!(plan.moves.len(), 20);
    let reports: Vec<_> = std::thread::scope(|s| {
        let h: Vec<_> = (0..2)
            .map(|_| s.spawn(|| execute(&env.roots, &plan.moves, &Journal::new(env.journal.path()), &opts)))
            .collect();
        h.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let moved: usize = reports.iter().map(|r| r.moved.len()).sum();
    assert_eq!(moved, 20, "each file moved exactly once");
    assert_ne!(reports[0].batch, reports[1].batch);
    let recs = env.journal.records().unwrap();
    assert_eq!(recs.len(), 20, "one readable journal line per move");
    let month = names_in(&env.roots.downloads.join("Documents"));
    assert_eq!(month.len(), 1);
    assert_eq!(names_in(&env.roots.downloads.join("Documents").join(&month[0])).len(), 20);
}

// ================================================================ Start with Windows

#[test]
fn autostart_command_line() {
    let exe = Path::new(r"C:\Apps\file-organizer.exe");
    assert_eq!(organizer_core::autostart::command_for(exe), "\"C:\\Apps\\file-organizer.exe\" --tray");
}

#[cfg(windows)]
#[test]
fn autostart_value_written_and_removed_in_hkcu() {
    use organizer_core::autostart::*;
    let name = format!("FileOrganizerTest-{}", std::process::id());
    let cmd = command_for(Path::new(r"C:\Test\file-organizer.exe"));
    assert_eq!(get_run_value(&name).unwrap(), None);
    set_run_value(&name, &cmd).unwrap();
    let got = get_run_value(&name);
    remove_run_value(&name).unwrap(); // clean up before asserting
    assert_eq!(got.unwrap().as_deref(), Some(cmd.as_str()));
    assert_eq!(get_run_value(&name).unwrap(), None);
    remove_run_value(&name).unwrap(); // removing twice is fine

    // The real switch, only if it is off on this machine (CI): on, then off again.
    if get_run_value(VALUE_NAME).unwrap().is_none() {
        let exe = std::env::current_exe().unwrap();
        set_enabled(true, &exe).unwrap();
        let on = is_enabled();
        let value = get_run_value(VALUE_NAME).unwrap();
        set_enabled(false, &exe).unwrap();
        assert!(on);
        assert_eq!(value, Some(command_for(&exe)));
        assert!(!is_enabled());
        assert_eq!(get_run_value(VALUE_NAME).unwrap(), None);
    }
}
