// File > Model Info: the name and author saved in the .aom meta.

use crate::panels::EditorUi;
use egui::Ui;

impl EditorUi {
    pub(crate) fn model_info_window(&mut self, ui: &mut Ui) {
        if !self.show_model_info {
            return;
        }
        let mut open = self.show_model_info;
        crate::windows::Dialog::new("Model Info").open(&mut open).show(ui.ctx(), |ui| {
            let Some(s) = &mut self.session else {
                ui.weak("No model loaded.");
                return;
            };
            ui.set_width(280.0);
            crate::windows::form(ui, "minfo", |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut s.name);
                ui.end_row();
                ui.label("Author");
                ui.text_edit_singleline(&mut s.author);
                ui.end_row();
                let c = s.puppet.model().canvas;
                ui.label("Canvas");
                ui.weak(format!("{} × {} px, {} px/unit", c.width, c.height, c.pixels_per_unit));
                ui.end_row();
            });
            ui.weak("Name and author save into the .aom meta block.");
        });
        self.show_model_info = open;
    }
}
