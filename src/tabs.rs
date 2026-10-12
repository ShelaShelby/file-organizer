//! The tabs: Sort, Duplicates, Big and old, Rename.

use crate::{human_size, plural, short, show_in_folder, App, BigList, Pending, RED};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use organizer_core::dupes::{check_choices, space_freed};
use organizer_core::tree::{apply_order, date_text, sort_order, Skipped, SortKey};
use organizer_core::ScannedFile;

fn row_height(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body) + 8.0
}

fn intro(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.heading(title);
        ui.weak(sub);
    });
}

fn file_name(f: &ScannedFile) -> String {
    f.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn folder(app: &App, f: &ScannedFile) -> String {
    let dir = f.path.parent().unwrap_or(&f.path);
    match &app.roots {
        Some(r) => short(dir, r),
        None => dir.display().to_string(),
    }
}

fn skipped_list(ui: &mut egui::Ui, s: &Skipped) {
    let n = s.count + s.errors.len();
    if n == 0 {
        return;
    }
    egui::CollapsingHeader::new(format!("Left alone ({n})")).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("skipped").max_height(120.0).show(ui, |ui| {
            for (p, r) in &s.items {
                ui.weak(format!("{}  —  {r}", p.display()));
            }
            if s.count > s.items.len() {
                ui.weak(format!("… and {} more", s.count - s.items.len()));
            }
            for (p, e) in &s.errors {
                ui.colored_label(RED, format!("{}  —  {e}", p.display()));
            }
        });
    });
}

/// A clickable column header; returns true when clicked. Shows ▲/▼ next
/// to the column the list is sorted by.
fn sort_header(ui: &mut egui::Ui, title: &str, key: SortKey, current: Option<(SortKey, bool)>) -> bool {
    let text = match current {
        Some((k, asc)) if k == key => format!("{title} {}", if asc { "^" } else { "v" }),
        _ => title.to_string(),
    };
    ui.add(egui::Button::new(egui::RichText::new(text).strong()).frame(false)).on_hover_text("Click to sort").clicked()
}

/// One header cell: clickable when it has a sort key.
fn header_cell(
    h: &mut egui_extras::TableRow<'_, '_>,
    title: &str,
    key: Option<SortKey>,
    current: Option<(SortKey, bool)>,
    clicked: &mut Option<SortKey>,
) {
    h.col(|ui| match key {
        Some(k) => {
            if sort_header(ui, title, k, current) {
                *clicked = Some(k);
            }
        }
        None => {
            ui.strong(title);
        }
    });
}

// Column widths (points). The widest table (Big and old) must fit the
// content area at the smallest window size; see the test below.
const TICK_W: f32 = 28.0;
const KEEP_W: f32 = 60.0;
const NAME_MIN: f32 = 90.0;
const FOLDER_MIN: f32 = 80.0;
const SIZE_W: f32 = 75.0;
const DATE_W: f32 = 95.0;
const ACTIONS_W: f32 = 180.0;

/// Next sort state after clicking `key`: same column flips the direction;
/// a new column starts ascending (sizes and dates start with the biggest /
/// newest first).
fn next_sort(current: Option<(SortKey, bool)>, key: SortKey) -> (SortKey, bool) {
    match current {
        Some((k, asc)) if k == key => (key, !asc),
        _ => (key, key == SortKey::Name),
    }
}

// ---------------------------------------------------------------- Sort

