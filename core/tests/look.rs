//! Stage 4, part 5: logic behind the look and feel (the window itself is
//! not tested; everything it computes lives here).

use organizer_core::config::Theme;
use organizer_core::dupes::{find_duplicates, sort_groups, DupOptions};
use organizer_core::journal::totals;
use organizer_core::plan::summary_by_folder;
use organizer_core::remove::recycle_files;
use organizer_core::tree::{apply_order, sort_order, SortKey};
use organizer_core::window::{font_files, layout, tab_title, MIN_CONTENT_WIDTH, MIN_WINDOW};
use organizer_core::{execute, scan, Journal, Progress, Recycler, Roots, ScanOptions, ScannedFile, Settings};
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
        let tmp = tempfile::Builder::new().prefix("fo-look").tempdir_in(parent).unwrap();
        let base = tmp.path().to_path_buf();
        let roots = Roots::new(base.join("Downloads"), base.join("Desktop"), base.join("Documents"));
        for k in roots.kinds() {
            fs::create_dir_all(roots.get(k)).unwrap();
        }
        let journal = Journal::new(base.join("appdata").join("journal.jsonl"));
        Env { _tmp: tmp, base, roots, journal, settings: Settings::default() }
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

fn file(dir: &Path, name: &str, content: &[u8], age: Duration) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    fs::OpenOptions::new().write(true).open(&p).unwrap().set_modified(SystemTime::now() - age).unwrap();
    p
}

fn sf(name: &str, size: u64, age_days: u64) -> ScannedFile {
    ScannedFile {
        root: organizer_core::RootKind::Downloads,
        path: PathBuf::from(name),
        size,
        modified: SystemTime::UNIX_EPOCH + days(10_000 - age_days),
    }
}

#[test]
fn theme_setting_defaults_to_system_and_round_trips() {
    assert_eq!(Settings::default().theme, Theme::System);
    for (text, want) in [("\"dark\"", Theme::Dark), ("\"light\"", Theme::Light), ("\"system\"", Theme::System)] {
        let (s, warn) = Settings::from_json(&format!("{{\"theme\": {text}}}"));
        assert_eq!((s.theme, warn), (want, None));
    }
    let (s, warn) = Settings::from_json("{\"theme\": \"purple\"}");
    assert_eq!(s.theme, Theme::System);
    assert!(warn.is_some());
    let env = Env::new();
    let path = env.base.join("appdata").join("settings.json");
    Settings::update(&path, |s| s.theme = Theme::Light).unwrap();
    assert_eq!(Settings::load_or_create(&path).0.theme, Theme::Light);
    assert!(fs::read_to_string(&path).unwrap().contains("\"theme\": \"light\""));
    assert_eq!(Theme::ALL.len(), 3);
}

#[test]
fn tab_titles_show_counts_after_a_scan() {
    assert_eq!(tab_title("Duplicates", Some(12)), "Duplicates (12)");
    assert_eq!(tab_title("Rename", Some(0)), "Rename (0)");
    assert_eq!(tab_title("Sort", None), "Sort");
}

#[test]
fn layout_leaves_room_for_content_at_the_smallest_window() {
    let small = layout(MIN_WINDOW[0]);
    assert!(small.content_width >= MIN_CONTENT_WIDTH, "{small:?}");
    assert!(small.history_width >= 160.0);
    let big = layout(1150.0);
    assert!(big.history_width > small.history_width && big.history_width <= 300.0);
    assert_eq!(layout(300.0), small, "never smaller than the minimum window");
}

#[test]
fn lists_sort_by_name_size_and_date() {
    let files = vec![sf("b.txt", 5, 3), sf("A.txt", 9, 1), sf("c.txt", 1, 7)];
    let names = |o: &[usize]| o.iter().map(|&i| files[i].path.display().to_string()).collect::<Vec<_>>();
    assert_eq!(names(&sort_order(&files, SortKey::Name, true)), ["A.txt", "b.txt", "c.txt"]);
    assert_eq!(names(&sort_order(&files, SortKey::Size, false)), ["A.txt", "b.txt", "c.txt"]);
    assert_eq!(names(&sort_order(&files, SortKey::Size, true)), ["c.txt", "b.txt", "A.txt"]);
    assert_eq!(names(&sort_order(&files, SortKey::Modified, true)), ["c.txt", "b.txt", "A.txt"]);
    let mut ticks = vec![true, false, false];
    let order = sort_order(&files, SortKey::Name, true);
    apply_order(&mut ticks, &order);
    assert_eq!(ticks, [false, true, false], "ticks move with their rows");
}

