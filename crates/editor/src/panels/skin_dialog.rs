// Modeling > Skinning, the two dialogs Cubism has besides the deform path
// way: Skinning (a mesh cut onto rotation deformers already placed along
// it) and Generate Parameters for Rotation Deformers (one angle parameter
// each). Both list the model's rotation deformers to tick, root first.

use ayatsuri_format::model::DeformerKind;
use egui::{Ui, vec2};

use super::{EditorUi, Sel};
use crate::i18n::{tr, trf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SkinMode {
    /// Skin this mesh onto the ticked deformers.
    Skin(usize),
    /// A parameter for each ticked deformer.
    Params,
}

pub(super) struct SkinDialog {
    mode: SkinMode,
    /// Rotation deformers, root first, and whether each is ticked.
    rows: Vec<(usize, bool)>,
    range: f32,
}

/// How deep a deformer sits in the tree: roots first, then their children.
fn depth(model: &ayatsuri_format::Model, mut d: usize) -> usize {
    let mut n = 0;
    while let Some(p) = model.deformers.get(d).and_then(|x| x.parent_deformer) {
        d = p as usize;
        n += 1;
        if n > 256 {
            break;
        }
    }
    n
}

impl EditorUi {
    /// Open the Skinning dialog for the selected mesh: the rotation
    /// deformers whose pivot sits on the mesh start ticked.
    pub(super) fn open_skin_dialog(&mut self, params_only: bool) {
        let Some(s) = &self.session else { return };
        let model = s.puppet.model();
        let mesh = match self.selected {
            Some(Sel::Mesh(m)) => Some(m),
            _ => None,
        };
        let bounds = mesh.and_then(|m| s.puppet.mesh_geometry(m)).map(|g| {
            let pts = g.positions.as_chunks::<2>().0;
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for p in pts {
                x0 = x0.min(p[0]);
                y0 = y0.min(p[1]);
                x1 = x1.max(p[0]);
                y1 = y1.max(p[1]);
            }
            (x0, y0, x1, y1)
        });
        let mut rows: Vec<(usize, bool)> = model
            .deformers
            .iter()
            .enumerate()
            .filter(|(_, d)| matches!(d.kind, DeformerKind::Rotation(_)))
            .map(|(i, _)| {
                let on = if params_only {
                    self.selected == Some(Sel::Deformer(i))
                } else {
                    bounds.zip(s.puppet.rotation_state(i)).is_some_and(|((x0, y0, x1, y1), r)| {
                        r.origin.x >= x0 && r.origin.x <= x1 && r.origin.y >= y0 && r.origin.y <= y1
                    })
                };
                (i, on)
            })
            .collect();
        rows.sort_by_key(|&(i, _)| (depth(model, i), i));
        let mode = match (params_only, mesh) {
            (false, Some(m)) => SkinMode::Skin(m),
            (false, None) => return,
            (true, _) => SkinMode::Params,
        };
        self.skin_dlg = Some(SkinDialog { mode, rows, range: 30.0 });
    }

    pub(super) fn skin_dialog(&mut self, ui: &mut Ui) {
        let Some(dlg) = &mut self.skin_dlg else { return };
        let Some(s) = &self.session else {
            self.skin_dlg = None;
            return;
        };
        let title = match dlg.mode {
            SkinMode::Skin(_) => tr("Skinning"),
            SkinMode::Params => tr("Generate Parameters for Rotation Deformers"),
        };
        let mut open = true;
        let mut go = false;
        crate::windows::Dialog::new(title).open(&mut open).width(380.0).show(ui.ctx(), |ui| {
            match dlg.mode {
                SkinMode::Skin(m) => {
                    ui.label(trf(
                        "Cut {mesh} onto these rotation deformers, root first:",
                        &[("mesh", &s.mesh_name(m))],
                    ));
                }
                SkinMode::Params => {
                    ui.label(tr("A parameter for each of these rotation deformers:"));
                }
            }
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                if dlg.rows.is_empty() {
                    ui.weak(tr("This model has no rotation deformers yet."));
                }
                for (d, on) in &mut dlg.rows {
                    let name = s
                        .puppet
                        .model()
                        .deformers
                        .get(*d)
                        .map(|x| x.id.clone())
                        .unwrap_or_default();
                    ui.checkbox(on, name);
                }
            });
            if dlg.mode == SkinMode::Params {
                ui.horizontal(|ui| {
                    ui.label(tr("Angle range"));
                    ui.add(egui::DragValue::new(&mut dlg.range).range(1.0..=180.0).suffix("°"));
                    ui.weak(trf("(-{r} to {r})", &[("r", &dlg.range)]));
                });
            }
            ui.add_space(6.0);
            let picked = dlg.rows.iter().filter(|(_, on)| *on).count();
            let need = if matches!(dlg.mode, SkinMode::Skin(_)) { 2 } else { 1 };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = match dlg.mode {
                    SkinMode::Skin(_) => tr("Skin"),
                    SkinMode::Params => tr("Generate"),
                };
                if ui
                    .add_enabled(picked >= need, egui::Button::new(label).min_size(vec2(84.0, 0.0)))
                    .clicked()
                {
                    go = true;
                }
                if picked < need {
                    ui.weak(if need == 2 {
                        tr("Tick two rotation deformers or more")
                    } else {
                        tr("Tick a rotation deformer")
                    });
                }
            });
        });
        if go {
            self.run_skin_dialog();
        } else if !open {
            self.skin_dlg = None;
        }
    }

    fn run_skin_dialog(&mut self) {
        let Some(dlg) = self.skin_dlg.take() else { return };
        let Some(s) = &mut self.session else { return };
        let picked: Vec<usize> = dlg.rows.iter().filter(|(_, on)| *on).map(|(d, _)| *d).collect();
        match dlg.mode {
            SkinMode::Skin(mesh) => {
                let name = s.mesh_name(mesh);
                let mut failed = None;
                let mut made = None;
                s.edit(|m| match ayatsuri_core::skin::skin_with_rotations(m, mesh, &picked) {
                    Ok(r) => {
                        made = Some(r);
                        true
                    }
                    Err(e) => {
                        failed = Some(e);
                        false
                    }
                });
                if let Some(e) = failed {
                    log::warn!("skinning {name}: {}", skin_error(e));
                } else if let Some(r) = made {
                    log::info!(
                        "skinned {name} onto {} rotation deformers ({} pieces, glued)",
                        r.deformers.len(),
                        r.pieces.len()
                    );
                }
            }
            SkinMode::Params => {
                let mut made = None;
                let mut failed = None;
                s.edit(|m| {
                    match ayatsuri_core::skin::rotation_parameters(
                        m,
                        &picked,
                        dlg.range,
                        "ParamRotation",
                    ) {
                        Ok(p) => {
                            made = Some(p);
                            true
                        }
                        Err(e) => {
                            failed = Some(e);
                            false
                        }
                    }
                });
                if let Some(e) = failed {
                    log::warn!("rotation parameters: {}", skin_error(e));
                    return;
                }
                let Some(params) = made else { return };
                let names: Vec<String> = picked
                    .iter()
                    .map(|&d| {
                        s.puppet.model().deformers.get(d).map(|x| x.id.clone()).unwrap_or_default()
                    })
                    .collect();
                for name in &names {
                    s.param_names.push(name.clone());
                }
                s.groups.push(crate::session::ParamGroup {
                    name: tr("Rotation").to_string(),
                    params: params.clone(),
                    open: true,
                    color: None,
                });
                log::info!(
                    "{} rotation parameter(s) made, -{r} to {r}",
                    params.len(),
                    r = dlg.range
                );
            }
        }
    }
}

pub(super) fn skin_error(e: ayatsuri_core::skin::SkinError) -> &'static str {
    use ayatsuri_core::skin::SkinError;
    match e {
        SkinError::EmptyPiece => {
            "a piece came out empty; space the points or deformers further apart or give the mesh more points"
        }
        SkinError::TooFewPoints => "it needs a root and a tip at least",
        SkinError::NotRotation => "only rotation deformers can skin a mesh",
        SkinError::NoMesh => "the mesh is gone",
    }
}