pub fn sort_tab(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let can = !app.busy() && app.roots.is_some();
    ui.horizontal(|ui| {
        if ui.add_enabled(can, egui::Button::new("Scan")).clicked() {
            app.start_scan(ctx);
        }
        let n = app.checked.iter().filter(|c| **c).count();
        let can_confirm = can && app.plan.is_some() && n > 0 && app.journal.is_some();
        if ui.add_enabled(can_confirm, egui::Button::new(format!("Confirm ({n})"))).clicked() {
            app.start_confirm(ctx);
        }
        if ui.add_enabled(!app.busy() && app.journal.is_some(), egui::Button::new("Undo last")).clicked() {
            app.start_undo(ctx, None);
        }
    });
    ui.separator();
    let (Some(plan), Some(roots)) = (&app.plan, &app.roots) else {
        intro(
            ui,
            "Press Scan to see what would be tidied up.",
            "Nothing is moved until you press Confirm. Every move can be undone.",
        );
        return;
    };
    let total: u64 = plan.moves.iter().zip(&app.checked).filter(|(_, c)| **c).map(|(m, _)| m.size).sum();
    ui.horizontal(|ui| {
        ui.strong(format!("{} to move ({})", plural(plan.moves.len(), "file"), human_size(total)));
        if ui.small_button("Select all").clicked() {
            app.checked.iter_mut().for_each(|c| *c = true);
        }
        if ui.small_button("Select none").clicked() {
            app.checked.iter_mut().for_each(|c| *c = false);
        }
    });
    let by_folder = organizer_core::plan::summary_by_folder(plan, roots);
    if !by_folder.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(by_folder).weak()).wrap());
    }
    if !plan.skipped.is_empty() || !plan.errors.is_empty() {
        egui::CollapsingHeader::new(format!("Left alone ({})", plan.skipped.len() + plan.errors.len())).show(
            ui,
            |ui| {
                egui::ScrollArea::vertical().id_salt("skipped").max_height(120.0).show(ui, |ui| {
                    for (p, r) in &plan.skipped {
                        ui.weak(format!("{}  —  {r}", p.display()));
                    }
                    for (p, e) in &plan.errors {
                        ui.colored_label(RED, format!("{}  —  {e}", p.display()));
                    }
                });
            },
        );
    }
    if plan.moves.is_empty() {
        ui.add_space(20.0);
        ui.label("All tidy — nothing to move.");
        return;
    }
    let row_h = row_height(ui);
    let checked = &mut app.checked;
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
                    ui.label(roots.label(m.root));
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

// ---------------------------------------------------------------- Duplicates

pub fn dupes_tab(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let can = !app.busy() && app.roots.is_some();
    let choices = app.dupes.as_ref().map(|d| d.choices()).unwrap_or_default();
    let problem = check_choices(&choices);
    let to_go: usize = choices.iter().map(|c| c.to_remove().count()).sum();
    ui.horizontal(|ui| {
        if ui.add_enabled(can, egui::Button::new("Find duplicates")).clicked() {
            app.start_dupes(ctx);
        }
        let can_confirm = can && app.journal.is_some() && problem.is_none() && to_go > 0;
        if ui.add_enabled(can_confirm, egui::Button::new(format!("Confirm ({to_go})"))).clicked() {
            app.pending = Some(Pending::Duplicates(choices.clone()));
        }
        ui.weak(format!("Ignores files under {} KB.", app.settings.min_duplicate_kb));
    });
    ui.separator();
    let Some(view) = &mut app.dupes else {
        intro(
            ui,
            "Find files with exactly the same content.",
            "Downloads, Desktop, Documents and your extra folders, with all sub-folders. Nothing goes until you press Confirm, and then only to the Recycle Bin.",
        );
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.strong(format!(
            "{} of identical files · {} would be freed",
            plural(view.scan.groups.len(), "group"),
            human_size(space_freed(&choices))
        ));
        if let Some(p) = &problem {
            ui.colored_label(RED, p);
        }
    });
    skipped_list(ui, &view.scan.skipped);
    if view.scan.groups.is_empty() {
        ui.add_space(20.0);
        ui.label(format!("No duplicates found ({} checked).", plural(view.scan.files_checked as usize, "file")));
        return;
    }
    ui.weak(
        "The copy with the oldest date is kept. Tick \"Keep\" to choose another; untick a group to leave it alone.",
    );

    // One table row per group header and per file.
    let rows: Vec<(usize, Option<usize>)> = view
        .scan
        .groups
        .iter()
        .enumerate()
        .flat_map(|(g, grp)| std::iter::once((g, None)).chain((0..grp.files.len()).map(move |f| (g, Some(f)))))
        .collect();
    let row_h = row_height(ui);
    let roots = app.roots.clone();
    let current = view.sort;
    let mut clicked = None;
    TableBuilder::new(ui)
        .striped(false)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(KEEP_W))
        .column(Column::initial(260.0).at_least(NAME_MIN).clip(true))
        .column(Column::remainder().at_least(FOLDER_MIN).clip(true))
        .column(Column::exact(SIZE_W))
        .column(Column::exact(DATE_W))
        .header(row_h, |mut h| {
            for (t, key) in [
                ("Keep", None),
                ("File", Some(SortKey::Name)),
                ("Folder", None),
                ("Size", Some(SortKey::Size)),
                ("Last changed", Some(SortKey::Modified)),
            ] {
                header_cell(&mut h, t, key, current, &mut clicked);
            }
        })
        .body(|body| {
            body.rows(row_h, rows.len(), |mut row| {
                let (g, f) = rows[row.index()];
                let group = &view.scan.groups[g];
                match f {
                    None => {
                        let included = &mut view.include[g];
                        let keeps = view.keep[g].iter().filter(|k| **k).count();
                        let frees = group.size * (group.files.len() - keeps) as u64;
                        row.col(|ui| {
                            ui.checkbox(included, "");
                        });
                        row.col(|ui| {
                            ui.strong(format!("{} copies", group.files.len()));
                        });
                        row.col(|ui| {
                            let each = format!("{} each", human_size(group.size));
                            if !*included {
                                ui.weak(format!("{each} · left alone"));
                            } else if keeps == 0 {
                                ui.colored_label(RED, "keeps no copy!");
                            } else {
                                ui.weak(format!("{each} · frees {}", human_size(frees)));
                            }
                        });
                        row.col(|_| {});
                        row.col(|_| {});
                    }
                    Some(i) => {
                        let file = &group.files[i];
                        let included = view.include[g];
                        row.col(|ui| {
                            ui.add_space(14.0);
                            ui.add_enabled(included, egui::Checkbox::without_text(&mut view.keep[g][i]));
                        });
                        row.col(|ui| {
                            let name = file_name(file);
                            let gone = included && !view.keep[g][i];
                            let text = if gone {
                                egui::RichText::new(name).strikethrough().weak()
                            } else {
                                egui::RichText::new(name)
                            };
                            ui.label(text).on_hover_text(file.path.display().to_string());
                        });
                        row.col(|ui| {
                            let dir = file.path.parent().unwrap_or(&file.path);
                            let s = roots.as_ref().map(|r| short(dir, r)).unwrap_or_else(|| dir.display().to_string());
                            ui.label(s).on_hover_text(dir.display().to_string());
                        });
                        row.col(|ui| {
                            ui.label(human_size(file.size));
                        });
                        row.col(|ui| {
                            ui.label(date_text(file.modified));
                        });
                    }
                }
            });
        });
    if let Some(k) = clicked {
        let (key, asc) = next_sort(current, k);
        organizer_core::dupes::sort_groups(&mut view.scan.groups, &mut view.keep, &mut view.include, key, asc);
        view.sort = Some((key, asc));
    }
}