#[test]
fn duplicate_groups_sort_and_keep_ticks_follow() {
    let env = Env::new();
    let small = vec![1u8; 2000];
    let large = vec![2u8; 9000];
    file(&env.roots.downloads, "z-small.bin", &small, days(30));
    file(&env.roots.documents, "a-small.bin", &small, days(20));
    file(&env.roots.downloads, "m-large.bin", &large, days(10));
    file(&env.roots.desktop, "b-large.bin", &large, days(5));
    let mut scan =
        find_duplicates(&env.roots, &DupOptions::new(&env.settings, SystemTime::now()), &Progress::default()).unwrap();
    let mut keep: Vec<Vec<bool>> = scan.groups.iter().map(|g| g.default_keep()).collect();
    let mut include = vec![true, false];
    let kept_before: Vec<PathBuf> =
        scan.groups.iter().zip(&keep).map(|(g, k)| g.files[k.iter().position(|x| *x).unwrap()].path.clone()).collect();
    sort_groups(&mut scan.groups, &mut keep, &mut include, SortKey::Name, true);
    let first: Vec<String> =
        scan.groups.iter().map(|g| g.files[0].path.file_name().unwrap().to_string_lossy().to_string()).collect();
    assert_eq!(first, ["a-small.bin", "b-large.bin"]);
    assert_eq!(include, [false, true], "include ticks follow their group");
    for (g, k) in scan.groups.iter().zip(&keep) {
        let kept = &g.files[k.iter().position(|x| *x).unwrap()].path;
        assert!(kept_before.contains(kept), "keep tick follows its file");
    }
    sort_groups(&mut scan.groups, &mut keep, &mut include, SortKey::Size, false);
    assert_eq!(scan.groups[0].size, 9000);
}

#[test]
fn history_totals_from_the_journal() {
    let env = Env::new();
    file(&env.roots.downloads, "a.pdf", b"aaaa", days(30));
    file(&env.roots.downloads, "b.pdf", b"bb", days(30));
    let opts = ScanOptions::new(&env.settings, SystemTime::now());
    let plan = scan(&env.roots, &env.settings, &opts).unwrap();
    assert_eq!(execute(&env.roots, &plan.moves, &env.journal, &opts).moved.len(), 2);
    let big = file(&env.roots.documents, "old.iso", &[0u8; 3000], days(400));
    let m = fs::metadata(&big).unwrap();
    let f = ScannedFile {
        root: organizer_core::RootKind::Documents,
        path: big,
        size: m.len(),
        modified: m.modified().unwrap(),
    };
    let r = recycle_files(&env.roots, &[f], &env.journal, &TestBin(env.base.join("bin")), SystemTime::now());
    assert_eq!(r.recycled.len(), 1);
    let t = env.journal.totals().unwrap();
    assert_eq!((t.recycled_files, t.recycled_bytes, t.tidied_files), (1, 3000, 2));
    env.journal.undo_last(&env.roots).unwrap();
    assert_eq!(env.journal.totals().unwrap().tidied_files, 0, "undone moves are not counted");
    assert_eq!(totals(&[]).recycled_files, 0);
}

#[test]
fn sort_summary_line_per_folder() {
    let env = Env::new();
    for i in 0..3 {
        file(&env.roots.downloads, &format!("d{i}.pdf"), b"d", days(30));
    }
    file(&env.roots.documents, "doc.txt", b"d", days(30));
    let extra = env.base.join("Projects");
    fs::create_dir_all(&extra).unwrap();
    file(&extra, "p.zip", b"p", days(30));
    let (roots, w) = env.roots.clone().with_extra(&[extra]);
    assert!(w.is_empty());
    let plan = scan(&roots, &env.settings, &ScanOptions::new(&env.settings, SystemTime::now())).unwrap();
    assert_eq!(summary_by_folder(&plan, &roots), "Downloads 3 · Documents 1 · Projects 1");
}

#[test]
fn fonts_are_used_only_if_the_files_exist() {
    let env = Env::new();
    assert_eq!(font_files(&env.base), (None, None));
    fs::write(env.base.join("segoeui.ttf"), b"x").unwrap();
    let (main, emoji) = font_files(&env.base);
    assert_eq!(main, Some(env.base.join("segoeui.ttf")));
    assert!(emoji.is_none());
    #[cfg(windows)]
    {
        let dir = organizer_core::paths::fonts_dir().unwrap();
        assert!(dir.is_dir());
    }
}
