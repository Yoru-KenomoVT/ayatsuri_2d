// File > Export Model: moc3 (version and bundle), cmo3 (in progress) and a
// still image, on tabs.

use crate::panels::EditorUi;
use crate::theme;
use egui::Ui;

impl EditorUi {
    pub(crate) fn export_window(&mut self, ui: &mut Ui) {
        let Some(dlg) = &mut self.export_dlg else { return };
        let mut open = true;
        let mut go_moc3 = false;
        let mut go_image: Option<(u32, bool, bool)> = None;
        let canvas = self.session.as_ref().map(|s| s.puppet.model().canvas);
        crate::windows::Dialog::new(crate::i18n::tr("Export Model"))
            .open(&mut open)
            .width(400.0)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    for (i, label) in ["moc3", "cmo3", "Image"].iter().enumerate() {
                        ui.selectable_value(&mut dlg.tab, i, crate::i18n::tr(label));
                    }
                });
                ui.separator();
                match dlg.tab {
                    1 => {
                        ui.label(crate::i18n::tr("Cubism Editor project (.cmo3)"));
                        ui.weak(crate::i18n::tr(
                            "Writing a project Cubism Editor opens is still being built. Until \
                             it lands, a moc3 export runs in every Cubism-compatible app, and a \
                             .cmo3 can already be opened here.",
                        ));
                        ui.add_space(6.0);
                        ui.add_enabled(false, egui::Button::new(crate::i18n::tr("Export…")));
                    }
                    2 => {
                        crate::windows::form(ui, "export-image", |ui| {
                            ui.label(crate::i18n::tr("Format"));
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut dlg.jpeg, false, "PNG");
                                ui.selectable_value(&mut dlg.jpeg, true, "JPEG");
                            });
                            ui.end_row();
                            ui.label(crate::i18n::tr("Width"));
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(&mut dlg.image_width)
                                        .range(16..=16384)
                                        .suffix(" px"),
                                );
                                if let Some(c) = canvas {
                                    let h = (dlg.image_width as f32 * c.height / c.width.max(1.0))
                                        .round() as u32;
                                    ui.weak(crate::i18n::trf("× {height} px", &[("height", &h)]));
                                    if ui.small_button(crate::i18n::tr("Canvas size")).clicked() {
                                        dlg.image_width = c.width.max(1.0) as u32;
                                    }
                                }
                            });
                            ui.end_row();
                            ui.label(crate::i18n::tr("Background"));
                            ui.add_enabled(
                                !dlg.jpeg,
                                egui::Checkbox::new(
                                    &mut dlg.transparent,
                                    crate::i18n::tr("Transparent"),
                                ),
                            );
                            ui.end_row();
                        });
                        ui.weak(crate::i18n::tr(
                            "The model as it stands now: the parameters, parts and physics \
                             pose on the canvas. JPEG and opaque PNG sit on white.",
                        ));
                        ui.add_space(6.0);
                        if ui
                            .button(egui::RichText::new(crate::i18n::tr("Export…")).strong())
                            .clicked()
                        {
                            go_image =
                                Some((dlg.image_width, dlg.jpeg, dlg.transparent && !dlg.jpeg));
                        }
                    }
                    _ => {
                        crate::windows::form(ui, "export", |ui| {
                            ui.label(crate::i18n::tr("Name"));
                            ui.text_edit_singleline(&mut dlg.name);
                            ui.end_row();
                            ui.label(crate::i18n::tr("moc3 version"));
                            egui::ComboBox::from_id_salt("moc-ver")
                                .selected_text(
                                    ayatsuri_import::ExportVersion::ALL
                                        .get(dlg.version)
                                        .map(|v| v.label())
                                        .unwrap_or("4.2"),
                                )
                                .show_ui(ui, |ui| {
                                    for (i, v) in
                                        ayatsuri_import::ExportVersion::ALL.iter().enumerate()
                                    {
                                        let item =
                                            ui.selectable_value(&mut dlg.version, i, v.label());
                                        if *v == ayatsuri_import::ExportVersion::V5_03 {
                                            item.on_hover_text(ayatsuri_import::VTS_NOTE);
                                        }
                                    }
                                });
                            ui.end_row();
                            ui.label(crate::i18n::tr("Include"));
                            ui.vertical(|ui| {
                                ui.checkbox(&mut dlg.physics, crate::i18n::tr("physics3.json"));
                                ui.checkbox(
                                    &mut dlg.motions,
                                    crate::i18n::tr("motion3.json for each animation"),
                                );
                                ui.checkbox(
                                    &mut dlg.expressions,
                                    crate::i18n::tr("exp3.json for each expression"),
                                );
                            });
                            ui.end_row();
                        });
                        ui.weak(crate::i18n::tr(
                            "Writes <name>.moc3, .model3.json, .cdi3.json and the textures.",
                        ));
                        if let (Some(s), Some(v)) =
                            (&self.session, ayatsuri_import::ExportVersion::ALL.get(dlg.version))
                        {
                            for w in ayatsuri_import::export_warnings(s.puppet.model(), *v) {
                                let lossy = !w.starts_with("physics");
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} {w}",
                                        if lossy {
                                            egui_phosphor::regular::WARNING
                                        } else {
                                            egui_phosphor::regular::INFO
                                        }
                                    ))
                                    .size(11.0)
                                    .color(if lossy {
                                        theme::pal().accent
                                    } else {
                                        theme::pal().text_faint
                                    }),
                                );
                            }
                        }
                        if ui
                            .button(egui::RichText::new(crate::i18n::tr("Export…")).strong())
                            .clicked()
                        {
                            go_moc3 = true;
                        }
                    }
                }
            });
        if go_moc3 {
            let Some(dlg) = self.export_dlg.take() else { return };
            self.pick_async(crate::panels::PickFor::ExportMoc3(Box::new(dlg)), || {
                rfd::FileDialog::new()
                    .set_title(crate::i18n::tr("Export as moc3: choose a folder"))
                    .pick_folder()
            });
        } else if let Some((w, jpeg, transparent)) = go_image {
            self.export_image(w, jpeg, transparent);
        } else if !open {
            self.export_dlg = None;
        }
    }

    /// Write the Cubism bundle into the picked folder.
    /// Export to a folder, asking first when it already holds a model.
    pub(crate) fn export_moc3_to(&mut self, dlg: &crate::panels::ExportDlg, dir: &std::path::Path) {
        let found = crate::export::existing_models(dir);
        if found.is_empty() {
            self.export_moc3_now(dlg, dir);
        } else {
            let names = found
                .iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect();
            self.export_overwrite = Some((dlg.clone(), dir.to_path_buf(), names));
        }
    }

    /// The overwrite question for an export folder that already has a model.
    pub(crate) fn export_overwrite_window(&mut self, ui: &mut Ui) {
        let Some((_, dir, names)) = &self.export_overwrite else { return };
        let (dir, names) = (dir.clone(), names.clone());
        let mut open = true;
        let mut go = false;
        let mut cancel = false;
        crate::windows::Dialog::new(crate::i18n::tr("Overwrite Export"))
            .open(&mut open)
            .width(380.0)
            .show(ui.ctx(), |ui| {
                ui.label(crate::i18n::trf(
                    "{folder} already has a model: {names}.",
                    &[("folder", &dir.display()), ("names", &names.join(", "))],
                ));
                ui.weak(crate::i18n::tr(
                    "Overwrite it with a fresh export? The files that model lists (moc3, \
                     textures, physics, motions, expressions...) are replaced; anything else \
                     in the folder, like VTube Studio's settings, stays as it is.",
                ));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("Overwrite")).clicked() {
                        go = true;
                    }
                    if ui.button(crate::i18n::tr("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if go && let Some((dlg, dir, _)) = self.export_overwrite.take() {
            match crate::export::clear_previous(&dir) {
                Ok(n) => {
                    log::info!("removed {n} file(s) of the previous export in {}", dir.display());
                    self.export_moc3_now(&dlg, &dir);
                }
                Err(e) => log::error!("could not clear the previous export: {e}"),
            }
        } else if cancel || !open {
            self.export_overwrite = None;
            log::info!("export cancelled: the folder already had a model");
        }
    }

    fn export_moc3_now(&mut self, dlg: &crate::panels::ExportDlg, dir: &std::path::Path) {
        let version = ayatsuri_import::ExportVersion::ALL
            .get(dlg.version)
            .copied()
            .unwrap_or(ayatsuri_import::ExportVersion::DEFAULT);
        let Some(s) = &self.session else { return };
        let opt = crate::export::Options {
            name: dlg.name.clone(),
            version,
            physics: dlg.physics,
            motions: dlg.motions,
            expressions: dlg.expressions,
        };
        match crate::export::export(s, dir, &opt) {
            Ok(files) => {
                log::info!(
                    "exported {} files (moc3 {}) to {}",
                    files.len(),
                    version.label(),
                    dir.display()
                );
                // apps watching for exports (VTube Studio) pick it up
                let moc = files.iter().find(|f| f.extension().is_some_and(|e| e == "moc3"));
                let model_file =
                    s.aom_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                self.api_notify(
                    "NotifyMocFileExported",
                    serde_json::json!({
                        "Path": moc.map(|p| p.display().to_string()).unwrap_or_default(),
                        "ModelFilePath": model_file,
                        "Files": files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
                    }),
                );
            }
            Err(e) => log::error!("export failed: {e}"),
        }
    }
}