// ---------------------------------------------------------------- Big and old

pub fn bigold_tab(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let can = !app.busy() && app.roots.is_some();
    ui.horizontal(|ui| {
        if ui.add_enabled(can, egui::Button::new("Find big and old files")).clicked() {
            app.start_bigold(ctx);
        }
        ui.weak(format!(
            "Big: {} MB or more · Old: not changed for {} days",
            app.settings.big_file_mb, app.settings.old_days
        ));
    });
    ui.separator();
    let Some(view) = &mut app.bigold else {
        intro(
            ui,
            "See your biggest and oldest files.",
            "A list only — nothing happens unless you send a file to the Recycle Bin yourself.",
        );
        return;
    };
    ui.horizontal(|ui| {
        ui.selectable_value(&mut view.list, BigList::Big, format!("Big files ({})", view.scan.big.len()));
        ui.selectable_value(&mut view.list, BigList::Old, format!("Old files ({})", view.scan.old.len()));
        ui.selectable_value(
            &mut view.list,
            BigList::Installers,
            format!("Old installers ({})", view.installers.files.len()),
        );
    });
    if view.list == BigList::Installers {
        installers_list(app, ui);
        return;
    }
    let Some(view) = &mut app.bigold else { return };
    if view.list == BigList::Old {
        ui.weak(
            "\"Old\" means not changed for a long time (Windows does not reliably record when a file was last opened).",
        );
    }
    skipped_list(ui, &view.scan.skipped);
    let list = if view.list == BigList::Old { view.scan.old.clone() } else { view.scan.big.clone() };
    if list.is_empty() {
        ui.add_space(20.0);
        ui.label("Nothing on this list.");
        return;
    }
    let row_h = row_height(ui);
    let mut recycle: Option<ScannedFile> = None;
    let mut open_err = None;
    let current = view.sort;
    let busy = app.busy();
    let folders: Vec<String> = list.iter().map(|f| folder(app, f)).collect();
    let mut clicked = None;
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(240.0).at_least(NAME_MIN).clip(true))
        .column(Column::remainder().at_least(FOLDER_MIN).clip(true))
        .column(Column::exact(SIZE_W))
        .column(Column::exact(DATE_W))
        .column(Column::exact(ACTIONS_W))
        .header(row_h, |mut h| {
            for (t, key) in [
                ("File", Some(SortKey::Name)),
                ("Folder", None),
                ("Size", Some(SortKey::Size)),
                ("Last changed", Some(SortKey::Modified)),
                ("", None),
            ] {
                header_cell(&mut h, t, key, current, &mut clicked);
            }
        })
        .body(|body| {
            body.rows(row_h, list.len(), |mut row| {
                let i = row.index();
                let f = &list[i];
                row.col(|ui| {
                    ui.label(file_name(f)).on_hover_text(f.path.display().to_string());
                });
                row.col(|ui| {
                    ui.label(&folders[i]);
                });
                row.col(|ui| {
                    ui.label(human_size(f.size));
                });
                row.col(|ui| {
                    ui.label(date_text(f.modified));
                });
                row.col(|ui| {
                    if ui.small_button("Open folder").clicked() {
                        if let Err(e) = show_in_folder(&f.path) {
                            open_err = Some(format!("Could not open Explorer: {e}"));
                        }
                    }
                    if ui.add_enabled(!busy, egui::Button::new("Recycle").small()).clicked() {
                        recycle = Some(f.clone());
                    }
                });
            });
        });
    if let (Some(k), Some(view)) = (clicked, &mut app.bigold) {
        let (key, asc) = next_sort(current, k);
        let files = if view.list == BigList::Old { &mut view.scan.old } else { &mut view.scan.big };
        let order = sort_order(files, key, asc);
        apply_order(files, &order);
        view.sort = Some((key, asc));
    }
    if let Some(f) = recycle {
        app.pending = Some(Pending::Files(vec![f]));
    }
    if open_err.is_some() {
        app.message = open_err;
    }
}

