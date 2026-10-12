//! File Organizer window. All file logic lives in `organizer-core`;
//! this code only draws the window and starts background jobs.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use organizer_core::bigold::{find_big_old, BigOldOptions, BigOldScan};
use organizer_core::dupes::{find_duplicates, remove_duplicates, DupOptions, DupScan, GroupChoice};
use organizer_core::journal::BatchReport;
use organizer_core::journal::{TAG_AUTO, TAG_RENAME};
use organizer_core::paths::app_data_dir;
use organizer_core::remove::recycle_files;
use organizer_core::rename::{find_renames, rename_files, RenameScan};
use organizer_core::{
    execute, scan, BatchSummary, Journal, Plan, Progress, RecycleBin, Recycler, Roots, ScanOptions,
    ScannedFile, Settings,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

mod icon;
mod note;
mod tabs;
#[cfg(windows)]
mod tray;

pub const RED: egui::Color32 = egui::Color32::from_rgb(240, 120, 110);

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--tray") => {
            #[cfg(windows)]
            tray::run();
            return Ok(());
        }
        Some("--note") => {
            let batch = args.get(1).and_then(|b| b.parse().ok()).unwrap_or(0);
            return note::run(batch);
        }
        _ => {}
    }
    // One main window: a second start brings the open one to the front.
    let _single = match single_window() {
        Some(m) => m,
        None => return Ok(()),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(organizer_core::window::MAIN_TITLE)
            .with_inner_size([1150.0, 720.0])
            .with_min_inner_size([760.0, 420.0])
            .with_icon(icon::icon()),
        ..Default::default()
    };
    eframe::run_native(
        "File Organizer",
        options,
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
    )
}

/// Holds the main-window mutex (`Some(None)` if Windows refused to create
/// it: open anyway), or, if a window is already open, brings that window to
/// the front and returns `None`. Sends it no data.
fn single_window() -> Option<Option<organizer_core::window::MainMutex>> {
    use organizer_core::window::{acquire_main_mutex, start_action, StartAction};
    let Some(m) = acquire_main_mutex() else {
        return Some(None);
    };
    let found = m.already_existed && find_main_window();
    match start_action(m.already_existed, found) {
        StartAction::Open => Some(Some(m)),
        StartAction::BringToFront | StartAction::Exit => None,
    }
}

/// Finds the main window by its exact title, restores and raises it.
fn find_main_window() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
        };
        let title: Vec<u16> = organizer_core::window::MAIN_TITLE
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: plain Win32 calls with a NUL-terminated title.
        unsafe {
            let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
            if hwnd.is_null() {
                return false;
            }
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
        true
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Sort,
    Duplicates,
    BigOld,
    Rename,
}

pub enum JobResult {
    Scanned(Result<Plan, String>),
    Dupes(Result<DupScan, String>),
    BigOld(Result<BigOldScan, String>),
    Renames(Result<RenameScan, String>),
    Done(Result<BatchReport, String>, &'static str),
}

pub struct Job {
    rx: Receiver<JobResult>,
    /// Set for scans that can be cancelled and show a file count.
    pub progress: Option<Arc<Progress>>,
    pub started: Instant,
    pub label: &'static str,
}

/// Duplicate groups with the user's choices.
pub struct DupView {
    pub scan: DupScan,
    pub include: Vec<bool>,
    pub keep: Vec<Vec<bool>>,
}

impl DupView {
    fn new(scan: DupScan) -> DupView {
        let keep = scan.groups.iter().map(|g| g.default_keep()).collect();
        let include = vec![true; scan.groups.len()];
        DupView {
            scan,
            include,
            keep,
        }
    }

    pub fn choices(&self) -> Vec<GroupChoice> {
        self.scan
            .groups
            .iter()
            .zip(&self.include)
            .zip(&self.keep)
            .filter(|((_, inc), _)| **inc)
            .map(|((g, _), k)| GroupChoice {
                files: g.files.clone(),
                keep: k.clone(),
            })
            .collect()
    }

