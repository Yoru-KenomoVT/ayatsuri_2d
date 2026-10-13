// Recover Unsaved Work: shown at startup when a crash or a kill left
// autosaves or crash copies behind.

use egui::{RichText, Ui};

use crate::panels::EditorUi;
use crate::recovery::Found;
use crate::theme;

#[derive(Default)]
pub(crate) struct RecoveryState {
    pub(crate) found: Vec<Found>,
    pub(crate) checked: bool,
}

impl EditorUi {
    pub(crate) fn recovery_window(&mut self, ui: &mut Ui) {
        if !self.recovery.checked {
            self.recovery.checked = true;
            let handed = std::env::var_os("AYATSURI_RECOVER").map(std::path::PathBuf::from);
            self.recovery.found = crate::recovery::scan(handed.as_deref());
            if !self.recovery.found.is_empty() {
                log::info!("{} recoverable file(s) found", self.recovery.found.len());
            }
        }
        if self.recovery.found.is_empty() {
            return;
        }
        let mut open_one: Option<usize> = None;
        let mut delete_one: Option<usize> = None;
        let mut delete_all = false;
        let mut later = false;
        crate::windows::Dialog::new(crate::i18n::tr("Recover Unsaved Work")).width(460.0).show(
            ui.ctx(),
            |ui| {
                ui.label(crate::i18n::tr(
                    "Ayatsuri2D closed before this work was saved. Recover it?",
                ));
                ui.label(
                    RichText::new(crate::i18n::tr(
                        "A recovered model opens unsaved, so Save asks where it goes.",
                    ))
                    .size(10.5)
                    .color(theme::pal().text_faint),
                );
                ui.add_space(8.0);
                for (i, f) in self.recovery.found.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(250.0);
                            ui.label(RichText::new(&f.name).strong());
                            ui.label(
                                RichText::new(crate::i18n::trf(
                                    "{age}  ·  {size} MB",
                                    &[
                                        ("age", &crate::recovery::ago(f.age)),
                                        ("size", &format!("{:.1}", f.bytes as f64 / 1_048_576.0)),
                                    ],
                                ))
                                .size(10.5)
                                .color(theme::pal().text_faint),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(crate::i18n::tr("Delete")).clicked() {
                                delete_one = Some(i);
                            }
                            if ui
                                .button(RichText::new(crate::i18n::tr("Recover")).strong())
                                .clicked()
                            {
                                open_one = Some(i);
                            }
                        });
                    });
                    ui.add_space(4.0);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("Delete All")).clicked() {
                        delete_all = true;
                    }
                    if ui
                        .button(crate::i18n::tr("Later"))
                        .on_hover_text(crate::i18n::tr("Ask again next time"))
                        .clicked()
                    {
                        later = true;
                    }
                });
            },
        );
        if let Some(i) = open_one
            && i < self.recovery.found.len()
        {
            let f = self.recovery.found.remove(i);
            self.recovering = Some(f.path.clone());
            self.import(&ui.ctx().clone(), f.path);
        }
        if let Some(i) = delete_one
            && i < self.recovery.found.len()
        {
            let f = self.recovery.found.remove(i);
            let _ = std::fs::remove_file(&f.path);
            log::info!("deleted recovery file {}", f.path.display());
        }
        if delete_all {
            for f in self.recovery.found.drain(..) {
                let _ = std::fs::remove_file(&f.path);
            }
            log::info!("deleted all recovery files");
        }
        if later {
            self.recovery.found.clear();
        }
    }
}
