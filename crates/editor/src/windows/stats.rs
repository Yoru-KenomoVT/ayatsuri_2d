// Modeling > Model Statistics: counts of everything in the open model.

use crate::panels::EditorUi;
use ayatsuri_format::model::DeformerKind;
use egui::Ui;

impl EditorUi {
    /// Cubism's Model Statistics dialog: object and geometry counts.
    pub(crate) fn stats_window(&mut self, ui: &mut Ui) {
        if !self.show_stats {
            return;
        }
        let mut open = self.show_stats;
        egui::Window::new("Model Statistics")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                let Some(s) = &self.session else {
                    ui.weak("No model loaded.");
                    return;
                };
                let m = s.puppet.model();
                let (mut verts, mut tris, mut keyforms) = (0usize, 0usize, 0usize);
                for am in &m.art_meshes {
                    verts += am.vertex_count as usize;
                    tris += am.indices.len() / 3;
                    let stride = (am.vertex_count as usize).saturating_mul(2);
                    keyforms += am.positions.len().checked_div(stride).unwrap_or(0);
                }
                let warps =
                    m.deformers.iter().filter(|d| matches!(d.kind, DeformerKind::Warp(_))).count();
                ui.set_width(260.0);
                egui::Grid::new("stats").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
                    for (label, value) in [
                        ("Canvas", format!("{} × {} px", m.canvas.width, m.canvas.height)),
                        ("Parameters", m.parameters.len().to_string()),
                        ("Parts", m.parts.len().to_string()),
                        ("Deformers", m.deformers.len().to_string()),
                        ("  Warp", warps.to_string()),
                        ("  Rotation", (m.deformers.len() - warps).to_string()),
                        ("ArtMeshes", m.art_meshes.len().to_string()),
                        ("Vertices", verts.to_string()),
                        ("Triangles", tris.to_string()),
                        ("Keyforms", keyforms.to_string()),
                        ("Textures", s.texture_names.len().to_string()),
                    ] {
                        ui.weak(label);
                        ui.label(value);
                        ui.end_row();
                    }
                });
            });
        self.show_stats = open;
    }
}