    /// Drops files that are gone now; groups left with one copy disappear.
    fn forget(&mut self, gone: &[PathBuf]) {
        let mut i = 0;
        while i < self.scan.groups.len() {
            let g = &mut self.scan.groups[i];
            let k = &mut self.keep[i];
            let mut j = 0;
            while j < g.files.len() {
                if gone.contains(&g.files[j].path) {
                    g.files.remove(j);
                    k.remove(j);
                } else {
                    j += 1;
                }
            }
            if g.files.len() < 2 {
                self.scan.groups.remove(i);
                self.keep.remove(i);
                self.include.remove(i);
            } else {
                i += 1;
            }
        }
    }
}

pub struct BigOldView {
    pub scan: BigOldScan,
    pub show_old: bool,
}

/// Files waiting for the user to press Confirm in the dialog.
pub enum Pending {
    Duplicates(Vec<GroupChoice>),
    Files(Vec<ScannedFile>),
}

impl Pending {
    fn files(&self) -> Vec<&ScannedFile> {
        match self {
            Pending::Duplicates(c) => c.iter().flat_map(|c| c.to_remove()).collect(),
            Pending::Files(f) => f.iter().collect(),
        }
    }
}

pub struct App {
    pub tab: Tab,
    pub roots: Option<Roots>,
    pub settings: Settings,
    settings_path: Option<PathBuf>,
    settings_open: bool,
    settings_draft: Settings,
    pub journal: Option<Journal>,
    recycler: Arc<dyn Recycler>,
    // Sort tab
    pub plan: Option<Plan>,
    pub checked: Vec<bool>,
    // Duplicates tab
    pub dupes: Option<DupView>,
    // Big and old tab
    pub bigold: Option<BigOldView>,
    /// Which big/old list to show when the running scan finishes.
    bigold_show_old_next: bool,
    // Rename tab
    pub renames: Option<RenameScan>,
    pub rename_checked: Vec<bool>,
    pub pending: Option<Pending>,
    history: Vec<BatchSummary>,
    /// Last-changed time of the journal when the history was read (the
    /// background sorter writes it too).
    history_stamp: Option<SystemTime>,
    history_checked: Instant,
    autostart: organizer_core::autostart::Status,
    last: Option<(String, BatchReport)>,
    pub message: Option<String>,
    pub job: Option<Job>,
}

impl App {
    fn new(ctx: &egui::Context) -> App {
        ctx.set_theme(egui::ThemePreference::Dark);
        ctx.all_styles_mut(|s| {
            for font in s.text_styles.values_mut() {
                font.size *= 1.15;
            }
            s.spacing.item_spacing = egui::vec2(10.0, 8.0);
            s.spacing.button_padding = egui::vec2(12.0, 6.0);
        });
        let mut problems = Vec::new();
        let roots = Roots::from_system()
            .and_then(|r| r.validate().map(|_| r))
            .map_err(|e| problems.push(e.to_string()))
            .ok();
        let (settings, journal, settings_path) = match app_data_dir() {
            Ok(dir) => {
                let path = dir.join("settings.json");
                let (s, warn) = Settings::load_or_create(&path);
                problems.extend(warn);
                (
                    s,
                    Some(Journal::new(
                        dir.join(organizer_core::journal::JOURNAL_FILE),
                    )),
                    Some(path),
                )
            }
            Err(e) => {
                problems.push(format!("cannot find the app-data folder: {e}"));
                (Settings::default(), None, None)
            }
        };
        let mut app = App {
            tab: Tab::Sort,
            roots,
            settings_draft: settings.clone(),
            settings,
            settings_path,
            settings_open: false,
            journal,
            recycler: Arc::new(RecycleBin),
            plan: None,
            checked: Vec::new(),
            dupes: None,
            bigold: None,
            bigold_show_old_next: false,
            renames: None,
            rename_checked: Vec::new(),
            pending: None,
            history: Vec::new(),
            history_stamp: None,
            history_checked: Instant::now(),
            autostart: organizer_core::autostart::status(),
            last: None,
            message: (!problems.is_empty()).then(|| problems.join("\n")),
            job: None,
        };
        app.refresh_history();
        app
    }

