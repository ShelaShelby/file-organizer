//! The tabs: Sort, Duplicates, Big and old, Rename.

use crate::{human_size, plural, short, show_in_folder, App, Pending, RED};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use organizer_core::dupes::{check_choices, space_freed};
use organizer_core::tree::{date_text, Skipped};
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
    f.path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
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
        egui::ScrollArea::vertical()
            .id_salt("skipped")
            .max_height(120.0)
            .show(ui, |ui| {
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

// ---------------------------------------------------------------- Sort

pub fn sort_tab(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let can = !app.busy() && app.roots.is_some();
    ui.horizontal(|ui| {
        if ui.add_enabled(can, egui::Button::new("🔍 Scan")).clicked() {
            app.start_scan(ctx);
        }
        let n = app.checked.iter().filter(|c| **c).count();
        let can_confirm = can && app.plan.is_some() && n > 0 && app.journal.is_some();
        if ui
            .add_enabled(can_confirm, egui::Button::new(format!("✔ Confirm ({n})")))
            .clicked()
        {
            app.start_confirm(ctx);
        }
        if ui
            .add_enabled(
                !app.busy() && app.journal.is_some(),
                egui::Button::new("Undo last"),
            )
            .clicked()
        {
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
    let total: u64 = plan
        .moves
        .iter()
        .zip(&app.checked)
        .filter(|(_, c)| **c)
        .map(|(m, _)| m.size)
        .sum();
    ui.horizontal(|ui| {
        ui.strong(format!(
            "{} to move ({})",
            plural(plan.moves.len(), "file"),
            human_size(total)
        ));
        if ui.small_button("Select all").clicked() {
            app.checked.iter_mut().for_each(|c| *c = true);
        }
        if ui.small_button("Select none").clicked() {
            app.checked.iter_mut().for_each(|c| *c = false);
        }
    });
    if !plan.skipped.is_empty() || !plan.errors.is_empty() {
        egui::CollapsingHeader::new(format!(
            "Left alone ({})",
            plan.skipped.len() + plan.errors.len()
        ))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("skipped")
                .max_height(120.0)
                .show(ui, |ui| {
                    for (p, r) in &plan.skipped {
                        ui.weak(format!("{}  —  {r}", p.display()));
                    }
                    for (p, e) in &plan.errors {
                        ui.colored_label(RED, format!("{}  —  {e}", p.display()));
                    }
                });
        });
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
                    ui.label(
                        m.source
                            .file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default(),
                    )
                    .on_hover_text(m.source.display().to_string());
                });
                row.col(|ui| {
                    ui.label(m.root.label());
                });
                row.col(|ui| {
                    ui.label(short(&m.target, roots))
                        .on_hover_text(m.target.display().to_string());
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
        if ui
            .add_enabled(can, egui::Button::new("🔍 Find duplicates"))
            .clicked()
        {
            app.start_dupes(ctx);
        }
        let can_confirm = can && app.journal.is_some() && problem.is_none() && to_go > 0;
        if ui
            .add_enabled(
                can_confirm,
                egui::Button::new(format!("🗑 Confirm ({to_go})")),
            )
            .clicked()
        {
            app.pending = Some(Pending::Duplicates(choices.clone()));
        }
        ui.weak(format!(
            "Ignores files under {} KB.",
            app.settings.min_duplicate_kb
        ));
    });
    ui.separator();
    let Some(view) = &mut app.dupes else {
        intro(
            ui,
            "Find files with exactly the same content.",
            "Downloads, Desktop and Documents with all sub-folders. Nothing goes until you press Confirm, and then only to the Recycle Bin.",
        );
        return;
    };
    ui.horizontal(|ui| {
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
        ui.label(format!(
            "No duplicates found ({} checked).",
            plural(view.scan.files_checked as usize, "file")
        ));
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
        .flat_map(|(g, grp)| {
            std::iter::once((g, None)).chain((0..grp.files.len()).map(move |f| (g, Some(f))))
        })
        .collect();
    let row_h = row_height(ui);
    let roots = app.roots.clone();
    TableBuilder::new(ui)
        .striped(false)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(80.0))
        .column(Column::initial(260.0).at_least(100.0).clip(true))
        .column(Column::remainder().at_least(140.0).clip(true))
        .column(Column::exact(90.0))
        .column(Column::exact(110.0))
        .header(row_h, |mut h| {
            for t in ["Keep", "File", "Folder", "Size", "Last changed"] {
                h.col(|ui| {
                    ui.strong(t);
                });
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
                            ui.add_space(18.0);
                            ui.add_enabled(
                                included,
                                egui::Checkbox::without_text(&mut view.keep[g][i]),
                            );
                        });
                        row.col(|ui| {
                            let name = file_name(file);
                            let gone = included && !view.keep[g][i];
                            let text = if gone {
                                egui::RichText::new(name).strikethrough().weak()
                            } else {
                                egui::RichText::new(name)
                            };
                            ui.label(text)
                                .on_hover_text(file.path.display().to_string());
                        });
                        row.col(|ui| {
                            let dir = file.path.parent().unwrap_or(&file.path);
                            let s = roots
                                .as_ref()
                                .map(|r| short(dir, r))
                                .unwrap_or_else(|| dir.display().to_string());
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
}

// ---------------------------------------------------------------- Big and old

pub fn bigold_tab(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let can = !app.busy() && app.roots.is_some();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can, egui::Button::new("🔍 Find big and old files"))
            .clicked()
        {
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
        ui.selectable_value(
            &mut view.show_old,
            false,
            format!("Big files ({})", view.scan.big.len()),
        );
        ui.selectable_value(
            &mut view.show_old,
            true,
            format!("Old files ({})", view.scan.old.len()),
        );
    });
    if view.show_old {
        ui.weak(
            "\"Old\" means not changed for a long time (Windows does not reliably record when a file was last opened).",
        );
    }
    skipped_list(ui, &view.scan.skipped);
    let list = if view.show_old {
        view.scan.old.clone()
    } else {
        view.scan.big.clone()
    };
    if list.is_empty() {
        ui.add_space(20.0);
        ui.label("Nothing on this list.");
        return;
    }
    let row_h = row_height(ui);
    let mut recycle: Option<ScannedFile> = None;
    let mut open_err = None;
    let busy = app.busy();
    let folders: Vec<String> = list.iter().map(|f| folder(app, f)).collect();
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(260.0).at_least(100.0).clip(true))
        .column(Column::remainder().at_least(140.0).clip(true))
        .column(Column::exact(90.0))
        .column(Column::exact(110.0))
        .column(Column::exact(230.0))
        .header(row_h, |mut h| {
            for t in ["File", "Folder", "Size", "Last changed", ""] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(row_h, list.len(), |mut row| {
                let i = row.index();
                let f = &list[i];
                row.col(|ui| {
                    ui.label(file_name(f))
                        .on_hover_text(f.path.display().to_string());
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
                    if ui.small_button("📂 Open folder").clicked() {
                        if let Err(e) = show_in_folder(&f.path) {
                            open_err = Some(format!("Could not open Explorer: {e}"));
                        }
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("🗑 Recycle").small())
                        .clicked()
                    {
                        recycle = Some(f.clone());
                    }
                });
            });
        });
    if let Some(f) = recycle {
        app.pending = Some(Pending::Files(vec![f]));
    }
    if open_err.is_some() {
        app.message = open_err;
    }
}

// ---------------------------------------------------------------- Rename

pub fn rename_tab(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let can = !app.busy() && app.roots.is_some();
    let n = app.rename_checked.iter().filter(|c| **c).count();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can, egui::Button::new("🔍 Find messy names"))
            .clicked()
        {
            app.start_renames(ctx);
        }
        let can_confirm = can && app.renames.is_some() && n > 0 && app.journal.is_some();
        if ui
            .add_enabled(
                can_confirm,
                egui::Button::new(format!("✔ Rename ticked ({n})")),
            )
            .clicked()
        {
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
        egui::CollapsingHeader::new(format!(
            "Not renamed, name taken ({})",
            scan.conflicts.len()
        ))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("conflicts")
                .max_height(120.0)
                .show(ui, |ui| {
                    for (p, why) in &scan.conflicts {
                        ui.weak(format!("{}  —  {why}", p.display()));
                    }
                });
        });
    }
    skipped_list(ui, &scan.skipped);
    if scan.items.is_empty() {
        ui.add_space(20.0);
        ui.label(format!(
            "No messy names found ({} checked).",
            plural(scan.files_checked as usize, "file")
        ));
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
                    ui.label(item.old_name())
                        .on_hover_text(item.file.path.display().to_string());
                });
                row.col(|ui| {
                    ui.label(format!("→ {}", item.new_name()));
                });
                row.col(|ui| {
                    ui.label(&folders[i]).on_hover_text(
                        item.file
                            .path
                            .parent()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                    );
                });
            });
        });
}
