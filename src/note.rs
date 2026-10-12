//! `--note <batch>`: the small popup after background sorting. Started by
//! the tray process as a short-lived child (eframe cannot reopen windows in
//! one process); shows near the bottom-right corner, offers Undo and closes
//! itself after about 10 seconds.

use eframe::egui;
use organizer_core::autosort::note_text;
use organizer_core::journal::{Journal, JOURNAL_FILE};
use organizer_core::paths::app_data_dir;
use organizer_core::Roots;
use std::time::{Duration, Instant};

const SIZE: [f32; 2] = [430.0, 120.0];
const SHOW_FOR: Duration = Duration::from_secs(10);

struct Note {
    journal: Option<Journal>,
    roots: Option<Roots>,
    batch: u64,
    text: String,
    can_undo: bool,
    close_at: Instant,
}

impl Note {
    fn load(batch: u64) -> Note {
        let journal = app_data_dir().ok().map(|d| Journal::new(d.join(JOURNAL_FILE)));
        let summary =
            journal.as_ref().and_then(|j| j.history().ok()).and_then(|h| h.into_iter().find(|b| b.batch == batch));
        let roots = Roots::from_system().ok();
        let (text, can_undo) = match (&summary, &roots) {
            (Some(s), Some(r)) if !s.remaining.is_empty() => {
                let moved: Vec<_> = s.remaining.iter().map(|e| (e.old.clone(), e.new.clone())).collect();
                (note_text(r, &moved), true)
            }
            (Some(s), _) if s.total > 0 => {
                (format!("Undone: {} back in Downloads.", crate::plural(s.total, "file")), false)
            }
            _ => ("Nothing to show.".to_string(), false),
        };
        Note { journal, roots, batch, text, can_undo, close_at: Instant::now() + SHOW_FOR }
    }
}

impl eframe::App for Note {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        if ctx.is_pointer_over_egui() {
            self.close_at = self.close_at.max(now + Duration::from_secs(3)); // keep open while the mouse is on it
        }
        if now >= self.close_at {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(500));
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("File Organizer");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Close").clicked() {
                        self.close_at = now;
                    }
                });
            });
            ui.add(egui::Label::new(&self.text).wrap());
            ui.add_space(4.0);
            if self.can_undo && ui.button("Undo").clicked() {
                if let (Some(j), Some(roots)) = (&self.journal, &self.roots) {
                    self.text = match j.undo(self.batch, roots) {
                        Ok(r) if r.skipped.is_empty() && r.errors.is_empty() => {
                            format!("Undone: {} back in Downloads.", crate::plural(r.moved.len(), "file"))
                        }
                        Ok(r) => format!("Undo: {}", r.summary()),
                        Err(e) => format!("Undo failed: {e}"),
                    };
                }
                self.can_undo = false;
                self.close_at = Instant::now() + Duration::from_secs(4);
            }
        });
    }
}

/// Bottom-right corner of the primary monitor's work area (above the
/// taskbar), in points, using that monitor's real DPI.
fn corner_position() -> Option<egui::Pos2> {
    #[cfg(windows)]
    {
        use organizer_core::window::{note_position, PxRect};
        use windows_sys::Win32::Foundation::{POINT, RECT};
        use windows_sys::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
        };
        use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
        // SAFETY: plain Win32 calls; MONITORINFO has its size set.
        unsafe {
            let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            if mon.is_null() {
                return None;
            }
            let empty = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                rcMonitor: empty,
                rcWork: empty,
                dwFlags: 0,
            };
            if GetMonitorInfoW(mon, &mut info) == 0 {
                return None;
            }
            let (mut dx, mut dy) = (0u32, 0u32);
            let scale = if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) >= 0 && dx > 0 {
                dx as f32 / 96.0
            } else {
                1.0
            };
            let w = info.rcWork;
            let work = PxRect { left: w.left, top: w.top, right: w.right, bottom: w.bottom };
            let (x, y) = note_position(work, scale, SIZE);
            Some(egui::pos2(x, y))
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn run(batch: u64) -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(organizer_core::window::NOTE_TITLE)
        .with_inner_size(SIZE)
        .with_resizable(false)
        .with_decorations(false)
        .with_always_on_top()
        .with_taskbar(false)
        .with_active(false)
        .with_icon(crate::icon::icon());
    if let Some(p) = corner_position() {
        viewport = viewport.with_position(p);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "File Organizer note",
        options,
        Box::new(move |cc| {
            let theme = app_data_dir()
                .ok()
                .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
                .map(|t| organizer_core::Settings::from_json(&t).0.theme)
                .unwrap_or_default();
            crate::load_fonts(&cc.egui_ctx);
            cc.egui_ctx.set_theme(crate::theme_preference(theme));
            Ok(Box::new(Note::load(batch)))
        }),
    )
}
