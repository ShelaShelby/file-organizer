//! Stage 2 tests: duplicates, big / old lists, Recycle Bin removal, settings.

use organizer_core::bigold::{find_big_old, BigOldOptions};
use organizer_core::dupes::{
    check_choices, find_duplicates, remove_duplicates, space_freed, DupOptions, GroupChoice,
};
use organizer_core::journal::Record;
use organizer_core::remove::{recycle_files, root_of};
use organizer_core::{
    Journal, Progress, RecycleBin, Recycler, RootKind, Roots, ScannedFile, Settings,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const DAY: u64 = 86_400;

struct Env {
    _tmp: tempfile::TempDir,
    base: PathBuf,
    roots: Roots,
    journal: Journal,
    settings: Settings,
    bin: TestBin,
}

impl Env {
    fn new() -> Env {
        let parent = Path::new(env!("CARGO_TARGET_TMPDIR"));
        fs::create_dir_all(parent).unwrap();
        let tmp = tempfile::Builder::new()
            .prefix("fo-s2")
            .tempdir_in(parent)
            .unwrap();
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
        let bin = TestBin::new(base.join("bin"));
        Env {
            _tmp: tmp,
            base,
            roots,
            journal,
            settings: Settings::default(),
            bin,
        }
    }

    fn dupes(&self) -> organizer_core::dupes::DupScan {
        find_duplicates(
            &self.roots,
            &DupOptions::new(&self.settings, SystemTime::now()),
            &Progress::default(),
        )
        .unwrap()
    }

    fn big_old(&self) -> organizer_core::bigold::BigOldScan {
        find_big_old(
            &self.roots,
            &BigOldOptions::new(&self.settings, SystemTime::now()),
            &Progress::default(),
        )
        .unwrap()
    }

    fn remove(&self, choices: &[GroupChoice]) -> organizer_core::BatchReport {
        remove_duplicates(
            &self.roots,
            choices,
            &self.journal,
            &self.bin,
            SystemTime::now(),
        )
    }
}

/// Stand-in Recycle Bin: moves files into a folder (never deletes) and
/// remembers what it was asked to do.
struct TestBin {
    dir: PathBuf,
    calls: Mutex<Vec<PathBuf>>,
    fail: bool,
}

impl TestBin {
    fn new(dir: PathBuf) -> TestBin {
        fs::create_dir_all(&dir).unwrap();
        TestBin {
            dir,
            calls: Mutex::new(Vec::new()),
            fail: false,
        }
    }
    fn calls(&self) -> Vec<PathBuf> {
        self.calls.lock().unwrap().clone()
    }
}

impl Recycler for TestBin {
    fn recycle(&self, path: &Path) -> io::Result<Option<PathBuf>> {
        self.calls.lock().unwrap().push(path.to_path_buf());
        if self.fail {
            return Err(io::Error::other("no Recycle Bin on this drive"));
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let n = self.calls.lock().unwrap().len();
        let dst = self.dir.join(format!("{n}-{name}"));
        organizer_core::fsops::move_no_replace(path, &dst)?;
        Ok(Some(dst))
    }
}

fn days(n: u64) -> Duration {
    Duration::from_secs(n * DAY)
}

fn file(dir: &Path, name: &str, content: &[u8], age: Duration) -> PathBuf {
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

/// `n` bytes of content that depends on `seed`.
fn data(seed: u8, n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn names(files: &[ScannedFile]) -> Vec<String> {
    files
        .iter()
        .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
        .collect()
}

fn group_names(scan: &organizer_core::dupes::DupScan) -> Vec<Vec<String>> {
    let mut g: Vec<Vec<String>> = scan
        .groups
        .iter()
        .map(|g| {
            let mut n = names(&g.files);
            n.sort();
            n
        })
        .collect();
    g.sort();
    g
}

fn choice(g: &organizer_core::dupes::DupGroup) -> GroupChoice {
    GroupChoice {
        files: g.files.clone(),
        keep: g.default_keep(),
    }
}

// ---------------------------------------------------------------- finding

#[test]
fn duplicates_found_by_content_not_name() {
    let env = Env::new();
    file(&env.roots.downloads, "report.pdf", &data(1, 5000), days(10));
    file(
        &env.roots.documents,
        "copy of something.bin",
        &data(1, 5000),
        days(5),
    );
    file(&env.roots.desktop, "report.pdf", &data(2, 5000), days(5)); // same name, other content
    let scan = env.dupes();
    assert_eq!(
        group_names(&scan),
        vec![vec![
            "copy of something.bin".to_string(),
            "report.pdf".to_string()
        ]]
    );
    assert_eq!(scan.groups[0].files[0].root, RootKind::Downloads);
}

#[test]
fn same_size_different_content_not_grouped() {
    let env = Env::new();
    file(&env.roots.downloads, "a.bin", &data(1, 3000), days(5));
    file(&env.roots.downloads, "b.bin", &data(9, 3000), days(5));
    // Same size and the same first block, different only at the very end.
    let x = data(3, 100_000);
    let mut y = x.clone();
    *y.last_mut().unwrap() ^= 0xFF;
    file(&env.roots.downloads, "x.bin", &x, days(5));
    file(&env.roots.downloads, "y.bin", &y, days(5));
    assert!(env.dupes().groups.is_empty());
}

#[test]
fn big_identical_files_grouped_with_full_hash() {
    let env = Env::new();
    let d = data(7, 300_000);
    file(&env.roots.downloads, "one.iso", &d, days(5));
    file(&env.roots.documents, "two.iso", &d, days(5));
    file(&env.roots.desktop, "three.iso", &d, days(5));
    let scan = env.dupes();
    assert_eq!(scan.groups.len(), 1);
    assert_eq!(scan.groups[0].files.len(), 3);
    assert_eq!(space_freed(&[choice(&scan.groups[0])]), 600_000);
}

#[test]
fn empty_and_tiny_files_ignored() {
    let mut env = Env::new();
    file(&env.roots.downloads, "empty1.txt", b"", days(5));
    file(&env.roots.downloads, "empty2.txt", b"", days(5));
    file(&env.roots.downloads, "tiny1.txt", &data(1, 100), days(5));
    file(&env.roots.downloads, "tiny2.txt", &data(1, 100), days(5));
    assert!(env.dupes().groups.is_empty(), "default minimum is 1 KB");
    env.settings.min_duplicate_kb = 0;
    assert_eq!(
        group_names(&env.dupes()),
        vec![vec!["tiny1.txt".to_string(), "tiny2.txt".to_string()]]
    );
}

#[test]
fn subfolders_are_scanned() {
    let env = Env::new();
    let deep = env
        .roots
        .documents
        .join("Projects")
        .join("2024")
        .join("old");
    file(&deep, "deep.docx", &data(4, 2048), days(5));
    file(
        &env.roots.downloads.join("stuff"),
        "top.docx",
        &data(4, 2048),
        days(5),
    );
    assert_eq!(
        group_names(&env.dupes()),
        vec![vec!["deep.docx".to_string(), "top.docx".to_string()]]
    );
}

#[test]
fn default_keep_is_oldest_copy() {
    let env = Env::new();
    file(&env.roots.downloads, "newer.jpg", &data(5, 2048), days(3));
    file(
        &env.roots.documents,
        "oldest.jpg",
        &data(5, 2048),
        days(300),
    );
    file(&env.roots.desktop, "middle.jpg", &data(5, 2048), days(30));
    let scan = env.dupes();
    let g = &scan.groups[0];
    assert_eq!(
        names(&g.files),
        vec!["oldest.jpg", "middle.jpg", "newer.jpg"]
    );
    assert_eq!(g.default_keep(), vec![true, false, false]);
}

#[test]
fn skip_rules_apply_to_duplicates() {
    let env = Env::new();
    let d = data(6, 2048);
    file(&env.roots.downloads, "real.dat", &d, days(5));
    for n in [
        "a.tmp",
        "b.crdownload",
        "c.part",
        "~$d.docx",
        "e.lnk",
        "f.url",
        "desktop.ini",
    ] {
        file(&env.roots.downloads, n, &d, days(5));
    }
    file(
        &env.roots.downloads,
        "just-now.dat",
        &d,
        Duration::from_secs(10),
    ); // may still be downloading
    let scan = env.dupes();
    assert!(scan.groups.is_empty(), "{:?}", group_names(&scan));
    assert!(scan.skipped.count >= 8);
}

#[cfg(unix)]
#[test]
fn hidden_files_and_folders_skipped_unix() {
    let env = Env::new();
    let d = data(6, 2048);
    file(&env.roots.downloads, "real.dat", &d, days(5));
    file(&env.roots.downloads, ".hidden.dat", &d, days(5));
    file(
        &env.roots.downloads.join(".hiddendir"),
        "inside.dat",
        &d,
        days(5),
    );
    assert!(env.dupes().groups.is_empty());
}

#[cfg(windows)]
fn attrib(flag: &str, p: &Path) {
    let st = std::process::Command::new("attrib")
        .arg(flag)
        .arg(p)
        .status()
        .unwrap();
    assert!(st.success());
}

#[cfg(windows)]
#[test]
fn hidden_and_system_files_and_folders_skipped_windows() {
    let env = Env::new();
    let d = data(6, 2048);
    file(&env.roots.downloads, "real.dat", &d, days(5));
    let h = file(&env.roots.downloads, "hidden.dat", &d, days(5));
    let s = file(&env.roots.downloads, "system.dat", &d, days(5));
    let hd = env.roots.documents.join("HiddenDir");
    file(&hd, "inside.dat", &d, days(5));
    attrib("+h", &h);
    attrib("+s", &s);
    attrib("+h", &hd);
    assert!(env.dupes().groups.is_empty());
    let mut o = BigOldOptions::new(&env.settings, SystemTime::now());
    o.big_bytes = 1;
    let big = find_big_old(&env.roots, &o, &Progress::default())
        .unwrap()
        .big;
    assert_eq!(names(&big), vec!["real.dat"]);
    attrib("-h", &h);
    attrib("-s", &s);
    attrib("-h", &hd);
}

#[cfg(windows)]
#[test]
fn locked_files_skipped_windows() {
    let env = Env::new();
    let d = data(6, 2048);
    file(&env.roots.downloads, "a.dat", &d, days(5));
    let locked = file(&env.roots.downloads, "b.dat", &d, days(5));
    let h = fs::File::open(&locked).unwrap();
    assert!(env.dupes().groups.is_empty());
    drop(h);
    let scan = env.dupes();
    assert_eq!(scan.groups.len(), 1);
    // Locked after the scan: skipped on confirm, nothing removed.
    let h = fs::File::open(&locked).unwrap();
    let mut c = choice(&scan.groups[0]);
    c.keep = vec![false, false];
    let a_pos = c.files.iter().position(|f| f.path == locked).unwrap();
    c.keep[1 - a_pos] = true;
    let r = env.remove(&[c]);
    drop(h);
    assert!(r.recycled.is_empty(), "{r:?}");
    assert!(locked.is_file());
}

#[cfg(windows)]
#[test]
fn junction_folders_not_entered_windows() {
    let env = Env::new();
    let d = data(6, 2048);
    let real = env.base.join("outside");
    let target = file(&real, "a.dat", &d, days(5));
    file(&env.roots.downloads, "b.dat", &d, days(5));
    let j = env.roots.documents.join("Link");
    let st = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&j)
        .arg(&real)
        .status()
        .unwrap();
    assert!(st.success());
    assert!(env.dupes().groups.is_empty());
    // A path through the junction is refused for removal too.
    assert!(root_of(&env.roots, &j.join("a.dat")).is_err());
    fs::remove_dir(&j).unwrap();
    assert!(target.is_file());
}

#[test]
fn symlinks_not_followed() {
    let env = Env::new();
    let d = data(6, 2048);
    let real = file(&env.base.join("outside"), "a.dat", &d, days(5));
    file(&env.roots.downloads, "b.dat", &d, days(5));
    let link = env.roots.downloads.join("link.dat");
    let dirlink = env.roots.documents.join("dirlink");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&real, &link).and(std::os::unix::fs::symlink(
        env.base.join("outside"),
        &dirlink,
    ));
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(&real, &link).and(
        std::os::windows::fs::symlink_dir(env.base.join("outside"), &dirlink),
    );
    if made.is_err() {
        eprintln!("symlinks not permitted here; skipping");
        return;
    }
    assert!(env.dupes().groups.is_empty());
    assert!(root_of(&env.roots, &dirlink.join("a.dat")).is_err());
    assert!(real.is_file());
}

#[test]
fn cancel_stops_the_scan() {
    let env = Env::new();
    file(&env.roots.downloads, "a.dat", &data(1, 2048), days(5));
    let p = Progress::default();
    p.cancel();
    let opts = DupOptions::new(&env.settings, SystemTime::now());
    let err = find_duplicates(&env.roots, &opts, &p).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Interrupted);
}

// ---------------------------------------------------------------- removing

#[test]
fn confirm_sends_unkept_copies_to_recycler_and_journals() {
    let env = Env::new();
    let keep = file(&env.roots.documents, "keep.jpg", &data(5, 4096), days(100));
    let dup = file(&env.roots.downloads, "dup.jpg", &data(5, 4096), days(2));
    let scan = env.dupes();
    let r = env.remove(&[choice(&scan.groups[0])]);
    assert_eq!(r.recycled, vec![dup.clone()], "{r:?}");
    assert!(keep.is_file() && !dup.exists());
    assert_eq!(env.bin.calls(), vec![dup.clone()]);
    // Not deleted: the content is in the (test) Recycle Bin.
    let in_bin: Vec<_> = fs::read_dir(&env.bin.dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(fs::read(&in_bin[0]).unwrap(), data(5, 4096));
    // Journal line + history entry with the old path.
    let recs = env.journal.records().unwrap();
    assert!(matches!(&recs[0], Record::Recycle(e) if e.old == dup && e.new == in_bin[0]));
    let h = env.journal.history().unwrap();
    assert_eq!(h[0].recycled.len(), 1);
    assert_eq!(h[0].recycled[0].old, dup);
    assert!(
        h[0].remaining.is_empty(),
        "recycled files are not offered for move-undo"
    );
}

#[test]
fn user_can_change_kept_copy_and_untick_groups() {
    let env = Env::new();
    let a = file(&env.roots.documents, "a.jpg", &data(5, 4096), days(100));
    let b = file(&env.roots.downloads, "b.jpg", &data(5, 4096), days(2));
    let c = file(&env.roots.downloads, "c.txt", &data(6, 4096), days(2));
    let d = file(&env.roots.desktop, "d.txt", &data(6, 4096), days(3));
    let scan = env.dupes();
    let g1 = scan
        .groups
        .iter()
        .find(|g| g.files.iter().any(|f| f.path == a))
        .unwrap();
    let mut ch = choice(g1);
    ch.keep = ch.files.iter().map(|f| f.path == b).collect(); // keep the newer one instead
                                                              // The other group is unticked: simply not passed in.
    let r = env.remove(&[ch]);
    assert_eq!(r.recycled, vec![a.clone()]);
    assert!(!a.exists() && b.is_file() && c.is_file() && d.is_file());
}

#[test]
fn never_leaves_a_group_with_zero_copies() {
    let env = Env::new();
    let a = file(&env.roots.documents, "a.bin", &data(5, 4096), days(100));
    let b = file(&env.roots.downloads, "b.bin", &data(5, 4096), days(2));
    let scan = env.dupes();
    let mut ch = choice(&scan.groups[0]);
    ch.keep = vec![false, false];
    assert!(
        check_choices(std::slice::from_ref(&ch)).is_some(),
        "Confirm must be blocked"
    );
    let r = env.remove(&[ch.clone()]);
    assert!(r.recycled.is_empty());
    assert!(a.is_file() && b.is_file());
    assert!(env.bin.calls().is_empty());
    // Wrong length is refused too.
    ch.keep = vec![true];
    assert!(check_choices(std::slice::from_ref(&ch)).is_some());
    assert!(env.remove(&[ch]).recycled.is_empty());
    assert!(check_choices(&[choice(&scan.groups[0])]).is_none());
}

#[test]
fn group_left_alone_if_kept_copy_gone_or_changed() {
    let env = Env::new();
    let a = file(&env.roots.documents, "a.bin", &data(5, 4096), days(100));
    let b = file(&env.roots.downloads, "b.bin", &data(5, 4096), days(2));
    let scan = env.dupes();
    fs::write(&a, data(9, 4096)).unwrap(); // kept copy edited after the scan
    set_age(&a, days(100));
    let r = env.remove(&[choice(&scan.groups[0])]);
    assert!(r.recycled.is_empty(), "{r:?}");
    assert!(b.is_file());
    fs::remove_file(&a).unwrap(); // kept copy gone (test cleanup of our own file)
    assert!(env.remove(&[choice(&scan.groups[0])]).recycled.is_empty());
    assert!(b.is_file());
}

#[test]
fn file_changed_after_scan_is_skipped_on_confirm() {
    let env = Env::new();
    file(&env.roots.documents, "a.bin", &data(5, 4096), days(100));
    let b = file(&env.roots.downloads, "b.bin", &data(5, 4096), days(2));
    let scan = env.dupes();
    fs::write(&b, data(5, 5000)).unwrap(); // size changed
    set_age(&b, days(2));
    let r = env.remove(&[choice(&scan.groups[0])]);
    assert!(r.recycled.is_empty());
    assert_eq!(r.skipped.len(), 1);
    assert!(b.is_file());
}

#[test]
fn same_size_but_new_time_skipped_on_confirm() {
    let env = Env::new();
    file(&env.roots.documents, "a.bin", &data(5, 4096), days(100));
    let b = file(&env.roots.downloads, "b.bin", &data(5, 4096), days(20));
    let scan = env.dupes();
    set_age(&b, days(3));
    let r = env.remove(&[choice(&scan.groups[0])]);
    assert!(r.recycled.is_empty());
    assert!(b.is_file());
}

#[test]
fn recycler_failure_leaves_file_and_reports_it() {
    let mut env = Env::new();
    env.bin.fail = true;
    let a = file(&env.roots.documents, "a.bin", &data(5, 4096), days(100));
    let b = file(&env.roots.downloads, "b.bin", &data(5, 4096), days(2));
    let scan = env.dupes();
    let r = env.remove(&[choice(&scan.groups[0])]);
    assert!(r.recycled.is_empty());
    assert_eq!(r.errors.len(), 1);
    assert!(a.is_file() && b.is_file());
    assert!(env.journal.records().unwrap().is_empty());
}

#[test]
fn removal_refused_outside_roots_and_in_protected_places() {
    let env = Env::new();
    let outside = file(
        &env.base.join("elsewhere"),
        "x.bin",
        &data(1, 2048),
        days(30),
    );
    let f = ScannedFile {
        root: RootKind::Downloads,
        path: outside.clone(),
        size: 2048,
        modified: fs::metadata(&outside).unwrap().modified().unwrap(),
    };
    let r = recycle_files(&env.roots, &[f], &env.journal, &env.bin, SystemTime::now());
    assert_eq!(r.errors.len(), 1);
    assert!(outside.is_file() && env.bin.calls().is_empty());
    let dotdot = env
        .roots
        .downloads
        .join("..")
        .join("elsewhere")
        .join("x.bin");
    assert!(root_of(&env.roots, &dotdot).is_err());
    let appdata = env.roots.downloads.join("AppData").join("x.bin");
    assert!(root_of(&env.roots, &appdata).is_err());
    assert_eq!(
        root_of(&env.roots, &env.roots.downloads.join("ok.bin")),
        Ok(RootKind::Downloads)
    );
}

/// The real Recycle Bin: the file must end up IN the Recycle Bin (its
/// content is read back from there), not just vanish.
#[cfg(windows)]
#[test]
fn real_windows_recycle_bin() {
    let env = Env::new();
    let p = file(
        &env.roots.downloads,
        "recycle-me.txt",
        b"stage 2 recycle test",
        days(5),
    );
    let place = RecycleBin
        .recycle(&p)
        .expect("Recycle Bin should work on this drive");
    eprintln!("recycled to {place:?}");
    assert!(!p.exists());
    let place = place.expect("Windows reports the place in the Recycle Bin");
    assert!(place
        .to_string_lossy()
        .to_lowercase()
        .contains("$recycle.bin"));
    assert_eq!(fs::read(&place).unwrap(), b"stage 2 recycle test");
}

#[cfg(not(windows))]
#[test]
fn recycle_bin_never_deletes_off_windows() {
    let env = Env::new();
    let p = file(&env.roots.downloads, "x.txt", b"x", days(5));
    assert!(RecycleBin.recycle(&p).is_err());
    assert!(p.is_file());
}

// ---------------------------------------------------------------- big and old

fn sized(dir: &Path, name: &str, len: u64, age: Duration) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    let f = fs::File::create(&p).unwrap();
    f.set_len(len).unwrap();
    f.set_modified(SystemTime::now() - age).unwrap();
    p
}

#[test]
fn big_list_uses_size_setting_biggest_first() {
    let mut env = Env::new();
    const MB: u64 = 1024 * 1024;
    sized(&env.roots.downloads, "small.iso", MB / 2, days(1));
    sized(&env.roots.documents.join("sub"), "two.iso", 2 * MB, days(1));
    sized(&env.roots.desktop, "three.iso", 3 * MB, days(1));
    assert!(env.big_old().big.is_empty(), "default limit is 500 MB");
    env.settings.big_file_mb = 1;
    assert_eq!(names(&env.big_old().big), vec!["three.iso", "two.iso"]);
}

#[test]
fn old_list_uses_days_setting_oldest_first() {
    let mut env = Env::new();
    file(&env.roots.downloads, "new.txt", b"x", days(10));
    file(
        &env.roots.documents.join("a").join("b"),
        "older.txt",
        b"x",
        days(400),
    );
    file(&env.roots.desktop, "oldest.txt", b"x", days(900));
    assert_eq!(
        names(&env.big_old().old),
        vec!["oldest.txt", "older.txt"],
        "default 365 days"
    );
    env.settings.old_days = 5;
    assert_eq!(
        names(&env.big_old().old),
        vec!["oldest.txt", "older.txt", "new.txt"]
    );
}

#[test]
fn big_old_lists_are_capped() {
    let env = Env::new();
    for i in 0..5 {
        file(
            &env.roots.downloads,
            &format!("f{i}.txt"),
            b"x",
            days(400 + i),
        );
    }
    let mut o = BigOldOptions::new(&env.settings, SystemTime::now());
    o.limit = 3;
    let s = find_big_old(&env.roots, &o, &Progress::default()).unwrap();
    assert_eq!(names(&s.old), vec!["f4.txt", "f3.txt", "f2.txt"]);
}

#[test]
fn big_old_scan_changes_nothing_and_recycle_uses_same_rules() {
    let mut env = Env::new();
    env.settings.old_days = 30;
    let a = file(&env.roots.downloads, "a.txt", b"aaa", days(60));
    let b = file(&env.roots.downloads, "b.txt", b"bbb", days(90));
    let scan = env.big_old();
    assert!(a.is_file() && b.is_file());
    fs::write(&b, "changed").unwrap(); // changed after scan
    let r = recycle_files(
        &env.roots,
        &scan.old,
        &env.journal,
        &env.bin,
        SystemTime::now(),
    );
    assert_eq!(r.recycled, vec![a.clone()]);
    assert_eq!(r.skipped.len(), 1);
    assert!(!a.exists() && b.is_file());
    assert_eq!(env.journal.history().unwrap()[0].recycled[0].old, a);
}

// ---------------------------------------------------------------- settings

#[test]
fn settings_defaults_and_round_trip() {
    let env = Env::new();
    let path = env.base.join("appdata").join("settings.json");
    let (s, warn) = Settings::load_or_create(&path);
    assert!(warn.is_none());
    assert_eq!(
        (s.recent_days, s.big_file_mb, s.old_days, s.min_duplicate_kb),
        (7, 500, 365, 1)
    );
    let mut e = s.clone();
    e.big_file_mb = 2000;
    e.old_days = 30;
    e.min_duplicate_kb = 64;
    e.recent_days = 3;
    e.save(&path).unwrap();
    assert_eq!(Settings::load_or_create(&path).0, e);
}

#[test]
fn broken_values_fall_back_to_defaults() {
    let (s, warn) = Settings::from_json(
        r#"{"recent_days": 14, "big_file_mb": -5, "old_days": "a year", "min_duplicate_kb": 99999999999, "categories": 3}"#,
    );
    assert!(warn.is_some());
    let d = Settings::default();
    assert_eq!(s.recent_days, 14, "good values are kept");
    assert_eq!(s.big_file_mb, d.big_file_mb);
    assert_eq!(s.old_days, d.old_days);
    assert_eq!(s.min_duplicate_kb, d.min_duplicate_kb);
    assert_eq!(s.categories, d.categories);
    let (s, _) = Settings::from_json(r#"{"big_file_mb": 0, "old_days": 0}"#);
    assert_eq!((s.big_file_mb, s.old_days), (500, 365));
    // Old stage-1 file without the new fields: defaults for them.
    let (s, warn) = Settings::from_json(r#"{"recent_days": 9}"#);
    assert!(warn.is_none());
    assert_eq!((s.recent_days, s.big_file_mb), (9, 500));
}

#[test]
fn unreadable_settings_file_is_never_overwritten() {
    let env = Env::new();
    let path = env.base.join("settings.json");
    fs::write(&path, "{ not json").unwrap();
    let (s, warn) = Settings::load_or_create(&path);
    assert!(warn.is_some());
    assert_eq!(s, Settings::default());
    assert!(s.save(&path).is_err(), "saving must refuse");
    assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
}

#[test]
fn saving_fixes_out_of_range_values() {
    let env = Env::new();
    let path = env.base.join("settings.json");
    let s = Settings {
        big_file_mb: 0,
        ..Settings::default()
    };
    s.save(&path).unwrap();
    assert_eq!(Settings::load_or_create(&path).0.big_file_mb, 500);
}
