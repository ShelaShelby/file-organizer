//! File Organizer window. All file logic lives in `organizer-core`;
//! this file only draws the window and starts background jobs.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use organizer_core::journal::BatchReport;
use organizer_core::paths::app_data_dir;
use organizer_core::{execute, scan, BatchSummary, Journal, Plan, Roots, ScanOptions, Settings};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::SystemTime;

mod icon;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("File Organizer")
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([700.0, 400.0])
            .with_icon(icon::icon()),
        ..Default::default()
    };
    eframe::run_native("File Organizer", options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
}

enum JobResult {
    Scanned(Result<Plan, String>),
    Done(Result<BatchReport, String>, &'static str),
}

struct App {
    roots: Option<Roots>,
    settings: Settings,
    journal: Option<Journal>,
    plan: Option<Plan>,
    checked: Vec<bool>,
    history: Vec<BatchSummary>,
    last: Option<(String, BatchReport)>,
    message: Option<String>,
    job: Option<Receiver<JobResult>>,
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
        let roots = Roots::from_system().and_then(|r| r.validate().map(|_| r)).map_err(|e| problems.push(e.to_string())).ok();
        let (settings, journal) = match app_data_dir() {
            Ok(dir) => {
                let (s, warn) = Settings::load_or_create(&dir.join("settings.json"));
                problems.extend(warn);
                (s, Some(Journal::new(dir.join(organizer_core::journal::JOURNAL_FILE))))
            }
            Err(e) => {
                problems.push(format!("cannot find the app-data folder: {e}"));
                (Settings::default(), None)
            }
        };
        let mut app = App {
            roots,
            settings,
            journal,
            plan: None,
            checked: Vec::new(),
            history: Vec::new(),
            last: None,
            message: (!problems.is_empty()).then(|| problems.join("\n")),
            job: None,
        };
        app.refresh_history();
        app
    }

    fn refresh_history(&mut self) {
        if let Some(j) = &self.journal {
            match j.history() {
                Ok(h) => self.history = h,
                Err(e) => self.message = Some(format!("cannot read the journal: {e}")),
            }
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        let Some(roots) = self.roots.clone() else { return };
        let settings = self.settings.clone();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let opts = ScanOptions::new(&settings, SystemTime::now());
            let r = scan(&roots, &settings, &opts).map_err(|e| e.to_string());
            let _ = tx.send(JobResult::Scanned(r));
            ctx.request_repaint();
        });
        self.last = None;
        self.job = Some(rx);
    }

    fn start_confirm(&mut self, ctx: &egui::Context) {
        let (Some(roots), Some(plan), Some(journal)) = (self.roots.clone(), self.plan.take(), &self.journal) else {
            return;
        };
        let checked = std::mem::take(&mut self.checked);
        let moves: Vec<_> = plan.moves.into_iter().zip(checked).filter(|(_, c)| *c).map(|(m, _)| m).collect();
        let journal = Journal::new(journal.path());
        let settings = self.settings.clone();
        self.spawn_batch(ctx, "Organised", move || {
            let opts = ScanOptions::new(&settings, SystemTime::now());
            Ok(execute(&roots, &moves, &journal, &opts))
        });
    }

    fn start_undo(&mut self, ctx: &egui::Context, batch: Option<u64>) {
        let Some(journal) = &self.journal else { return };
        let journal = Journal::new(journal.path());
        self.plan = None;
        self.checked.clear();
        self.spawn_batch(ctx, "Undo", move || match batch {
            Some(b) => journal.undo(b).map_err(|e| e.to_string()),
            None => journal.undo_last().map_err(|e| e.to_string())?.ok_or_else(|| "Nothing to undo.".to_string()),
        });
    }

    fn spawn_batch(
        &mut self,
        ctx: &egui::Context,
        label: &'static str,
        work: impl FnOnce() -> Result<BatchReport, String> + Send + 'static,
    ) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(JobResult::Done(work(), label));
            ctx.request_repaint();
        });
        self.job = Some(rx);
    }

    fn poll_job(&mut self) {
        let Some(rx) = &self.job else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.job = None;
        match result {
            JobResult::Scanned(Ok(plan)) => {
                self.checked = vec![true; plan.moves.len()];
                self.plan = Some(plan);
            }
            JobResult::Done(Ok(report), label) => {
                self.last = Some((label.to_string(), report));
                self.refresh_history();
            }
            JobResult::Scanned(Err(e)) | JobResult::Done(Err(e), _) => self.message = Some(e),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_job();
        let ctx = ui.ctx().clone();

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("File Organizer");
                ui.add_space(20.0);
                let can = !self.busy() && self.roots.is_some();
                if ui.add_enabled(can, egui::Button::new("🔍 Scan")).clicked() {
                    self.start_scan(&ctx);
                }
                let n = self.checked.iter().filter(|c| **c).count();
                let can_confirm = can && self.plan.is_some() && n > 0 && self.journal.is_some();
                if ui.add_enabled(can_confirm, egui::Button::new(format!("✔ Confirm ({n})"))).clicked() {
                    self.start_confirm(&ctx);
                }
                let can_undo = !self.busy() && self.history.iter().any(|b| !b.remaining.is_empty());
                if ui.add_enabled(can_undo, egui::Button::new("Undo last")).clicked() {
                    self.start_undo(&ctx, None);
                }
                if self.busy() {
                    ui.spinner();
                }
            });
            ui.add_space(6.0);
        });

        egui::Panel::bottom("bottom").show(ui, |ui| {
            ui.add_space(4.0);
            if let Some(msg) = &self.message {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(240, 120, 110), msg);
                });
            }
            if let Some((label, r)) = &self.last {
                ui.strong(format!("{label}: {}", r.summary()));
                if !r.skipped.is_empty() || !r.errors.is_empty() {
                    egui::CollapsingHeader::new("Details").show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                            for (p, why) in &r.skipped {
                                ui.label(format!("Skipped  {}  —  {why}", p.display()));
                            }
                            for (p, why) in &r.errors {
                                ui.colored_label(egui::Color32::from_rgb(240, 120, 110), format!("Error  {}  —  {why}", p.display()));
                            }
                        });
                    });
                }
            } else if let Some(roots) = &self.roots {
                ui.weak(format!(
                    "Folders: {}   ·   {}   ·   {}",
                    roots.downloads.display(),
                    roots.desktop.display(),
                    roots.documents.display()
                ));
            }
            ui.add_space(4.0);
        });

        egui::Panel::right("history").default_size(270.0).resizable(true).show(ui, |ui| {
            ui.add_space(6.0);
            ui.heading("History");
            ui.separator();
            let mut undo = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.history.is_empty() {
                    ui.weak("Nothing moved yet.");
                }
                for b in &self.history {
                    let time = b.time.get(..16).unwrap_or(&b.time).replace('T', " ");
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(time);
                            let left = b.remaining.len();
                            if left == 0 {
                                ui.weak(format!("{} files · undone", b.total));
                            } else {
                                ui.weak(format!("{} files · {left} can be undone", b.total));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(!self.busy() && !b.remaining.is_empty(), egui::Button::new("Undo")).clicked() {
                                undo = Some(b.batch);
                            }
                        });
                    });
                    ui.separator();
                }
            });
            if let Some(b) = undo {
                self.start_undo(&ctx, Some(b));
            }
        });

        egui::CentralPanel::default_margins().show(ui, |ui| self.preview(ui));
    }
}