/// The "Old installers" list: tick boxes (none ticked at first), Select
/// all, total size and one button for the ticked files.
fn installers_list(app: &mut App, ui: &mut egui::Ui) {
    let busy = app.busy();
    let can_remove = !busy && app.journal.is_some();
    let installer_days = app.settings.installer_days;
    let folders: Vec<String> =
        app.bigold.as_ref().map(|v| v.installers.files.iter().map(|f| folder(app, f)).collect()).unwrap_or_default();
    let Some(view) = &mut app.bigold else { return };
    ui.weak(format!(
        "Setup files (.exe, .msi, .msix, ...) not changed for {installer_days} days, only at the top of Downloads and in Downloads\\Installers\\<month>."
    ));
    skipped_list(ui, &view.installers.skipped);
    let files = &view.installers.files;
    if files.is_empty() {
        ui.add_space(20.0);
        ui.label("No old installers found.");
        return;
    }
    let ticked: Vec<ScannedFile> =
        files.iter().zip(&view.installer_ticked).filter(|(_, t)| **t).map(|(f, _)| f.clone()).collect();
    let total: u64 = ticked.iter().map(|f| f.size).sum();
    let mut send = false;
    ui.horizontal(|ui| {
        ui.strong(format!("{} ticked ({})", plural(ticked.len(), "file"), human_size(total)));
        if ui.small_button("Select all").clicked() {
            view.installer_ticked.iter_mut().for_each(|t| *t = true);
        }
        if ui.small_button("Select none").clicked() {
            view.installer_ticked.iter_mut().for_each(|t| *t = false);
        }
        let button = egui::Button::new(egui::RichText::new("Send ticked to the Recycle Bin").color(RED));
        send = ui.add_enabled(can_remove && !ticked.is_empty(), button).clicked();
    });
    let row_h = row_height(ui);
    let ticks = &mut view.installer_ticked;
    let current = view.sort;
    let mut clicked = None;
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(TICK_W))
        .column(Column::initial(260.0).at_least(NAME_MIN).clip(true))
        .column(Column::remainder().at_least(FOLDER_MIN).clip(true))
        .column(Column::exact(SIZE_W))
        .column(Column::exact(DATE_W))
        .header(row_h, |mut h| {
            for (t, key) in [
                ("", None),
                ("File", Some(SortKey::Name)),
                ("Folder", None),
                ("Size", Some(SortKey::Size)),
                ("Last changed", Some(SortKey::Modified)),
            ] {
                header_cell(&mut h, t, key, current, &mut clicked);
            }
        })
        .body(|body| {
            body.rows(row_h, files.len(), |mut row| {
                let i = row.index();
                let f = &files[i];
                row.col(|ui| {
                    ui.checkbox(&mut ticks[i], "");
                });
                row.col(|ui| {
                    ui.label(file_name(f)).on_hover_text(f.path.display().to_string());
                });
                row.col(|ui| {
                    ui.label(&folders[i]);
                });
                row.col(|ui| {
                    ui.label(human_size(f.size));
                });
                row.col(|ui| {
                    ui.label(date_text(f.modified));
                });
            });
        });
    if let Some(k) = clicked {
        let (key, asc) = next_sort(current, k);
        let order = sort_order(&view.installers.files, key, asc);
        apply_order(&mut view.installers.files, &order);
        apply_order(&mut view.installer_ticked, &order);
        view.sort = Some((key, asc));
    }
    if send {
        app.pending = Some(Pending::Files(ticked));
    }
}