    fn journal_stamp(&self) -> Option<SystemTime> {
        let j = self.journal.as_ref()?;
        std::fs::metadata(j.path()).and_then(|m| m.modified()).ok()
    }

    /// Re-reads the history when the journal changed (e.g. background sorting).
    fn refresh_history_if_changed(&mut self) {
        if self.history_checked.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.history_checked = Instant::now();
        if self.journal_stamp() != self.history_stamp {
            self.refresh_history();
        }
    }

    fn refresh_history(&mut self) {
        self.history_stamp = self.journal_stamp();
        if let Some(j) = &self.journal {
            match j.history() {
                Ok(h) => self.history = h,
                Err(e) => self.message = Some(format!("cannot read the journal: {e}")),
            }
        }
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn spawn(
        &mut self,
        ctx: &egui::Context,
        label: &'static str,
        progress: Option<Arc<Progress>>,
        work: impl FnOnce() -> JobResult + Send + 'static,
    ) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
        self.last = None;
        self.message = None;
        self.job = Some(Job {
            rx,
            progress,
            started: Instant::now(),
            label,
        });
    }

    pub fn start_scan(&mut self, ctx: &egui::Context) {
        let Some(roots) = self.roots.clone() else {
            return;
        };
        let settings = self.settings.clone();
        self.spawn(ctx, "Scanning", None, move || {
            let opts = ScanOptions::new(&settings, SystemTime::now());
            JobResult::Scanned(scan(&roots, &settings, &opts).map_err(|e| e.to_string()))
        });
    }

    pub fn start_confirm(&mut self, ctx: &egui::Context) {
        let (Some(roots), Some(plan), Some(journal)) =
            (self.roots.clone(), self.plan.take(), &self.journal)
        else {
            return;
        };
        let checked = std::mem::take(&mut self.checked);
        let moves: Vec<_> = plan
            .moves
            .into_iter()
            .zip(checked)
            .filter(|(_, c)| *c)
            .map(|(m, _)| m)
            .collect();
        let journal = Journal::new(journal.path());
        let settings = self.settings.clone();
        self.spawn(ctx, "Moving", None, move || {
            let opts = ScanOptions::new(&settings, SystemTime::now());
            JobResult::Done(Ok(execute(&roots, &moves, &journal, &opts)), "Organised")
        });
    }

    pub fn start_undo(&mut self, ctx: &egui::Context, batch: Option<u64>) {
        let (Some(journal), Some(roots)) = (&self.journal, self.roots.clone()) else {
            return;
        };
        let journal = Journal::new(journal.path());
        self.plan = None;
        self.checked.clear();
        self.spawn(ctx, "Undoing", None, move || {
            let r = match batch {
                Some(b) => journal.undo(b, &roots).map_err(|e| e.to_string()),
                None => journal
                    .undo_last(&roots)
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.ok_or_else(|| "Nothing to undo.".to_string())),
            };
            JobResult::Done(r, "Undo")
        });
    }

    pub fn start_dupes(&mut self, ctx: &egui::Context) {
        let Some(roots) = self.roots.clone() else {
            return;
        };
        let opts = DupOptions::new(&self.settings, SystemTime::now());
        let progress = Arc::new(Progress::default());
        let p = progress.clone();
        self.dupes = None;
        self.spawn(ctx, "Looking for duplicates", Some(progress), move || {
            JobResult::Dupes(find_duplicates(&roots, &opts, &p).map_err(|e| e.to_string()))
        });
    }

