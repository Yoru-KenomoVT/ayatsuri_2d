// File > Export as moc3: version picker and the bundle written next to it.

use crate::panels::EditorUi;
use crate::theme;
use egui::Ui;

impl EditorUi {
    pub(crate) fn export_window(&mut self, ui: &mut Ui) {
        let Some(dlg) = &mut self.export_dlg else { return };
        let mut open = true;
        let mut go = false;
        egui::Window::new("Export as moc3")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                egui::Grid::new("export").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut dlg.name);
                    ui.end_row();
                    ui.label("moc3 version");
                    egui::ComboBox::from_id_salt("moc-ver")
                        .selected_text(
                            ayatsuri_import::ExportVersion::ALL
                                .get(dlg.version)
                                .map(|v| v.label())
                                .unwrap_or("4.2"),
                        )
                        .show_ui(ui, |ui| {
                            for (i, v) in ayatsuri_import::ExportVersion::ALL.iter().enumerate() {
                                ui.selectable_value(&mut dlg.version, i, v.label());
                            }
                        });
                    ui.end_row();
                    ui.label("Include");
                    ui.vertical(|ui| {
                        ui.checkbox(&mut dlg.physics, "physics3.json");
                        ui.checkbox(&mut dlg.motions, "motion3.json for each animation");
                    });
                    ui.end_row();
                });
                ui.weak("Writes <name>.moc3, .model3.json, .cdi3.json and the textures.");
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
                if ui.button(egui::RichText::new("Export…").strong()).clicked() {
                    go = true;
                }
            });
        if go {
            let Some(dlg) = self.export_dlg.take() else { return };
            let Some(dir) = rfd::FileDialog::new().pick_folder() else {
                self.export_dlg = Some(dlg);
                return;
            };
            let version = ayatsuri_import::ExportVersion::ALL
                .get(dlg.version)
                .copied()
                .unwrap_or(ayatsuri_import::ExportVersion::V5_03);
            if let Some(s) = &self.session {
                let opt = crate::export::Options {
                    name: dlg.name.clone(),
                    version,
                    physics: dlg.physics,
                    motions: dlg.motions,
                };
                match crate::export::export(s, &dir, &opt) {
                    Ok(files) => log::info!(
                        "exported {} files (moc3 {}) to {}",
                        files.len(),
                        version.label(),
                        dir.display()
                    ),
                    Err(e) => log::error!("export failed: {e}"),
                }
            }
        } else if !open {
            self.export_dlg = None;
        }
    }
}
