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
        crate::windows::Dialog::new(crate::i18n::tr("Model Statistics")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                let Some(s) = &self.session else {
                    ui.weak(crate::i18n::tr("No model loaded."));
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
                // offscreen parts, and the deepest nesting of them: each
                // layer costs a full-size draw, nested ones multiply it
                let offscreen = m.parts.iter().filter(|p| p.offscreen.is_some()).count();
                let depth_of = |mut at: Option<u32>| {
                    let mut depth = 0;
                    for _ in 0..m.parts.len() {
                        let Some(p) = at.and_then(|i| m.parts.get(i as usize)) else { break };
                        depth += usize::from(p.offscreen.is_some());
                        at = p.parent_part;
                    }
                    depth
                };
                let layers =
                    (0..m.parts.len()).map(|i| depth_of(Some(i as u32))).max().unwrap_or(0);
                let shapes: usize = m.art_meshes.iter().map(|am| am.blend_shapes.len()).sum();
                // meshes that may flicker: an advanced blend and triangles
                // covering each other, as the model stands now
                let overlapping = m
                    .art_meshes
                    .iter()
                    .enumerate()
                    .filter(|(mi, am)| {
                        ayatsuri_core::overlap::can_flicker(am)
                            && s.puppet.mesh_geometry(*mi).is_some_and(|g| {
                                !ayatsuri_core::overlap::overlapping(g.positions, g.indices)
                                    .is_empty()
                            })
                    })
                    .count();
                ui.set_width(260.0);
                crate::windows::form(ui, "stats", |ui| {
                    for (label, value) in [
                        (
                            "Canvas",
                            crate::i18n::trf(
                                "{width} × {height} px",
                                &[("width", &m.canvas.width), ("height", &m.canvas.height)],
                            ),
                        ),
                        ("Parameters", m.parameters.len().to_string()),
                        ("Parts", m.parts.len().to_string()),
                        ("  Offscreen drawing", offscreen.to_string()),
                        ("  Most nested offscreen layers", layers.to_string()),
                        ("Deformers", m.deformers.len().to_string()),
                        ("  Warp", warps.to_string()),
                        ("  Rotation", (m.deformers.len() - warps).to_string()),
                        ("ArtMeshes", m.art_meshes.len().to_string()),
                        ("  With overlapping polygons", overlapping.to_string()),
                        ("Vertices", verts.to_string()),
                        ("Triangles", tris.to_string()),
                        ("Keyforms", keyforms.to_string()),
                        ("Blend shapes", shapes.to_string()),
                        ("Textures", s.texture_names.len().to_string()),
                        (
                            "  On the GPU at full size",
                            crate::i18n::trf(
                                "{n} of {total}, {mb} MB",
                                &[
                                    ("n", &s.residency.resident_count()),
                                    ("total", &s.texture_names.len()),
                                    ("mb", &(s.residency.resident_bytes() >> 20)),
                                ],
                            ),
                        ),
                    ] {
                        ui.weak(crate::i18n::tr(label));
                        ui.label(value);
                        ui.end_row();
                    }
                });
            },
        );
        self.show_stats = open;
    }
}