    pub fn start_bigold(&mut self, ctx: &egui::Context) {
        let Some(roots) = self.roots.clone() else {
            return;
        };
        let opts = BigOldOptions::new(&self.settings, SystemTime::now());
        let progress = Arc::new(Progress::default());
        let p = progress.clone();
        let show_old = self.bigold.as_ref().is_some_and(|b| b.show_old);
        self.bigold = None;
        self.spawn(
            ctx,
            "Looking for big and old files",
            Some(progress),
            move || JobResult::BigOld(find_big_old(&roots, &opts, &p).map_err(|e| e.to_string())),
        );
        self.bigold_show_old_next = show_old;
    }

    pub fn start_renames(&mut self, ctx: &egui::Context) {
        let Some(roots) = self.roots.clone() else {
            return;
        };
        let progress = Arc::new(Progress::default());
        let p = progress.clone();
        self.renames = None;
        self.rename_checked.clear();
        self.spawn(ctx, "Looking for messy names", Some(progress), move || {
            JobResult::Renames(
                find_renames(&roots, SystemTime::now(), &p).map_err(|e| e.to_string()),
            )
        });
    }

    pub fn start_rename_confirm(&mut self, ctx: &egui::Context) {
        let (Some(roots), Some(scan), Some(journal)) =
            (self.roots.clone(), self.renames.take(), &self.journal)
        else {
            return;
        };
        let checked = std::mem::take(&mut self.rename_checked);
        let items: Vec<_> = scan
            .items
            .into_iter()
            .zip(checked)
            .filter(|(_, c)| *c)
            .map(|(i, _)| i)
            .collect();
        let journal = Journal::new(journal.path());
        self.spawn(ctx, "Renaming", None, move || {
            JobResult::Done(
                Ok(rename_files(&roots, &items, &journal, SystemTime::now())),
                "Renamed",
            )
        });
    }

    pub fn start_background(&mut self) {
        let started = std::env::current_exe()
            .and_then(|exe| std::process::Command::new(exe).arg("--tray").spawn());
        self.message = Some(match started {
            Ok(_) => {
                "Background sorting is on: look for the File Organizer icon in the tray.".into()
            }
            Err(e) => format!("Could not start background sorting: {e}"),
        });
    }

    fn set_autostart(&mut self, on: bool) {
        let r = std::env::current_exe()
            .and_then(|exe| organizer_core::autostart::set_enabled(on, &exe));
        if let Err(e) = r {
            self.message = Some(format!("Start with Windows could not be changed: {e}"));
        }
        self.autostart = organizer_core::autostart::status();
    }