// ---------------------------------------------------------------- Rename

pub fn rename_tab(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let can = !app.busy() && app.roots.is_some();
    let n = app.rename_checked.iter().filter(|c| **c).count();
    ui.horizontal(|ui| {
        if ui.add_enabled(can, egui::Button::new("Find messy names")).clicked() {
            app.start_renames(ctx);
        }
        let can_confirm = can && app.renames.is_some() && n > 0 && app.journal.is_some();
        if ui.add_enabled(can_confirm, egui::Button::new(format!("Rename ticked ({n})"))).clicked() {
            app.start_rename_confirm(ctx);
        }
    });
    ui.separator();
    let Some(scan) = &app.renames else {
        intro(
            ui,
            "Tidy up junk in file names.",
            "Only \"photo (1) (2)\" counters, \"Copy of\" / \" - Copy\" and extra spaces. The extension never changes. Nothing is renamed until you press Rename, and every rename can be undone.",
        );
        return;
    };
    ui.horizontal(|ui| {
        ui.strong(format!("{} to rename", plural(scan.items.len(), "file")));
        if ui.small_button("Select all").clicked() {
            app.rename_checked.iter_mut().for_each(|c| *c = true);
        }
        if ui.small_button("Select none").clicked() {
            app.rename_checked.iter_mut().for_each(|c| *c = false);
        }
    });
    if !scan.conflicts.is_empty() {
        egui::CollapsingHeader::new(format!("Not renamed, name taken ({})", scan.conflicts.len())).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("conflicts").max_height(120.0).show(ui, |ui| {
                for (p, why) in &scan.conflicts {
                    ui.weak(format!("{}  —  {why}", p.display()));
                }
            });
        });
    }
    skipped_list(ui, &scan.skipped);
    if scan.items.is_empty() {
        ui.add_space(20.0);
        ui.label(format!("No messy names found ({} checked).", plural(scan.files_checked as usize, "file")));
        return;
    }
    let row_h = row_height(ui);
    let folders: Vec<String> = scan.items.iter().map(|i| folder(app, &i.file)).collect();
    let checked = &mut app.rename_checked;
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(28.0))
        .column(Column::initial(260.0).at_least(100.0).clip(true))
        .column(Column::initial(260.0).at_least(100.0).clip(true))
        .column(Column::remainder().at_least(120.0).clip(true))
        .header(row_h, |mut h| {
            for t in ["", "Old name", "New name", "Folder"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(row_h, scan.items.len(), |mut row| {
                let i = row.index();
                let item = &scan.items[i];
                row.col(|ui| {
                    ui.checkbox(&mut checked[i], "");
                });
                row.col(|ui| {
                    ui.label(item.old_name()).on_hover_text(item.file.path.display().to_string());
                });
                row.col(|ui| {
                    ui.label(format!("→ {}", item.new_name()));
                });
                row.col(|ui| {
                    ui.label(&folders[i])
                        .on_hover_text(item.file.path.parent().map(|p| p.display().to_string()).unwrap_or_default());
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widest_table_fits_the_smallest_window() {
        let content = organizer_core::window::layout(organizer_core::window::MIN_WINDOW[0]).content_width;
        let margins = 16.0 + 5.0 * 8.0; // panel margins + space between columns
        let big_old = NAME_MIN + FOLDER_MIN + SIZE_W + DATE_W + ACTIONS_W + margins;
        let dupes = KEEP_W + NAME_MIN + FOLDER_MIN + SIZE_W + DATE_W + margins;
        let installers = TICK_W + NAME_MIN + FOLDER_MIN + SIZE_W + DATE_W + margins;
        for w in [big_old, dupes, installers] {
            assert!(w <= content, "{w} > {content}");
        }
    }

    #[test]
    fn clicking_a_header_sorts_then_flips() {
        assert_eq!(next_sort(None, SortKey::Name), (SortKey::Name, true));
        assert_eq!(next_sort(None, SortKey::Size), (SortKey::Size, false));
        assert_eq!(next_sort(Some((SortKey::Size, false)), SortKey::Size), (SortKey::Size, true));
        assert_eq!(next_sort(Some((SortKey::Size, true)), SortKey::Modified), (SortKey::Modified, false));
    }
}
