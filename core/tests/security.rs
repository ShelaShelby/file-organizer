//! Stage 4, part 1: security hardening. One or more tests per item.

use organizer_core::autosort::{AutoSorter, MAX_PER_TICK};
use organizer_core::autostart::{exe_in_command, status_for, Status};
use organizer_core::config::{safe_folder_name, Category, OTHER};
use organizer_core::dupes::{find_duplicates, remove_duplicates, DupOptions, GroupChoice};
use organizer_core::journal::{Entry, Record};
use organizer_core::paths::{final_path, is_protected};
use organizer_core::plan::month_folder;
use organizer_core::remove::{recycle_files, root_of};
use organizer_core::rename::find_renames;
use organizer_core::safety::SkipReason;
use organizer_core::{
    execute, scan, Journal, PlannedMove, Progress, Recycler, RootKind, Roots, ScanOptions, ScannedFile, Settings,
};
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
        let tmp = tempfile::Builder::new().prefix("fo-s4").tempdir_in(parent).unwrap();
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

    fn opts(&self) -> ScanOptions {
        ScanOptions::new(&self.settings, SystemTime::now())
    }
}

/// Stand-in Recycle Bin: moves files into a folder (never deletes).
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

fn file(dir: &Path, name: &str, content: &[u8], age: Duration) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    set_time(&p, SystemTime::now() - age);
    p
}

fn set_time(p: &Path, t: SystemTime) {
    fs::OpenOptions::new().write(true).open(p).unwrap().set_modified(t).unwrap();
}

fn mtime(p: &Path) -> SystemTime {
    fs::metadata(p).unwrap().modified().unwrap()
}

fn scanned(root: RootKind, p: &Path) -> ScannedFile {
    let m = fs::metadata(p).unwrap();
    ScannedFile { root, path: p.to_path_buf(), size: m.len(), modified: m.modified().unwrap() }
}

fn repo_file(rel: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)).unwrap()
}

// ---------------------------------------------------------------- 1. undo never trusts the journal

/// A file sorted into Downloads\Images\<month>, plus a hand-made journal
/// line claiming it came from `old`. Undo must refuse it.
fn tampered_undo(env: &Env, old: &Path) -> organizer_core::BatchReport {
    tampered_undo_says(env, old, "journal entry looks wrong")
}

fn tampered_undo_says(env: &Env, old: &Path, says: &str) -> organizer_core::BatchReport {
    let src = file(&env.roots.downloads, "x.jpg", b"img", days(30));
    let new = env.roots.downloads.join("Images").join(month_folder(mtime(&src))).join("x.jpg");
    fs::create_dir_all(new.parent().unwrap()).unwrap();
    fs::rename(&src, &new).unwrap();
    let e = Entry::new(42, old, &new, 3, mtime(&new));
    env.journal.append(&Record::Move(e)).unwrap();
    let report = env.journal.undo(42, &env.roots).unwrap();
    assert!(report.moved.is_empty(), "{report:?}");
    assert!(new.is_file(), "file must stay where it is");
    assert!(!old.is_file());
    assert!(report.skipped.iter().any(|(_, why)| why.contains(says)), "{report:?}");
    report
}

#[cfg(windows)]
#[test]
fn undo_refuses_journal_pointing_at_windows_folder() {
    let env = Env::new();
    let win = organizer_core::paths::windows_dir().unwrap();
    tampered_undo(&env, &win.join("x.jpg"));
    tampered_undo(&Env::new(), Path::new(r"C:\Windows\System32\x.jpg"));
}

#[test]
fn undo_refuses_journal_pointing_at_startup_folder() {
    let env = Env::new();
    let roaming = organizer_core::paths::app_data_dir().unwrap().parent().unwrap().to_path_buf();
    let startup = roaming.join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("Startup");
    tampered_undo(&env, &startup.join("x.jpg"));
}