    fn start_removal(&mut self, ctx: &egui::Context) {
        let (Some(roots), Some(journal), Some(pending)) =
            (self.roots.clone(), &self.journal, self.pending.take())
        else {
            return;
        };
        let journal = Journal::new(journal.path());
        let recycler = self.recycler.clone();
        self.spawn(ctx, "Sending to the Recycle Bin", None, move || {
            let now = SystemTime::now();
            let r = match pending {
                Pending::Duplicates(c) => {
                    remove_duplicates(&roots, &c, &journal, recycler.as_ref(), now)
                }
                Pending::Files(f) => recycle_files(&roots, &f, &journal, recycler.as_ref(), now),
            };
            JobResult::Done(Ok(r), "Removed")
        });
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        let Ok(result) = job.rx.try_recv() else {
            return;
        };
        self.job = None;
        let cancelled = |e: &String| e == "cancelled";
        match result {
            JobResult::Scanned(Ok(plan)) => {
                self.checked = vec![true; plan.moves.len()];
                self.plan = Some(plan);
            }
            JobResult::Dupes(Ok(scan)) => self.dupes = Some(DupView::new(scan)),
            JobResult::Renames(Ok(scan)) => {
                self.rename_checked = vec![true; scan.items.len()];
                self.renames = Some(scan);
            }
            JobResult::BigOld(Ok(scan)) => {
                self.bigold = Some(BigOldView {
                    scan,
                    show_old: self.bigold_show_old_next,
                });
            }
            JobResult::Done(Ok(report), label) => {
                if let Some(d) = &mut self.dupes {
                    d.forget(&report.recycled);
                }
                if let Some(b) = &mut self.bigold {
                    b.scan.big.retain(|f| !report.recycled.contains(&f.path));
                    b.scan.old.retain(|f| !report.recycled.contains(&f.path));
                }
                self.last = Some((label.to_string(), report));
                self.refresh_history();
            }
            JobResult::Dupes(Err(e)) | JobResult::BigOld(Err(e)) | JobResult::Renames(Err(e))
                if cancelled(&e) =>
            {
                self.message = Some("Scan cancelled.".into());
            }
            JobResult::Scanned(Err(e))
            | JobResult::Dupes(Err(e))
            | JobResult::BigOld(Err(e))
            | JobResult::Renames(Err(e))
            | JobResult::Done(Err(e), _) => self.message = Some(e),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_job();
        self.refresh_history_if_changed();
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_secs(5)); // pick up background sorting in History
        if self.job.as_ref().is_some_and(|j| j.progress.is_some()) {
            ctx.request_repaint_after(Duration::from_millis(150)); // live file count
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("File Organizer");
                ui.add_space(24.0);
                for (tab, name) in [
                    (Tab::Sort, "Sort"),
                    (Tab::Duplicates, "Duplicates"),
                    (Tab::BigOld, "Big and old"),
                    (Tab::Rename, "Rename"),
                ] {
                    ui.selectable_value(&mut self.tab, tab, egui::RichText::new(name).size(17.0));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(self.settings_open, "⚙ Settings")
                        .clicked()
                    {
                        self.settings_open = !self.settings_open;
                        self.settings_draft = self.settings.clone();
                        self.autostart = organizer_core::autostart::status();
                    }
                    if ui
                        .button("Start background sorting")
                        .on_hover_text(
                            "Sorts finished downloads in Downloads automatically (tray icon).",
                        )
                        .clicked()
                    {
                        self.start_background();
                    }
                });
            });
            ui.add_space(4.0);
        });

        egui::Panel::bottom("bottom").show(ui, |ui| self.status_bar(ui));

        egui::Panel::right("history")
            .default_size(280.0)
            .resizable(true)
            .show(ui, |ui| self.history_panel(ui, &ctx));

        egui::CentralPanel::default_margins().show(ui, |ui| match self.tab {
            Tab::Sort => tabs::sort_tab(self, ui, &ctx),
            Tab::Duplicates => tabs::dupes_tab(self, ui, &ctx),
            Tab::BigOld => tabs::bigold_tab(self, ui, &ctx),
            Tab::Rename => tabs::rename_tab(self, ui, &ctx),
        });

        self.settings_window(&ctx);
        self.confirm_dialog(&ctx);
    }
}

