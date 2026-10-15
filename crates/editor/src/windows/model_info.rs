// File > Project: the name, author and timelapse switch saved in the .aom meta.

use crate::panels::EditorUi;
use egui::Ui;

impl EditorUi {
    pub(crate) fn model_info_window(&mut self, ui: &mut Ui) {
        if !self.show_model_info {
            return;
        }
        let mut open = self.show_model_info;
        let default_rec = !self.prefs.timelapse_off;
        crate::windows::Dialog::new(crate::i18n::tr("Project")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                let Some(s) = &mut self.session else {
                    ui.weak(crate::i18n::tr("No model loaded."));
                    return;
                };
                ui.set_width(280.0);
                crate::windows::form(ui, "minfo", |ui| {
                    ui.label(crate::i18n::tr("Name"));
                    ui.text_edit_singleline(&mut s.name);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Author"));
                    ui.text_edit_singleline(&mut s.author);
                    ui.end_row();
                    let c = s.puppet.model().canvas;
                    ui.label(crate::i18n::tr("Canvas"));
                    ui.weak(crate::i18n::trf(
                        "{width} × {height} px, {ppu} px/unit",
                        &[("width", &c.width), ("height", &c.height), ("ppu", &c.pixels_per_unit)],
                    ));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Timelapse"));
                    let mut rec = s.timelapse_pref.unwrap_or(default_rec);
                    if ui.checkbox(&mut rec, crate::i18n::tr("Record timelapse")).changed() {
                        s.timelapse_pref = Some(rec);
                        s.modified = true;
                    }
                    ui.end_row();
                });
                ui.weak(crate::i18n::tr(
                    "Saved with the project. Settings > General holds the default for new projects.",
                ));
            },
        );
        self.show_model_info = open;
    }
}