#[test]
fn undo_refuses_journal_paths_with_dot_dot() {
    let env = Env::new();
    // Resolves to Desktop\x.jpg (inside a root), but `..` is never accepted.
    let old = env.roots.downloads.join("..").join("Desktop").join("x.jpg");
    tampered_undo(&env, &old);
    let env = Env::new();
    tampered_undo(&env, &env.roots.downloads.join(".").join("x.jpg"));
}

#[test]
fn undo_refuses_paths_outside_roots_root_itself_and_app_data() {
    let env = Env::new();
    tampered_undo_says(&env, &env.base.join("elsewhere").join("x.jpg"), "add the folder back in Settings");
    let env = Env::new();
    tampered_undo(&env, &env.roots.downloads); // the root itself
    let env = Env::new();
    tampered_undo(&env, &env.journal.path().with_file_name("settings.json"));
}

#[test]
fn undo_refuses_tampered_new_path() {
    // `new` points at a file outside every root: it is never moved anywhere.
    let env = Env::new();
    let outside = file(&env.base.join("elsewhere"), "secret.txt", b"s", days(30));
    let old = env.roots.downloads.join("secret.txt");
    let e = Entry::new(7, &old, &outside, 1, mtime(&outside));
    env.journal.append(&Record::Move(e)).unwrap();
    let r = env.journal.undo(7, &env.roots).unwrap();
    assert!(r.moved.is_empty() && outside.is_file() && !old.exists());
    assert!(r.skipped[0].1.contains("add the folder back in Settings"), "{r:?}");
}

#[cfg(windows)]
#[test]
fn undo_refuses_journal_through_junction() {
    let env = Env::new();
    let real = env.base.join("real");
    fs::create_dir_all(&real).unwrap();
    let j = env.roots.downloads.join("j");
    let st = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&j).arg(&real).status().unwrap();
    assert!(st.success());
    tampered_undo(&env, &j.join("x.jpg"));
    fs::remove_dir(&j).unwrap();
}

#[test]
fn normal_undo_still_works() {
    let env = Env::new();
    let src = file(&env.roots.desktop, "a.pdf", b"a", days(30));
    let plan = scan(&env.roots, &env.settings, &env.opts()).unwrap();
    assert_eq!(execute(&env.roots, &plan.moves, &env.journal, &env.opts()).moved.len(), 1);
    let u = env.journal.undo_last(&env.roots).unwrap().unwrap();
    assert_eq!(u.moved.len(), 1, "{u:?}");
    assert!(src.is_file());
}

// ---------------------------------------------------------------- 2. journal line breaks

#[test]
fn broken_last_journal_line_never_swallows_the_next_record() {
    let env = Env::new();
    fs::create_dir_all(env.journal.path().parent().unwrap()).unwrap();
    fs::write(env.journal.path(), "{\"kind\":\"move\",\"batch\":1,\"ti").unwrap(); // crash mid-line
    let e = Entry::new(5, &env.roots.downloads.join("a"), &env.roots.downloads.join("b"), 1, SystemTime::now());
    env.journal.append(&Record::Move(e.clone())).unwrap();
    env.journal.append(&Record::Move(e)).unwrap();
    assert_eq!(env.journal.records().unwrap().len(), 2);
    let text = fs::read_to_string(env.journal.path()).unwrap();
    assert_eq!(text.lines().count(), 3);
    assert!(text.ends_with('\n'));
}

// ---------------------------------------------------------------- 3. settings.json

#[test]
fn settings_saved_atomically_without_leftovers() {
    let env = Env::new();
    let path = env.base.join("appdata").join("settings.json");
    let (s, warn) = Settings::load_or_create(&path);
    assert!(warn.is_none());
    Settings::update(&path, |s| s.recent_days = 9).unwrap();
    assert!(!path.with_extension("json.tmp").exists());
    assert_eq!(Settings::from_json(&fs::read_to_string(&path).unwrap()).0.recent_days, 9);
    assert_eq!(s.recent_days, 7);
}