impl App {
    fn preview(&mut self, ui: &mut egui::Ui) {
        let (Some(plan), Some(roots)) = (&self.plan, &self.roots) else {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.heading("Press Scan to see what would be tidied up.");
                ui.weak("Nothing is moved until you press Confirm. Every move can be undone.");
            });
            return;
        };
        let total: u64 = plan.moves.iter().zip(&self.checked).filter(|(_, c)| **c).map(|(m, _)| m.size).sum();
        ui.horizontal(|ui| {
            ui.strong(format!("{} files to move ({})", plan.moves.len(), human_size(total)));
            if ui.small_button("Select all").clicked() {
                self.checked.iter_mut().for_each(|c| *c = true);
            }
            if ui.small_button("Select none").clicked() {
                self.checked.iter_mut().for_each(|c| *c = false);
            }
        });
        if !plan.skipped.is_empty() || !plan.errors.is_empty() {
            egui::CollapsingHeader::new(format!("Left alone ({})", plan.skipped.len() + plan.errors.len())).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("skipped").max_height(120.0).show(ui, |ui| {
                    for (p, r) in &plan.skipped {
                        ui.weak(format!("{}  —  {r}", p.display()));
                    }
                    for (p, e) in &plan.errors {
                        ui.colored_label(egui::Color32::from_rgb(240, 120, 110), format!("{}  —  {e}", p.display()));
                    }
                });
            });
        }
        if plan.moves.is_empty() {
            ui.add_space(20.0);
            ui.label("All tidy — nothing to move.");
            return;
        }
        ui.add_space(4.0);
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let checked = &mut self.checked;
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(28.0))
            .column(Column::initial(240.0).at_least(80.0).clip(true))
            .column(Column::initial(100.0).at_least(60.0))
            .column(Column::remainder().at_least(120.0).clip(true))
            .column(Column::exact(80.0))
            .header(row_h, |mut h| {
                for t in ["", "File", "From", "To", "Size"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, plan.moves.len(), |mut row| {
                    let i = row.index();
                    let m = &plan.moves[i];
                    row.col(|ui| {
                        ui.checkbox(&mut checked[i], "");
                    });
                    row.col(|ui| {
                        ui.label(m.source.file_name().map(|n| n.to_string_lossy()).unwrap_or_default())
                            .on_hover_text(m.source.display().to_string());
                    });
                    row.col(|ui| {
                        ui.label(m.root.label());
                    });
                    row.col(|ui| {
                        ui.label(short(&m.target, roots)).on_hover_text(m.target.display().to_string());
                    });
                    row.col(|ui| {
                        ui.label(human_size(m.size));
                    });
                });
            });
    }
}

/// "C:\Users\me\Downloads\Images\2026-10" -> "Downloads\Images\2026-10".
fn short(dir: &Path, roots: &Roots) -> String {
    for base in [&roots.downloads, &roots.documents] {
        if let Some(name) = base.file_name() {
            if let Ok(rest) = dir.strip_prefix(base) {
                return PathBuf::from(name).join(rest).display().to_string();
            }
        }
    }
    dir.display().to_string()
}

fn human_size(b: u64) -> String {
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