impl App {
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        if let Some(job) = &self.job {
            let mut cancel = false;
            ui.horizontal(|ui| {
                ui.spinner();
                let secs = job.started.elapsed().as_secs();
                match &job.progress {
                    Some(p) => {
                        let files = p.files.load(std::sync::atomic::Ordering::Relaxed);
                        let hashed = p.hashed.load(std::sync::atomic::Ordering::Relaxed);
                        let extra = if hashed > 0 {
                            format!(" · {hashed} compared in full")
                        } else {
                            String::new()
                        };
                        ui.label(format!(
                            "{}…  {files} files checked{extra} · {secs} s",
                            job.label
                        ));
                        cancel = ui.button("Cancel").clicked();
                    }
                    None => {
                        ui.label(format!("{}…  {secs} s", job.label));
                    }
                }
            });
            if cancel {
                if let Some(p) = &job.progress {
                    p.cancel();
                }
            }
        } else if let Some(msg) = &self.message {
            ui.colored_label(RED, msg);
        }
        if let Some((label, r)) = &self.last {
            let text = if label == "Renamed" {
                r.summary().replacen("moved", "renamed", 1)
            } else if r.recycled.is_empty() && label != "Removed" {
                r.summary()
            } else {
                r.recycle_summary()
            };
            ui.strong(format!("{label}: {text}"));
            if !r.skipped.is_empty() || !r.errors.is_empty() {
                egui::CollapsingHeader::new("Details").show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(140.0)
                        .show(ui, |ui| {
                            for (p, why) in &r.skipped {
                                ui.label(format!("Skipped  {}  —  {why}", p.display()));
                            }
                            for (p, why) in &r.errors {
                                ui.colored_label(RED, format!("Error  {}  —  {why}", p.display()));
                            }
                        });
                });
            }
        } else if self.job.is_none() && self.message.is_none() {
            if let Some(roots) = &self.roots {
                ui.weak(format!(
                    "Folders: {}   ·   {}   ·   {}",
                    roots.downloads.display(),
                    roots.desktop.display(),
                    roots.documents.display()
                ));
            }
        }
        ui.add_space(4.0);
    }

    fn history_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(6.0);
        ui.heading("History");
        ui.separator();
        let mut undo = None;
        let busy = self.busy();
        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.history.is_empty() {
                ui.weak("Nothing done yet.");
            }
            for b in &self.history {
                let time = b.time.get(..16).unwrap_or(&b.time).replace('T', " ");
                if !b.recycled.is_empty() {
                    ui.label(time);
                    ui.weak(format!(
                        "{} sent to the Recycle Bin",
                        plural(b.recycled.len(), "file")
                    ));
                    ui.weak("Restore them from the Recycle Bin (right-click → Restore).");
                    egui::CollapsingHeader::new("Old places")
                        .id_salt(("rec", b.batch))
                        .show(ui, |ui| {
                            for e in &b.recycled {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(e.old.display().to_string()).small(),
                                    )
                                    .wrap(),
                                );
                            }
                        });
                    ui.separator();
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        let (what, who) = match b.tag.as_str() {
                            TAG_RENAME => ("renamed", ""),
                            TAG_AUTO => ("moved", " · background"),
                            _ => ("moved", ""),
                        };
                        ui.label(format!("{time}{who}"));
                        let left = b.remaining.len();
                        if left == 0 {
                            ui.weak(format!("{} {what} · undone", plural(b.total, "file")));
                        } else {
                            ui.weak(format!(
                                "{} {what} · {left} can be undone",
                                plural(b.total, "file")
                            ));
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !busy && !b.remaining.is_empty(),
                                egui::Button::new("Undo"),
                            )
                            .clicked()
                        {
                            undo = Some(b.batch);
                        }
                    });
                });
                ui.separator();
            }
        });
        if let Some(b) = undo {
            self.start_undo(ctx, Some(b));
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        use organizer_core::config::{
            BIG_FILE_MB_RANGE, MIN_DUPLICATE_KB_MAX, OLD_DAYS_RANGE, RECENT_DAYS_MAX,
        };
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut save = false;
        let mut autostart_change = None;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 60.0))
            .show(ctx, |ui| {
                let d = &mut self.settings_draft;
                egui::Grid::new("settings")
                    .num_columns(2)
                    .spacing([16.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Sort: leave Desktop / Documents files changed in the last");
                        ui.add(
                            egui::DragValue::new(&mut d.recent_days)
                                .range(0..=RECENT_DAYS_MAX)
                                .suffix(" days"),
                        );
                        ui.end_row();
                        ui.label("Big files: at least");
                        ui.add(
                            egui::DragValue::new(&mut d.big_file_mb)
                                .range(BIG_FILE_MB_RANGE.0..=BIG_FILE_MB_RANGE.1)
                                .suffix(" MB"),
                        );
                        ui.end_row();
                        ui.label("Old files: not changed for");
                        ui.add(
                            egui::DragValue::new(&mut d.old_days)
                                .range(OLD_DAYS_RANGE.0..=OLD_DAYS_RANGE.1)
                                .suffix(" days"),
                        );
                        ui.end_row();
                        ui.label("Duplicates: ignore files smaller than");
                        ui.add(
                            egui::DragValue::new(&mut d.min_duplicate_kb)
                                .range(0..=MIN_DUPLICATE_KB_MAX)
                                .suffix(" KB"),
                        );
                        ui.end_row();
                    });
                ui.add_space(6.0);
                use organizer_core::autostart::Status;
                let mut autostart = self.autostart != Status::Off;
                if ui
                    .checkbox(
                        &mut autostart,
                        "Start with Windows (background sorting of Downloads)",
                    )
                    .changed()
                {
                    autostart_change = Some(autostart);
                }
                match &self.autostart {
                    Status::Off => {}
                    Status::ThisCopy(p) => {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("Starts {}", p.display())).weak(),
                            )
                            .wrap(),
                        );
                    }
                    Status::OtherCopy(p) => {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!(
                                    "Points to another copy: {}",
                                    p.display()
                                ))
                                .color(RED),
                            )
                            .wrap(),
                        );
                        if ui.button("Use this copy").clicked() {
                            autostart_change = Some(true);
                        }
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    save = ui.button("Save").clicked();
                    if ui.button("Defaults").clicked() {
                        let cats = std::mem::take(&mut d.categories);
                        *d = Settings {
                            categories: cats,
                            ..Settings::default()
                        };
                    }
                });
                ui.weak("Takes effect on the next scan.");
            });
        if let Some(on) = autostart_change {
            self.set_autostart(on);
        }
        if save {
            let draft = self.settings_draft.sanitized();
            match &self.settings_path {
                // Keep the pause switch the tray may have changed meanwhile.
                Some(path) => match Settings::update(path, |s| {
                    let paused = s.background_paused;
                    *s = draft.clone();
                    s.background_paused = paused;
                }) {
                    Ok(saved) => {
                        self.settings = saved;
                        self.settings_open = false;
                        self.message = None;
                    }
                    Err(e) => self.message = Some(format!("Settings not saved: {e}")),
                },
                None => self.message = Some("Settings not saved: no app-data folder.".into()),
            }
        }
        if !open {
            self.settings_open = false;
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending else { return };
        let files = pending.files();
        let total: u64 = files.iter().map(|f| f.size).sum();
        let mut go = false;
        let mut cancel = false;
        let resp = egui::Modal::new(egui::Id::new("confirm-remove")).show(ctx, |ui| {
            ui.set_max_width(720.0);
            ui.heading(format!(
                "Send {} to the Recycle Bin?",
                plural(files.len(), "file")
            ));
            ui.label(format!(
                "These files will go ({}). You can restore them from the Recycle Bin.",
                human_size(total)
            ));
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for f in &files {
                        ui.label(format!("{}   ({})", f.path.display(), human_size(f.size)));
                    }
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                go = ui
                    .button(
                        egui::RichText::new(format!("🗑 Send {} to the Recycle Bin", files.len()))
                            .color(RED),
                    )
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if go && !self.busy() {
            self.start_removal(ctx);
        } else if cancel || resp.should_close() {
            self.pending = None;
        }
    }
}

pub fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// "C:\Users\me\Downloads\Images\2026-10" -> "Downloads\Images\2026-10".
pub fn short(dir: &Path, roots: &Roots) -> String {
    for base in [&roots.downloads, &roots.desktop, &roots.documents] {
        if let Some(name) = base.file_name() {
            if let Ok(rest) = dir.strip_prefix(base) {
                if rest.as_os_str().is_empty() {
                    return name.to_string_lossy().to_string();
                }
                return PathBuf::from(name).join(rest).display().to_string();
            }
        }
    }
    dir.display().to_string()
}

pub fn human_size(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Opens Explorer with the file selected.
pub fn show_in_folder(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Full path: a bare "explorer.exe" would be searched in the app's
        // own folder first.
        let explorer = organizer_core::paths::windows_dir()?.join("explorer.exe");
        std::process::Command::new(explorer)
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(windows))]
    {
        let dir = path.parent().unwrap_or(path);
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map(|_| ())
    }
}