#[test]
fn settings_updates_from_two_processes_are_never_lost() {
    let env = Env::new();
    let path = env.base.join("appdata").join("settings.json");
    Settings::load_or_create(&path);
    let p1 = path.clone();
    let p2 = path.clone();
    let a = std::thread::spawn(move || {
        for _ in 0..30 {
            Settings::update(&p1, |s| s.recent_days += 1).unwrap();
        }
    });
    let b = std::thread::spawn(move || {
        for _ in 0..30 {
            Settings::update(&p2, |s| s.old_days += 1).unwrap();
        }
    });
    a.join().unwrap();
    b.join().unwrap();
    let s = Settings::from_json(&fs::read_to_string(&path).unwrap()).0;
    assert_eq!((s.recent_days, s.old_days), (7 + 30, 365 + 30));
}

#[test]
fn unreadable_settings_never_overwritten_by_update_or_save() {
    let env = Env::new();
    let path = env.base.join("appdata").join("settings.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "my notes, not json").unwrap();
    assert!(Settings::update(&path, |s| s.recent_days = 1).is_err());
    assert!(Settings::default().save(&path).is_err());
    let (_, warn) = Settings::load_or_create(&path);
    assert!(warn.is_some());
    assert_eq!(fs::read_to_string(&path).unwrap(), "my notes, not json");
}

// ---------------------------------------------------------------- 4. execute never trusts the plan

fn planned(env: &Env, src: &Path) -> PlannedMove {
    let plan = scan(&env.roots, &env.settings, &env.opts()).unwrap();
    plan.moves.into_iter().find(|m| m.source == src).unwrap()
}

#[test]
fn tampered_planned_moves_are_refused() {
    let env = Env::new();
    let src = file(&env.roots.downloads, "x.txt", b"x", days(30));
    let good = planned(&env, &src);
    let outside = env.base.join("X");
    let tampered = [
        PlannedMove { target: env.roots.downloads.join("..").join("X").join("x.txt"), ..good.clone() },
        PlannedMove { target: good.target.parent().unwrap().join("..").join("x.txt"), ..good.clone() },
        PlannedMove { target: env.roots.downloads.join("Documents").join("x.txt"), ..good.clone() },
        PlannedMove { target: outside.join("x.txt"), ..good.clone() },
        PlannedMove {
            category: "..".into(),
            target: env.roots.downloads.join("..").join("2020-01").join("x.txt"),
            ..good.clone()
        },
        PlannedMove { category: "CON".into(), ..good.clone() },
        PlannedMove { source: env.roots.downloads.join("..").join("Downloads").join("x.txt"), ..good.clone() },
        PlannedMove { root: RootKind::Documents, ..good.clone() },
    ];
    for t in &tampered {
        let r = execute(&env.roots, std::slice::from_ref(t), &env.journal, &env.opts());
        assert!(r.moved.is_empty() && r.errors.len() == 1, "{t:?} -> {r:?}");
        assert!(src.is_file());
    }
    assert!(!outside.exists());
    // The untouched plan still works.
    assert_eq!(execute(&env.roots, std::slice::from_ref(&good), &env.journal, &env.opts()).moved.len(), 1);
    assert!(good.target.is_file());
}

#[cfg(windows)]
#[test]
fn planned_target_compared_ignoring_case() {
    let env = Env::new();
    let src = file(&env.roots.downloads, "y.txt", b"y", days(30));
    let mut m = planned(&env, &src);
    m.target = PathBuf::from(m.target.to_string_lossy().to_uppercase());
    let r = execute(&env.roots, &[m], &env.journal, &env.opts());
    assert_eq!(r.moved.len(), 1, "{r:?}");
}

// ---------------------------------------------------------------- 5. category names

#[test]
fn category_names_can_never_be_device_names() {
    for bad in ["CON", "nul", "Com1", "LPT9", "aux.txt", "PRN ", "x.", "a:b", "..", ".", "", &"n".repeat(300)] {
        assert_eq!(safe_folder_name(bad), OTHER, "{bad:?}");
    }
    assert_eq!(safe_folder_name(" Pictures "), "Pictures");
    assert_eq!(safe_folder_name("COM0"), "COM0");
    let mut s = Settings::default();
    s.categories.insert(0, Category { name: "NUL".into(), extensions: vec!["jpg".into()] });
    assert_eq!(s.category_for("a.jpg"), OTHER);
}

// ---------------------------------------------------------------- 6. protected places

#[test]
fn protected_places_do_not_come_from_environment_variables() {
    let env = Env::new();
    assert!(!is_protected(&env.roots.downloads));
    for var in ["ProgramFiles", "SystemRoot", "windir", "ProgramData", "APPDATA", "LOCALAPPDATA"] {
        std::env::set_var(var, &env.base);
    }
    assert!(!is_protected(&env.roots.downloads), "a changed variable must not matter");
    assert!(env.roots.validate().is_ok());
}

#[test]
fn user_profile_itself_is_protected_but_not_folders_inside() {
    let home = organizer_core::paths::profile_dir().unwrap();
    assert!(is_protected(&home));
    assert!(!is_protected(&home.join("Downloads")));
    #[cfg(windows)]
    {
        for p in [r"C:\", r"C:\Windows\System32", r"C:\Program Files\Common Files\x", r"C:\ProgramData"] {
            assert!(is_protected(Path::new(p)), "{p}");
        }
        let local = organizer_core::paths::app_data_dir().unwrap().parent().unwrap().parent().unwrap().join("Local");
        assert!(is_protected(&local.join("Temp")));
    }
}

#[cfg(windows)]
#[test]
fn junction_root_to_windows_folder_is_refused() {
    let env = Env::new();
    let j = env.base.join("innocent");
    let win = organizer_core::paths::windows_dir().unwrap();
    let st = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&j).arg(&win).status().unwrap();
    assert!(st.success());
    assert!(!is_protected(&j), "the name alone looks harmless");
    let roots = Roots { downloads: j.clone(), ..env.roots.clone() };
    assert!(roots.validate().is_err());
    assert!(scan(&roots, &env.settings, &env.opts()).is_err());
    fs::remove_dir(&j).unwrap(); // removes only the junction
    assert!(win.join("explorer.exe").is_file());
}

