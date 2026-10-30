//! Stage 4, part 3: the "Old installers" list.

use organizer_core::installers::{find_installers, InstallerOptions};
use organizer_core::remove::recycle_files;
use organizer_core::safety::SkipReason;
use organizer_core::{Journal, Progress, Recycler, Roots, ScannedFile, Settings};
use std::fs;
use std::io;
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
        let tmp = tempfile::Builder::new().prefix("fo-inst").tempdir_in(parent).unwrap();
        let base = tmp.path().to_path_buf();
        let roots = Roots {
            downloads: base.join("Downloads"),
            desktop: base.join("Desktop"),
            documents: base.join("Documents"),
            extra: Vec::new(),
        };
        for k in roots.kinds() {
            fs::create_dir_all(roots.get(k)).unwrap();
        }
        let journal = Journal::new(base.join("appdata").join("journal.jsonl"));
        Env { _tmp: tmp, base, roots, journal, settings: Settings::default() }
    }

    fn found(&self) -> organizer_core::installers::InstallerScan {
        let o = InstallerOptions::new(&self.settings, SystemTime::now());
        find_installers(&self.roots, &o, &Progress::default()).unwrap()
    }

    fn names(&self) -> Vec<String> {
        self.found().files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().to_string()).collect()
    }
}

struct TestBin(PathBuf);

impl Recycler for TestBin {
    fn recycle(&self, path: &Path) -> io::Result<Option<PathBuf>> {
        fs::create_dir_all(&self.0)?;
        let dst = self.0.join(path.file_name().unwrap());
        organizer_core::fsops::move_no_replace(path, &dst)?;
        Ok(Some(dst))
    }
}

fn days(n: u64) -> Duration {
    Duration::from_secs(n * 86_400)
}

fn file(dir: &Path, name: &str, age: Duration) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    fs::write(&p, b"MZ installer").unwrap();
    fs::OpenOptions::new().write(true).open(&p).unwrap().set_times(ft(SystemTime::now() - age)).unwrap();
    p
}

#[test]
fn installers_older_than_the_limit_are_listed_oldest_first() {
    let mut env = Env::new();
    let d = env.roots.downloads.clone();
    file(&d, "new-setup.exe", days(5));
    file(&d, "setup.exe", days(40));
    file(&d, "Tool.MSI", days(400));
    file(&d, "app.msix", days(31));
    file(&d, "store.appx", days(60));
    file(&d, "bundle.msixbundle", days(60));
    file(&d, "bundle2.appxbundle", days(60));
    file(&d, "photo.jpg", days(400));
    file(&d, "notes.zip", days(400));
    let names = env.names();
    assert_eq!(names[0], "Tool.MSI");
    for n in ["setup.exe", "app.msix", "store.appx", "bundle.msixbundle", "bundle2.appxbundle"] {
        assert!(names.contains(&n.to_string()), "{n}: {names:?}");
    }
    assert!(!names.iter().any(|n| n == "new-setup.exe" || n.ends_with(".jpg") || n.ends_with(".zip")));
    env.settings.installer_days = 3;
    assert!(env.names().contains(&"new-setup.exe".to_string()));
    env.settings.installer_days = 100;
    assert_eq!(env.names(), vec!["Tool.MSI"]);
}

#[test]
fn installer_days_setting_default_range_and_round_trip() {
    let s = Settings::default();
    assert_eq!(s.installer_days, 30);
    for bad in ["0", "3651", "\"x\""] {
        let (s, warn) = Settings::from_json(&format!("{{\"installer_days\": {bad}}}"));
        assert_eq!(s.installer_days, 30, "{bad}");
        assert!(warn.is_some());
    }
    let (s, warn) = Settings::from_json("{\"installer_days\": 90}");
    assert_eq!((s.installer_days, warn), (90, None));
    let env = Env::new();
    let path = env.base.join("appdata").join("settings.json");
    Settings::update(&path, |s| s.installer_days = 3650).unwrap();
    assert_eq!(Settings::load_or_create(&path).0.installer_days, 3650);
}

#[test]
fn only_top_of_downloads_and_sorted_installer_month_folders() {
    let env = Env::new();
    let d = env.roots.downloads.clone();
    file(&d, "top.exe", days(40));
    file(&d.join("Installers").join("2025-03"), "sorted.exe", days(40));
    file(&d.join("installers").join("2025-04"), "case.msi", days(40)); // same folder on Windows
                                                                       // Never anywhere else:
    file(&d.join("Installers").join("2025-03").join("deeper"), "deep.exe", days(40));
    file(&d.join("Installers").join("old stuff"), "notmonth.exe", days(40));
    file(&d.join("Installers"), "loose-in-type-folder.exe", days(40));
    file(&d.join("PortableTool"), "tool.exe", days(40));
    file(&d.join("Other").join("2025-03"), "other.exe", days(40));
    file(&env.roots.desktop, "desk.exe", days(40));
    file(&env.roots.documents.join("Installers").join("2025-03"), "docs.exe", days(40));
    let names = env.names();
    assert!(names.contains(&"top.exe".to_string()));
    assert!(names.contains(&"sorted.exe".to_string()));
    for n in ["deep.exe", "notmonth.exe", "loose-in-type-folder.exe", "tool.exe", "other.exe", "desk.exe", "docs.exe"] {
        assert!(!names.contains(&n.to_string()), "{n} must not be listed: {names:?}");
    }
}