#[test]
fn final_path_strips_verbatim_prefix() {
    let env = Env::new();
    let f = final_path(&env.roots.downloads).unwrap();
    assert!(!f.to_string_lossy().starts_with(r"\\?\"));
    assert!(f.is_absolute());
}

// ---------------------------------------------------------------- 7. never the app itself

fn exe_roots(env: &Env) -> (Roots, PathBuf) {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().to_path_buf();
    (Roots { downloads: dir, ..env.roots.clone() }, exe)
}

#[test]
fn running_exe_is_never_sorted_moved_or_recycled() {
    let env = Env::new();
    let (roots, exe) = exe_roots(&env);
    let plan = scan(&roots, &env.settings, &env.opts()).unwrap();
    assert!(plan.moves.iter().all(|m| m.source != exe));
    let why = plan.skipped.iter().find(|(p, _)| *p == exe).map(|(_, r)| r.clone());
    assert_eq!(why, Some(SkipReason::SelfApp));
    assert_eq!(SkipReason::SelfApp.to_string(), "File Organizer itself");
    // A hand-made plan for it is refused at the re-check.
    let m = mtime(&exe);
    let fake = PlannedMove {
        root: RootKind::Downloads,
        source: exe.clone(),
        target: roots.downloads.join("Other").join(month_folder(m)).join(exe.file_name().unwrap()),
        category: "Other".into(),
        size: fs::metadata(&exe).unwrap().len(),
        modified: m,
    };
    let r = execute(&roots, &[fake], &env.journal, &env.opts());
    assert!(r.moved.is_empty() && exe.is_file(), "{r:?}");
    assert_eq!(r.skipped[0].1, "File Organizer itself");
    let r = recycle_files(
        &roots,
        &[scanned(RootKind::Downloads, &exe)],
        &env.journal,
        &TestBin(env.base.join("bin")),
        SystemTime::now(),
    );
    assert!(r.recycled.is_empty() && exe.is_file(), "{r:?}");
}

#[test]
fn running_exe_is_left_alone_by_walking_scans() {
    // The walker feeds duplicates, big/old, rename and installers.
    let env = Env::new();
    let (roots, exe) = exe_roots(&env);
    let old = SystemTime::now() + days(1); // nothing counts as "recent"
    let mut found = Vec::new();
    let mut skipped = organizer_core::tree::Skipped::default();
    organizer_core::tree::walk(&roots, old, &Progress::default(), &mut skipped, |f| found.push(f.path)).unwrap();
    assert!(!found.is_empty() && !found.contains(&exe));
    assert_eq!(
        organizer_core::safety::check_entry_unlocked(&exe, Duration::ZERO, old),
        organizer_core::safety::Check::Skip(SkipReason::SelfApp)
    );
    // A copy (another file) is a normal file.
    let copy = env.base.join(exe.file_name().unwrap());
    fs::copy(&exe, &copy).unwrap();
    assert!(!organizer_core::paths::is_self_exe(&copy, fs::metadata(&copy).unwrap().len()));
}

// ---------------------------------------------------------------- 8. program and project folders

fn left_alone_as_program(env: &Env, folder: &Path) {
    let inside = file(folder, "Copy of notes.txt", b"n", days(30));
    let r = find_renames(&env.roots, SystemTime::now(), &Progress::default()).unwrap();
    assert!(r.items.iter().all(|i| i.file.path != inside), "{}", folder.display());
    assert!(
        r.skipped.items.iter().any(|(p, why)| p == folder && *why == SkipReason::ProgramFolder),
        "{}: {:?}",
        folder.display(),
        r.skipped.items
    );
    // Removal is refused there too.
    assert!(root_of(&env.roots, &inside).is_err());
}

#[test]
fn program_and_tool_folder_names_are_never_entered() {
    let env = Env::new();
    for name in organizer_core::safety::PROGRAM_FOLDER_NAMES {
        left_alone_as_program(&env, &env.roots.documents.join(name));
    }
    left_alone_as_program(&env, &env.roots.downloads.join("NODE_MODULES"));
}

#[test]
fn folders_with_project_markers_are_never_entered() {
    let env = Env::new();
    let mut markers: Vec<String> = organizer_core::safety::PROJECT_MARKERS.iter().map(|s| s.to_string()).collect();
    markers.push("App.sln".into());
    markers.push("Tool.csproj".into());
    for (i, m) in markers.iter().enumerate() {
        let dir = env.roots.documents.join(format!("project{i}"));
        if m == ".git" {
            fs::create_dir_all(dir.join(".git")).unwrap();
        } else {
            file(&dir, m, b"x", days(30));
        }
        left_alone_as_program(&env, &dir);
    }
    // A .git FILE (worktrees, submodules) counts too.
    let dir = env.roots.desktop.join("worktree");
    file(&dir, ".git", b"gitdir: x", days(30));
    left_alone_as_program(&env, &dir);
}

#[test]
fn program_folders_with_exe_and_dll_are_never_entered() {
    let env = Env::new();
    let dir = env.roots.downloads.join("PortableApp");
    file(&dir, "app.exe", b"MZ", days(30));
    file(&dir, "helper.dll", b"MZ", days(30));
    left_alone_as_program(&env, &dir);
    // Only an exe, or only a dll: a normal folder.
    let env = Env::new();
    let only = env.roots.downloads.join("setups");
    file(&only, "setup.exe", b"MZ", days(30));
    let inside = file(&only, "Copy of readme.txt", b"r", days(30));
    let r = find_renames(&env.roots, SystemTime::now(), &Progress::default()).unwrap();
    assert!(r.items.iter().any(|i| i.file.path == inside));
}

#[test]
fn project_rules_do_not_block_the_chosen_folder_itself() {
    let env = Env::new();
    file(&env.roots.documents, "package.json", b"{}", days(30));
    let inside = file(&env.roots.documents, "Copy of a.txt", b"a", days(30));
    let r = find_renames(&env.roots, SystemTime::now(), &Progress::default()).unwrap();
    assert!(r.items.iter().any(|i| i.file.path == inside));
}

// ---------------------------------------------------------------- 9. duplicates are compared again

fn dup_pair(env: &Env) -> (PathBuf, PathBuf, GroupChoice) {
    let a = file(&env.roots.downloads, "a.bin", &[7u8; 5000], days(20));
    let b = file(&env.roots.documents, "b.bin", &[7u8; 5000], days(10));
    let scan =
        find_duplicates(&env.roots, &DupOptions::new(&env.settings, SystemTime::now()), &Progress::default()).unwrap();
    assert_eq!(scan.groups.len(), 1);
    let g = &scan.groups[0];
    let choice = GroupChoice { files: g.files.clone(), keep: g.default_keep() };
    assert_eq!(choice.files[0].path, a);
    (a, b, choice)
}

/// Same size, same last-changed time, different bytes.
fn change_secretly(p: &Path) {
    let t = mtime(p);
    let mut data = fs::read(p).unwrap();
    data[100] ^= 0xff;
    fs::write(p, data).unwrap();
    set_time(p, t);
}

#[test]
fn duplicate_changed_without_size_or_time_change_is_refused() {
    let env = Env::new();
    let (a, b, choice) = dup_pair(&env);
    change_secretly(&b);
    let bin = TestBin(env.base.join("bin"));
    let r = remove_duplicates(&env.roots, &[choice], &env.journal, &bin, SystemTime::now());
    assert!(r.recycled.is_empty(), "{r:?}");
    assert!(a.is_file() && b.is_file());
    assert!(r.skipped[0].1.contains("no longer the same"));
}

#[test]
fn kept_copy_changed_secretly_also_refuses() {
    let env = Env::new();
    let (a, b, choice) = dup_pair(&env);
    change_secretly(&a);
    let r = remove_duplicates(&env.roots, &[choice], &env.journal, &TestBin(env.base.join("bin")), SystemTime::now());
    assert!(r.recycled.is_empty() && a.is_file() && b.is_file(), "{r:?}");
}

#[test]
fn unchanged_duplicate_still_goes() {
    let env = Env::new();
    let (a, b, choice) = dup_pair(&env);
    let r = remove_duplicates(&env.roots, &[choice], &env.journal, &TestBin(env.base.join("bin")), SystemTime::now());
    assert_eq!(r.recycled, vec![b.clone()]);
    assert!(a.is_file() && !b.exists());
}

#[cfg(windows)]
#[test]
fn duplicate_that_cannot_be_opened_exclusively_is_refused() {
    let env = Env::new();
    let (a, b, choice) = dup_pair(&env);
    let h = fs::File::open(&a).unwrap(); // kept copy in use
    let r = remove_duplicates(&env.roots, &[choice], &env.journal, &TestBin(env.base.join("bin")), SystemTime::now());
    drop(h);
    assert!(r.recycled.is_empty() && b.is_file(), "{r:?}");
}

// ---------------------------------------------------------------- 10. planted DLLs, manifest, Explorer

#[test]
fn build_restricts_dll_loading_and_embeds_manifest() {
    let build = repo_file("build.rs");
    assert!(build.contains("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800"));
    assert!(!repo_file("src/main.rs").contains("SetDefaultDllDirectories"));
    let rc = repo_file("assets/app.rc");
    assert!(rc.contains("1 24 \"app.manifest\""));
    let m = repo_file("assets/app.manifest");
    assert!(m.contains("level=\"asInvoker\""));
    assert!(!m.contains("requireAdministrator") && !m.contains("highestAvailable"));
    assert!(m.contains("{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"));
    assert!(m.contains("PerMonitorV2"));
}

#[test]
fn explorer_is_started_by_full_path() {
    let main = repo_file("src/main.rs");
    assert!(main.contains("windows_dir()?.join(\"explorer.exe\")"));
    assert!(!main.contains("Command::new(\"explorer.exe\")"));
    #[cfg(windows)]
    assert!(organizer_core::paths::windows_dir().unwrap().join("explorer.exe").is_file());
}

#[test]
fn ci_starts_the_tray_exe_as_a_loader_check() {
    let ci = repo_file(".github/workflows/build.yml");
    assert!(ci.contains("file-organizer.exe") && ci.contains("--tray"));
}

// ---------------------------------------------------------------- 11. background sorter load

fn tick(s: &mut AutoSorter, env: &Env) -> Vec<PlannedMove> {
    s.eligible(&env.roots, &env.settings, SystemTime::now())
}

#[test]
fn background_sorter_moves_at_most_200_per_tick() {
    let env = Env::new();
    for i in 0..(MAX_PER_TICK + 5) {
        file(&env.roots.downloads, &format!("f{i:03}.txt"), b"x", days(1));
    }
    let mut s = AutoSorter::new();
    assert!(tick(&mut s, &env).is_empty());
    assert_eq!(tick(&mut s, &env).len(), MAX_PER_TICK);
}

#[cfg(windows)]
#[test]
fn background_sorter_opens_files_only_once_they_are_stable() {
    // Locked during the first two checks: the cheap check still counts it
    // as stable, so it goes at the first check after it is closed.
    let env = Env::new();
    let p = file(&env.roots.downloads, "movie.mp4", b"m", days(1));
    let h = fs::File::open(&p).unwrap();
    let mut s = AutoSorter::new();
    assert!(tick(&mut s, &env).is_empty());
    assert!(tick(&mut s, &env).is_empty(), "locked file must not be picked");
    drop(h);
    assert_eq!(tick(&mut s, &env).len(), 1);
}

// ---------------------------------------------------------------- 12. Start with Windows path

#[test]
fn autostart_shows_which_copy_it_starts() {
    let env = Env::new();
    let exe = std::env::current_exe().unwrap();
    assert_eq!(exe_in_command(&format!("\"{}\" --tray", exe.display())), Some(exe.clone()));
    assert_eq!(exe_in_command(r"C:\x\a.exe --tray"), Some(PathBuf::from(r"C:\x\a.exe")));
    assert_eq!(status_for(None, &exe), Status::Off);
    let cmd = organizer_core::autostart::command_for(&exe);
    assert_eq!(status_for(Some(&cmd), &exe), Status::ThisCopy(exe.clone()));
    let other = file(&env.base, "file-organizer.exe", b"MZ", days(1));
    let cmd = organizer_core::autostart::command_for(&other);
    assert_eq!(status_for(Some(&cmd), &exe), Status::OtherCopy(other));
    let gone = env.base.join("gone").join("file-organizer.exe");
    let cmd = organizer_core::autostart::command_for(&gone);
    assert_eq!(status_for(Some(&cmd), &exe), Status::OtherCopy(gone));
}

// ---------------------------------------------------------------- 13. CI

#[test]
fn ci_actions_pinned_and_permissions_split() {
    let ci = repo_file(".github/workflows/build.yml");
    let uses: Vec<&str> =
        ci.lines().filter_map(|l| l.trim().strip_prefix("- uses:").or(l.trim().strip_prefix("uses:"))).collect();
    assert!(uses.len() >= 4);
    for u in uses {
        let sha = u.trim().split('@').nth(1).unwrap_or("").split_whitespace().next().unwrap_or("");
        assert!(sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()), "not pinned: {u}");
        assert!(u.contains('#'), "tag comment missing: {u}");
    }
    assert!(ci.contains("contents: read") && ci.contains("contents: write"));
    assert!(ci.contains("needs: build"));
    assert!(ci.contains("file-organizer.zip.sha256"));
    assert!(ci.contains("cargo-audit") && ci.contains("continue-on-error: true"));
}

// ---------------------------------------------------------------- 14. SECURITY.md

#[test]
fn security_md_is_short_and_names_the_limits() {
    let s = repo_file("SECURITY.md");
    assert!(s.lines().count() <= 40);
    for w in ["same Windows user", "SmartScreen", "OpenGL", "Recycle Bin"] {
        assert!(s.contains(w), "{w}");
    }
}