#[test]
fn renamed_installers_type_folder_is_followed() {
    let mut env = Env::new();
    for c in &mut env.settings.categories {
        if c.name == "Installers" {
            c.name = "Setups".into();
        }
    }
    file(&env.roots.downloads.join("Setups").join("2025-01"), "a.exe", days(40));
    file(&env.roots.downloads.join("Installers").join("2025-01"), "b.exe", days(40));
    assert_eq!(env.names(), vec!["a.exe"]);
}

#[test]
fn only_ticked_installers_go_to_the_recycle_bin() {
    let env = Env::new();
    let d = env.roots.downloads.clone();
    let a = file(&d, "a.exe", days(40));
    let b = file(&d.join("Installers").join("2025-02"), "b.msi", days(50));
    let c = file(&d, "c.exe", days(60));
    let scan = env.found();
    assert_eq!(scan.files.len(), 3);
    let ticked: Vec<ScannedFile> = scan.files.iter().filter(|f| f.path != a).cloned().collect();
    let r = recycle_files(&env.roots, &ticked, &env.journal, &TestBin(env.base.join("bin")), SystemTime::now());
    assert_eq!(r.recycled.len(), 2, "{r:?}");
    assert!(a.is_file() && !b.exists() && !c.exists());
    assert_eq!(env.journal.history().unwrap()[0].recycled.len(), 2);
}

#[test]
fn installer_changed_after_the_list_is_not_removed() {
    let env = Env::new();
    let a = file(&env.roots.downloads, "a.exe", days(40));
    let scan = env.found();
    fs::write(&a, b"MZ changed!!").unwrap();
    let r = recycle_files(&env.roots, &scan.files, &env.journal, &TestBin(env.base.join("bin")), SystemTime::now());
    assert!(r.recycled.is_empty() && a.is_file());
}

#[test]
fn skip_rules_apply_to_installers() {
    let env = Env::new();
    let d = env.roots.downloads.clone();
    file(&d, "~$temp.exe", days(40));
    file(&d, "setup.exe.crdownload", days(40));
    // The type folder turned into a program folder (exe + dll): left alone.
    let month = d.join("Installers").join("2025-05");
    file(&month, "game.exe", days(40));
    file(&month, "engine.dll", days(40));
    let scan = env.found();
    assert!(scan.files.is_empty(), "{:?}", scan.files);
    assert!(scan.skipped.items.iter().any(|(p, r)| *p == month && *r == SkipReason::ProgramFolder));
}

#[cfg(unix)]
#[test]
fn hidden_installers_skipped_unix() {
    let env = Env::new();
    file(&env.roots.downloads, ".hidden.exe", days(40));
    assert!(env.found().files.is_empty());
}

#[cfg(windows)]
#[test]
fn hidden_system_and_locked_installers_skipped_windows() {
    let env = Env::new();
    let d = env.roots.downloads.clone();
    let h = file(&d, "hidden.exe", days(40));
    let s = file(&d, "system.msi", days(40));
    let l = file(&d, "locked.exe", days(40));
    for (flag, p) in [("+h", &h), ("+s", &s)] {
        assert!(std::process::Command::new("attrib").arg(flag).arg(p).status().unwrap().success());
    }
    let handle = fs::File::open(&l).unwrap();
    let scan = env.found();
    drop(handle);
    assert!(scan.files.is_empty(), "{:?}", scan.files);
    for (flag, p) in [("-h", &h), ("-s", &s)] {
        assert!(std::process::Command::new("attrib").arg(flag).arg(p).status().unwrap().success());
    }
}

#[test]
fn the_running_app_is_never_an_old_installer() {
    let env = Env::new();
    let exe = std::env::current_exe().unwrap();
    let roots = Roots { downloads: exe.parent().unwrap().to_path_buf(), ..env.roots.clone() };
    let mut o = InstallerOptions::new(&env.settings, SystemTime::now() + days(3650));
    o.extensions.push(exe.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default());
    let scan = find_installers(&roots, &o, &Progress::default()).unwrap();
    assert!(scan.files.iter().all(|f| f.path != exe));
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
