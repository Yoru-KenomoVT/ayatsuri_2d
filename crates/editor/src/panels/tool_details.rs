// The Tool Details palette, laid out after Cubism's: the active tool's
// name, then a section per group of options, sub-tools as square icon
// buttons, settings as label/value rows. Options the manual lists that we
// have not built yet are shown greyed out with a note, so the palette reads
// like Cubism's without pretending they work.

use egui::{Align2, Color32, FontId, Sense, Stroke, StrokeKind, Ui, vec2};
use egui_phosphor::regular as ph;

use super::{DeformerKind, EditorUi, MeshAction, Sel, Tool};
use crate::theme;

/// Overlay colours from the arrow tool's display settings.
#[derive(Clone, Copy)]
pub(super) struct DisplayOpts {
    pub(super) mesh_width: f32,
    pub(super) mesh_color: Color32,
    pub(super) warp_color: Color32,
    pub(super) rotation_color: Color32,
    pub(super) weight_color: Color32,
}

impl DisplayOpts {
    /// The settings in effect: saved choices, else the theme (mesh lines
    /// follow the primary colour, so changing it in Settings shows here).
    pub(super) fn from_prefs(o: &crate::prefs::Overlay) -> Self {
        let d = Self::default();
        let c = |v: Option<[u8; 4]>, fallback: Color32| {
            v.map_or(fallback, |[r, g, b, a]| Color32::from_rgba_unmultiplied(r, g, b, a))
        };
        Self {
            mesh_width: o.mesh_width.unwrap_or(d.mesh_width),
            mesh_color: c(o.mesh_color, d.mesh_color),
            warp_color: c(o.warp_color, d.warp_color),
            rotation_color: c(o.rotation_color, d.rotation_color),
            weight_color: c(o.weight_color, d.weight_color),
        }
    }
}

impl Default for DisplayOpts {
    fn default() -> Self {
        Self {
            mesh_width: 1.0,
            mesh_color: theme::pal().accent,
            warp_color: theme::pal().warp_green,
            rotation_color: theme::pal().rotation_red,
            // Cubism's default weight colour, #80FF0000
            weight_color: Color32::from_rgba_unmultiplied(255, 0, 0, 128),
        }
    }
}

const NOT_YET: &str = "Not built yet";

/// A section header: title, a rule, and an info button whose popup holds
/// the how-to text, so the palette itself stays short.
pub(super) fn section(ui: &mut Ui, title: &str, info: &'static str) {
    let info = if info.is_empty() { info } else { crate::i18n::tr(info) };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(11.0).strong().color(theme::pal().text_faint));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !info.is_empty() {
                ui.menu_button(egui::RichText::new(ph::INFO).size(13.0), |ui| {
                    ui.set_max_width(280.0);
                    ui.label(egui::RichText::new(title).strong());
                    ui.add_space(2.0);
                    for para in info.split("\n\n") {
                        ui.add(egui::Label::new(para).wrap());
                        ui.add_space(3.0);
                    }
                })
                .response
                .on_hover_text(crate::i18n::tr("How this works"));
            }
            let r = ui.available_rect_before_wrap();
            let y = r.center().y;
            ui.painter().line_segment(
                [egui::pos2(r.min.x + 4.0, y), egui::pos2(r.max.x - 4.0, y)],
                Stroke::new(1.0, theme::pal().border),
            );
        });
    });
    ui.add_space(2.0);
}

/// A square tool icon, the way Cubism's Tool Details shows sub-tools.
fn icon(ui: &mut Ui, on: bool, enabled: bool, glyph: &str, tip: &'static str) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 26.0), sense);
    let bg = if on {
        theme::pal().accent.gamma_multiply(0.30)
    } else if enabled && resp.hovered() {
        theme::pal().header
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 4.0, bg);
    if on {
        ui.painter().rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, theme::pal().accent),
            StrokeKind::Inside,
        );
    }
    let fg = if enabled { theme::pal().text } else { theme::pal().text_faint.gamma_multiply(0.5) };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(16.0), fg);
    let tip = crate::i18n::tr(tip);
    let tip =
        if enabled { tip.to_string() } else { format!("{tip} ({})", crate::i18n::tr(NOT_YET)) };
    resp.on_hover_text(tip)
}

/// One setting: the label on the left, its control flush right, so the
/// controls line up down the palette's edge.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(theme::pal().text_faint));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
}

/// A run of rows with even spacing.
fn grid(ui: &mut Ui, _id: &str, body: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        body(ui);
    });
}

impl EditorUi {
    pub(super) fn tool_details_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(crate::i18n::tr(self.tool.label())).size(12.5).strong());
        });
        let mut action: Option<MeshAction> = None;
        egui::ScrollArea::vertical().id_salt("tool-details").auto_shrink([false, false]).show(
            ui,
            |ui| match self.tool {
                Tool::Arrow | Tool::Lasso => self.td_select(ui),
                Tool::MeshManual | Tool::MeshAuto => self.td_mesh(ui, &mut action),
                Tool::Glue => self.td_glue(ui, &mut action),
                Tool::DeformPath => self.td_path(ui),
                Tool::WarpCreate | Tool::RotationCreate => self.td_create(ui),
                Tool::BrushSelect | Tool::DeformBrush => self.td_brush(ui),
                Tool::ArtPath => self.td_artpath(ui),
                Tool::TempDeform => self.td_temp_deform(ui),
                Tool::TextureAtlas => {
                    section(
                        ui,
                        crate::i18n::tr("Texture atlas"),
                        "The texture atlas editor is not built yet.",
                    );
                }
            },
        );
        if let Some(a) = action {
            self.run_mesh_action(a);
        }
    }

    fn td_select(&mut self, ui: &mut Ui) {
        section(
            ui,
            crate::i18n::tr("Selection"),
            "Click selects. Shift or Ctrl+click adds to the selection. Alt+click walks up the deformer chain.\n\nLasso: with a mesh or warp selected, draw around points to pick them (Shift adds), then use the box to move, scale or rotate them. With nothing selected it selects whole meshes.",
        );

        section(
            ui,
            crate::i18n::tr("Warp deformer"),
            "The edit level (keys 1, 2, 3) picks what you edit. Level 1: division points; pick them with a click, Shift+click or a rectangle and drag. Level 2: Bezier control points at the Bezier division; drag a green anchor (it brings its handles) or a handle. Level 3: one Bezier patch over the outer frame, for big shape changes.\n\nThe box: drag inside to move, a red dot to scale (Shift keeps the aspect, Alt scales from the centre), just outside to rotate (Shift snaps to 15 degrees).\n\nEdits go into the keyform the parameters sit on; between keys nothing changes. Esc clears the pick.",
        );
        // the edit level picks what a warp shows for editing (keys 1, 2, 3)
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (lvl, glyph, tip) in [
                (1u8, ph::DOTS_NINE, "Level 1 (refine): division points only"),
                (
                    2,
                    ph::BEZIER_CURVE,
                    "Level 2 (normal): Bezier control points at the warp's Bezier division",
                ),
                (3, ph::BOUNDING_BOX, "Level 3 (rough): one Bezier patch on the outer frame"),
            ] {
                if icon(ui, self.edit_level == lvl, true, glyph, tip).clicked() {
                    self.edit_level = lvl;
                }
            }
        });
        let warp = match (self.selected, &self.session) {
            (Some(Sel::Deformer(di)), Some(s)) => {
                match s.puppet.model().deformers.get(di).map(|d| &d.kind) {
                    Some(DeformerKind::Warp(w)) => Some((di, w.cols as usize, w.rows as usize)),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some((di, cols, rows)) = warp {
            let cur = self.bezier_div.get(&di).copied().unwrap_or((2.min(cols), 2.min(rows)));
            let (mut bw, mut bh) = cur;
            // the conversion division is typed, then applied: stepping it
            // down and back up live would lose the shape on the way
            let (mut cw, mut ch) = match self.td_division {
                Some((d, c, r)) if d == di => (c, r),
                _ => (cols as u32, rows as u32),
            };
            let mut apply = false;
            grid(ui, "td-warp", |ui| {
                row(ui, crate::i18n::tr("Conversion division"), |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut cw).range(1..=64));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut ch).range(1..=64));
                        apply = ui
                            .add_enabled(
                                (cw, ch) != (cols as u32, rows as u32),
                                egui::Button::new(crate::i18n::tr("Apply")),
                            )
                            .on_hover_text(crate::i18n::tr(
                                "Change its columns and rows; its shape is kept",
                            ))
                            .clicked();
                    });
                });
                row(ui, crate::i18n::tr("Bezier division"), |ui| {
                    ui.add_enabled_ui(self.edit_level == 2, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut bw).range(1..=cols.max(1)));
                            ui.label("×");
                            ui.add(egui::DragValue::new(&mut bh).range(1..=rows.max(1)));
                        });
                    });
                });
            });
            if (cw, ch) != (cols as u32, rows as u32) {
                ui.weak(if cw < cols as u32 || ch < rows as u32 {
                    crate::i18n::tr(
                        "Fewer divisions keep the shape only at the points that remain.",
                    )
                } else {
                    crate::i18n::tr("Apply to change the division; the shape is kept.")
                });
            }
            self.td_division = ((cw, ch) != (cols as u32, rows as u32)).then_some((di, cw, ch));
            if apply {
                self.td_division = None;
                self.apply_warp_division(di, cw, ch);
            } else if (bw, bh) != cur {
                self.bezier_div.insert(di, (bw, bh));
            }
        }

        self.overlay_settings(ui);
    }

    /// The overlay display settings, shared by Tool Details and Settings >
    /// View. Changes are saved straight away.
    pub(crate) fn overlay_settings(&mut self, ui: &mut Ui) {
        let rgba = |c: Color32| {
            let [r, g, b, a] = c.to_srgba_unmultiplied();
            Some([r, g, b, a])
        };
        let mut d = self.disp;
        let mut changed = false;
        section(
            ui,
            crate::i18n::tr("ArtMesh display settings"),
            "Mesh line colour follows your primary colour until you pick one here.",
        );
        grid(ui, "td-disp-mesh", |ui| {
            row(ui, crate::i18n::tr("Mesh line width"), |ui| {
                if ui
                    .add(egui::DragValue::new(&mut d.mesh_width).range(0.5..=4.0).speed(0.05))
                    .changed()
                {
                    self.prefs.overlay.mesh_width = Some(d.mesh_width);
                    changed = true;
                }
            });
            row(ui, crate::i18n::tr("Mesh line color"), |ui| {
                if ui.color_edit_button_srgba(&mut d.mesh_color).changed() {
                    self.prefs.overlay.mesh_color = rgba(d.mesh_color);
                    changed = true;
                }
            });
        });
        section(ui, crate::i18n::tr("Warp deformer display settings"), "");
        grid(ui, "td-disp-warp", |ui| {
            row(ui, crate::i18n::tr("Grid color"), |ui| {
                if ui.color_edit_button_srgba(&mut d.warp_color).changed() {
                    self.prefs.overlay.warp_color = rgba(d.warp_color);
                    changed = true;
                }
            });
        });
        section(ui, crate::i18n::tr("Rotation deformer display settings"), "");
        grid(ui, "td-disp-rot", |ui| {
            row(ui, crate::i18n::tr("Handle color"), |ui| {
                if ui.color_edit_button_srgba(&mut d.rotation_color).changed() {
                    self.prefs.overlay.rotation_color = rgba(d.rotation_color);
                    changed = true;
                }
            });
        });
        section(ui, crate::i18n::tr("Brush selection display settings"), "");
        grid(ui, "td-disp-weight", |ui| {
            row(ui, crate::i18n::tr("Weight color"), |ui| {
                if ui.color_edit_button_srgba(&mut d.weight_color).changed() {
                    self.prefs.overlay.weight_color = rgba(d.weight_color);
                    changed = true;
                }
            });
        });
        if ui.small_button(crate::i18n::tr("Reset display settings")).clicked() {
            self.prefs.overlay = crate::prefs::Overlay::default();
            changed = true;
        }
        if changed {
            self.prefs.save();
            self.disp = DisplayOpts::from_prefs(&self.prefs.overlay);
        }
    }

    /// The mesh section, shared by Tool Details and the Edit Mesh popup.
    pub(super) fn td_mesh(&mut self, ui: &mut Ui, action: &mut Option<MeshAction>) {
        section(
            ui,
            crate::i18n::tr("Edit mesh manually"),
            "Select / Edit drags points. Lasso select picks points: Ctrl+C copies them, and Ctrl+V pastes them into the mesh being edited at the same spots (points landing on existing ones merge, ready for glue); Delete removes them. Eraser deletes everything it sweeps ([ and ] resize it).\n\nThe actions work on the mesh being edited, or on the selected meshes.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon(ui, self.mesh_mode == 0, true, ph::CURSOR, "Select / Edit: drag points")
                .clicked()
            {
                self.mesh_mode = 0;
            }
            if icon(
                ui,
                self.mesh_mode == 6,
                true,
                ph::LASSO,
                "Lasso select: pick points (Shift adds), Ctrl+C copies, Ctrl+V pastes into the \
                 mesh being edited",
            )
            .clicked()
            {
                self.mesh_mode = 6;
            }
            if icon(
                ui,
                self.mesh_mode == 5,
                true,
                ph::ERASER,
                "Eraser: drag to delete points and edges ([ ] resize)",
            )
            .clicked()
            {
                self.mesh_mode = 5;
            }
            if self.prefs.stroke_mesh_on()
                && icon(
                    ui,
                    self.mesh_mode == 7,
                    true,
                    ph::PEN_NIB,
                    "Stroke mesh-mapping (experimental): drag to build a mesh along a stroke. \
                     Shift+E confirms, Delete cancels",
                )
                .clicked()
            {
                self.mesh_mode = 7;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon(ui, false, true, ph::MAGIC_WAND, "Automatic mesh generator").clicked() {
                *action = Some(MeshAction::AutoMesh);
            }
            if icon(ui, false, true, ph::GRAPH, "Auto connect: triangulate the points you placed")
                .clicked()
            {
                *action = Some(MeshAction::AutoConnect);
            }
            if icon(ui, false, true, ph::GRID_FOUR, "Mesh 4 division: split every face in four")
                .clicked()
            {
                *action = Some(MeshAction::Divide);
            }
            if icon(ui, false, true, ph::LINK, "Bind glue (Ctrl+G)").clicked() {
                *action = Some(MeshAction::Bind);
            }
            if icon(ui, false, true, ph::LINK_BREAK, "Unbind glue").clicked() {
                *action = Some(MeshAction::Unbind);
            }
            if icon(
                ui,
                false,
                true,
                ph::ARROW_COUNTER_CLOCKWISE,
                "Reset mesh: back to the mesh as the file had it",
            )
            .clicked()
            {
                *action = Some(MeshAction::Reset);
            }
            if icon(ui, false, true, ph::BROOM, "Clear mesh: remove every point to draw your own")
                .clicked()
            {
                *action = Some(MeshAction::Clear);
            }
        });
        grid(ui, "td-mesh", |ui| {
            row(ui, crate::i18n::tr("Auto connect"), |ui| {
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.connect_rebuild, crate::i18n::tr("Mesh reconstruction"))
                        .on_hover_text(crate::i18n::tr(
                            "On: redo every face from the points. Off: keep the lines you built",
                        ));
                    ui.checkbox(
                        &mut self.connect_protect,
                        crate::i18n::tr("Protect edge exterior"),
                    )
                    .on_hover_text(crate::i18n::tr(
                        "Keep faces inside the outline you drew with lines; with none drawn, no \
                         faces over empty texture, so hair and lashes don't get bridged",
                    ));
                });
            });
            row(ui, crate::i18n::tr("New points"), |ui| {
                ui.checkbox(&mut self.mesh_autoconnect, crate::i18n::tr("Connect automatically"));
            });
            row(ui, crate::i18n::tr("Selection range on click"), |ui| {
                ui.add(egui::DragValue::new(&mut self.pick_radius).range(2.0..=40.0).suffix(" px"));
            });
        });

        section(ui, crate::i18n::tr("Vertices and edges"), "");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (mode, glyph, tip) in [
                (
                    1u8,
                    ph::PLUS_CIRCLE,
                    "Add vertex: each click draws a line from the last point. Ctrl+click adds \
                     a single point, Shift+click keeps the line's start, Esc ends the line, \
                     Alt+click deletes a point or line",
                ),
                (2, ph::MINUS_CIRCLE, "Delete vertex"),
                (3, ph::LINE_SEGMENT, "Connect two points with an edge"),
                (
                    4,
                    ph::SCISSORS,
                    "Delete edge (line): click a line or its two points, or drag across lines",
                ),
            ] {
                if icon(ui, self.mesh_mode == mode, true, glyph, tip).clicked() {
                    self.mesh_mode = mode;
                }
            }
        });
        if self.mesh_mode == 6 || self.point_clip.is_some() {
            section(ui, crate::i18n::tr("Lasso select"), "");
            ui.weak(crate::i18n::trf(
                "{count} point(s) picked. Ctrl+C copies, Delete removes them.",
                &[("count", &self.mesh_picked.len())],
            ));
            match &self.point_clip {
                Some(c) => ui.weak(crate::i18n::trf(
                    "{count} point(s) from {mesh} on the clipboard: Ctrl+V pastes them into the \
                     mesh being edited.",
                    &[("count", &c.points.len()), ("mesh", &super::truncate(&c.from, 20))],
                )),
                None => ui.weak(crate::i18n::tr("Nothing copied yet.")),
            };
        }
        if self.mesh_mode == 7 {
            section(
                ui,
                crate::i18n::tr("Mesh-mapping settings"),
                "Drag to draw a stroke; drag away from it to redraw it, Shift+drag for another. Click the line to add a control point, Alt+click one to delete it. Ctrl+click the line adds a width handle (orange), Ctrl+drag one sets the width, Ctrl+Alt+click deletes it. Shift+E confirms, Delete cancels.",
            );
            let o = &mut self.tool_opts.stroke;
            grid(ui, "td-stroke", |ui| {
                row(ui, crate::i18n::tr("Mesh width"), |ui| {
                    ui.add(
                        egui::DragValue::new(&mut o.settings.width).range(0.5..=500.0).speed(0.2),
                    );
                });
                row(ui, crate::i18n::tr("Repeated density"), |ui| {
                    ui.add(
                        egui::DragValue::new(&mut o.settings.density).range(2.0..=500.0).speed(0.2),
                    );
                });
                row(ui, crate::i18n::tr("Number of vertices in mesh width"), |ui| {
                    egui::ComboBox::from_id_salt("td-stroke-across")
                        .width(48.0)
                        .selected_text(o.settings.across.to_string())
                        .show_ui(ui, |ui| {
                            for n in 1..=3u8 {
                                ui.selectable_value(&mut o.settings.across, n, n.to_string());
                            }
                        });
                });
                row(ui, crate::i18n::tr("Invert mesh"), |ui| {
                    ui.add_enabled(
                        o.settings.across == 2,
                        egui::Checkbox::without_text(&mut o.settings.invert),
                    )
                    .on_hover_text(crate::i18n::tr(
                        "With 2 vertices across: flip the diagonal of the first and last cells",
                    ));
                });
                row(ui, crate::i18n::tr("Maintain current mesh"), |ui| {
                    ui.checkbox(&mut o.maintain, "").on_hover_text(crate::i18n::tr(
                        "Off: the points and edges the stroke mesh covers are removed",
                    ));
                });
            });
        }
        if self.mesh_mode == 5 {
            section(ui, crate::i18n::tr("Eraser"), "");
            grid(ui, "td-eraser", |ui| {
                row(ui, crate::i18n::tr("Size"), |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.tool_opts.brush_size, 4.0..=400.0)
                            .logarithmic(true)
                            .suffix(" px"),
                    );
                });
            });
        }

        // the same switch as View > Mirror Edit (Ctrl+Shift+M)
        section(ui, crate::i18n::tr("Mirror edit"), "");
        ui.checkbox(&mut self.mirror_edit, crate::i18n::tr("Mirror Edit")).on_hover_text(
            crate::i18n::tr(
                "Moving a point also moves its mirror across the vertical axis: \
             the vertical guide line nearest the centre, or the canvas centre",
            ),
        );
        // the axis's position, as Cubism's: drag its Mirror tab on the
        // canvas, or put it at the canvas centre or the picked points' centre
        ui.add_enabled_ui(self.mirror_edit, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button(crate::i18n::tr("Center of the canvas")).clicked() {
                    self.mirror_at = None;
                }
                if ui
                    .add_enabled(
                        self.mesh_edit.is_some(),
                        egui::Button::new(crate::i18n::tr("Center of selected vertices")),
                    )
                    .clicked()
                {
                    self.mirror_to_pick = true;
                }
            });
        });

        section(
            ui,
            crate::i18n::tr("Automatic mesh"),
            "Cubism's Automatic Mesh generator, applied to every selected mesh. Intervals are the gaps between points on the outline and inside. Margins are pixels on a 1024px texture and grow with larger textures: outside is how far the outline stands off the art, inside how far in the inner ring runs.",
        );
        let builtin = crate::prefs::MeshPreset::built_in();
        let mut presets = builtin.clone();
        presets.extend(self.prefs.mesh_presets.iter().cloned());
        let o = &mut self.tool_opts.auto;
        // the preset these numbers came from, by name, and whether they
        // still match it
        let named = presets.iter().position(|p| p.name == o.name);
        let edited = named.and_then(|i| presets.get(i)).is_none_or(|p| !p.same_values(o));
        let label = |i: usize, p: &crate::prefs::MeshPreset| {
            let tr = crate::i18n::tr;
            match p.name.as_str() {
                _ if i >= builtin.len() => p.name.clone(),
                "Standard" => tr("Standard").to_string(),
                "Deformation (small)" => tr("Deformation (small)").to_string(),
                "Deformation (large)" => tr("Deformation (large)").to_string(),
                "Face standard" => tr("Face standard").to_string(),
                other => other.to_string(),
            }
        };
        let mut shown = named
            .and_then(|i| presets.get(i).map(|p| label(i, p)))
            .unwrap_or_else(|| crate::i18n::tr("Custom").to_string());
        if named.is_some() && edited {
            shown = crate::i18n::trf("{name} (changed)", &[("name", &shown)]);
        }
        let user = named.filter(|&i| i >= builtin.len()).map(|i| i - builtin.len());
        let (mut add, mut delete, mut overwrite) = (false, false, false);
        egui::ComboBox::from_id_salt("td-auto-preset").width(180.0).selected_text(shown).show_ui(
            ui,
            |ui| {
                for (i, p) in presets.iter().enumerate() {
                    if ui.selectable_label(named == Some(i) && !edited, label(i, p)).clicked() {
                        *o = p.clone();
                    }
                }
            },
        );
        ui.horizontal(|ui| {
            add = ui.button(crate::i18n::tr("Add…")).clicked();
            delete = ui
                .add_enabled(user.is_some(), egui::Button::new(crate::i18n::tr("Delete")))
                .on_hover_text(crate::i18n::tr("Remove this preset (the built-in ones stay)"))
                .clicked();
            overwrite = ui
                .add_enabled(
                    user.is_some() && edited,
                    egui::Button::new(crate::i18n::tr("Overwrite")),
                )
                .on_hover_text(crate::i18n::tr("Save the values below into this preset"))
                .clicked();
        });
        if add {
            self.tool_opts.auto_new_name = Some(String::new());
        }
        let mut save = false;
        let o = &mut self.tool_opts.auto;
        if let Some(name) = &mut self.tool_opts.auto_new_name {
            let mut done = None;
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(name)
                        .hint_text(crate::i18n::tr("Preset name"))
                        .desired_width(120.0),
                );
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button(crate::i18n::tr("OK")).clicked() || enter {
                    done = Some(true);
                }
                if ui.button(crate::i18n::tr("Cancel")).clicked() {
                    done = Some(false);
                }
            });
            if let Some(ok) = done {
                let name = name.trim().to_string();
                if ok && !name.is_empty() && !builtin.iter().any(|p| p.name == name) {
                    o.name = name.clone();
                    let preset = o.clone();
                    match self.prefs.mesh_presets.iter_mut().find(|p| p.name == name) {
                        Some(p) => *p = preset,
                        None => self.prefs.mesh_presets.push(preset),
                    }
                    save = true;
                }
                self.tool_opts.auto_new_name = None;
            }
        }
        let o = &mut self.tool_opts.auto;
        if let Some(u) = user {
            if delete && u < self.prefs.mesh_presets.len() {
                self.prefs.mesh_presets.remove(u);
                save = true;
            } else if overwrite && let Some(p) = self.prefs.mesh_presets.get_mut(u) {
                *p = o.clone();
                save = true;
            }
        }
        if save {
            self.prefs.save();
        }
        grid(ui, "td-auto", |ui| {
            let px = |ui: &mut Ui, v: &mut f32, max: f32| {
                ui.add(egui::DragValue::new(v).range(0.0..=max).speed(0.5));
            };
            row(ui, crate::i18n::tr("Vertices interval (outside)"), |ui| {
                px(ui, &mut o.interval_outside, 512.0)
            });
            row(ui, crate::i18n::tr("Vertices interval (inside)"), |ui| {
                px(ui, &mut o.interval_inside, 512.0)
            });
            row(ui, crate::i18n::tr("Boundary margin (outside)"), |ui| {
                px(ui, &mut o.margin_outside, 256.0)
            });
            row(ui, crate::i18n::tr("Boundary margin (inside)"), |ui| {
                px(ui, &mut o.margin_inside, 256.0)
            });
            row(ui, crate::i18n::tr("Minimum boundary margin"), |ui| {
                px(ui, &mut o.min_margin, 256.0)
            });
            row(ui, crate::i18n::tr("Minimum number of points on a boundary"), |ui| {
                ui.add(egui::DragValue::new(&mut o.min_points).range(3..=64));
            });
            row(ui, crate::i18n::tr("Alpha value determined as transparent"), |ui| {
                ui.add(egui::DragValue::new(&mut o.alpha).range(0..=254));
            });
            row(ui, crate::i18n::tr("Face layout"), |ui| {
                ui.checkbox(&mut o.face, "").on_hover_text(crate::i18n::tr(
                    "A second ring hugging the drawn edge instead of Cubism's inner ring, as \
                     riggers build faces",
                ));
            });
        });
    }

    fn td_glue(&mut self, ui: &mut Ui, action: &mut Option<MeshAction>) {
        section(
            ui,
            crate::i18n::tr("Edit glue"),
            "Select two meshes and Bind (Ctrl+G) to glue their overlapping points, or pick a glue in the Part palette.\n\nA:B drags the weight toward red A (Shift+drag toward green B). A and B change one side only; Re-merge rejoins points they pulled apart.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (mode, glyph, tip) in [
                (0u8, ph::SCALES, "Glue weight A:B: drag favours red A, Shift+drag green B"),
                (1, ph::CIRCLE_HALF, "Glue weight A: only the red mesh's weight"),
                (2, ph::CIRCLE_HALF_TILT, "Glue weight B: only the green mesh's weight"),
                (3, ph::ARROWS_IN_SIMPLE, "Re-merge: rejoin points A or B pulled apart"),
            ] {
                if icon(ui, self.glue_mode == mode, true, glyph, tip).clicked() {
                    self.glue_mode = mode;
                }
            }
        });
        grid(ui, "td-glue", |ui| {
            row(ui, crate::i18n::tr("Weight"), |ui| {
                ui.add(
                    egui::Slider::new(&mut self.glue_strength, 0.01..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });
            row(ui, crate::i18n::tr("Range"), |ui| {
                ui.add(
                    egui::Slider::new(&mut self.tool_opts.brush_size, 4.0..=400.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
            });
            row(ui, crate::i18n::tr("Opacity of weight"), |ui| {
                ui.add_enabled(false, egui::Slider::new(&mut 1.0f32, 0.0..=1.0))
                    .on_disabled_hover_text(NOT_YET);
            });
            row(ui, crate::i18n::tr("Bind distance"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.glue_tolerance).range(0.1..=64.0).suffix(" px"),
                )
                .on_hover_text(crate::i18n::tr("How close two points must be to bind"));
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon(ui, false, true, ph::LINK, "Bind glue (Ctrl+G, two meshes selected)").clicked()
            {
                *action = Some(MeshAction::Bind);
            }
            if icon(ui, false, true, ph::LINK_BREAK, "Unbind glue").clicked() {
                *action = Some(MeshAction::Unbind);
            }
        });
        if let Some(gi) = self.selected_glue {
            ui.weak(crate::i18n::trf("Editing {name}", &[("name", &format!("Glue{gi}"))]));
        }
    }

    fn td_path(&mut self, ui: &mut Ui) {
        section(
            ui,
            crate::i18n::tr("Deform path edit"),
            "Click on the selected mesh to add a control point, drag one to move it, Alt+click deletes, Backspace removes the last. Placing a path deforms nothing.\n\nSwitch to the arrow tool and drag a control point: points inside the red circle follow the path, keeping their shape fully inside the blue one. Edits go into the keyform the parameters sit on.\n\nEdit levels 2 and 3 each keep their own path for the same mesh (for example opening and closing a mouth); level 1 hides paths.",
        );
        let key = match self.selected {
            Some(Sel::Mesh(mi)) => Some((mi, self.edit_level)),
            _ => None,
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let _ = icon(ui, true, true, ph::PLUS_CIRCLE, "Add control point: click on the mesh");
            if icon(
                ui,
                false,
                key.is_some(),
                ph::MINUS_CIRCLE,
                "Delete control point: the last one (Backspace), or Alt+click any",
            )
            .clicked()
                && let Some(k) = key
                && let Some(list) = self.deform_paths.get_mut(&k)
            {
                list.pop();
            }
            if icon(ui, false, key.is_some(), ph::TRASH, "Delete path (this mesh, this edit level)")
                .clicked()
                && let Some(k) = key
            {
                self.deform_paths.remove(&k);
            }
            let _ = icon(ui, false, false, ph::LINE_SEGMENTS, "Fold line");
        });
        grid(ui, "td-path", |ui| {
            row(ui, crate::i18n::tr("Deform path width"), |ui| {
                ui.add(
                    egui::Slider::new(&mut self.path_width, 4.0..=600.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
            });
            row(ui, crate::i18n::tr("Hardness"), |ui| {
                ui.add(
                    egui::Slider::new(&mut self.path_hardness, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });
        });
        if self.edit_level < 2 {
            ui.weak(crate::i18n::tr("Edit level 1 hides deform paths; press 2 or 3."));
        }
    }

    fn td_create(&mut self, ui: &mut Ui) {
        let warp = self.tool == Tool::WarpCreate;
        section(
            ui,
            if warp {
                crate::i18n::tr("New warp deformer")
            } else {
                crate::i18n::tr("New rotation deformer")
            },
            if warp {
                "Create opens the dialog: part, name, and whether the deformer wraps the selection, goes under the selected deformer, or under a parent you pick. Clicking the canvas with objects selected opens it too."
            } else {
                "Click the canvas to place the origin, or use Create. The dialog picks the part, name and parent."
            },
        );
        if warp {
            let o = &mut self.tool_opts;
            grid(ui, "td-create", |ui| {
                row(ui, crate::i18n::tr("Conversion division"), |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut o.warp_cols).range(1..=32));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut o.warp_rows).range(1..=32));
                    });
                });
            });
        }
        if ui.button(crate::i18n::tr("Create…")).clicked() {
            self.pending_create = Some((warp, None));
        }
    }

    fn td_artpath(&mut self, ui: &mut Ui) {
        section(
            ui,
            crate::i18n::tr("ArtPath"),
            "Click to place a corner, press and drag for a smooth point (Alt+drag breaks its handles). Click the first point to close the path; Enter finishes an open one, Esc drops it, Backspace takes back the last point.\n\nThe path becomes an ordinary ArtMesh: clip by it, put it under deformers, give it keys. Select an ArtPath mesh with this tool to drag its points and handles; Alt+click deletes a point.",
        );
        let selected = match self.selected {
            Some(Sel::Mesh(mi)) if self.art_draft.is_none() => {
                self.session.as_ref().and_then(|s| {
                    let m = s.puppet.model();
                    let keyed = m.art_meshes.get(mi).is_some_and(|a| a.binding.keyform_count() > 1);
                    ayatsuri_core::edit::art_path_of(m, mi).cloned().map(|p| (mi, p, keyed))
                })
            }
            _ => None,
        };
        let mut p = match &selected {
            Some((mi, p, _)) => match &self.art_pending {
                Some((m, q)) if m == mi => q.clone(),
                _ => p.clone(),
            },
            None => self.art_draft.clone().unwrap_or_else(|| self.art_style.clone()),
        };
        let before = p.clone();
        let mut settled = false;
        grid(ui, "td-artpath", |ui| {
            row(ui, crate::i18n::tr("Stroke"), |ui| {
                settled |= ui.checkbox(&mut p.stroke, "").changed();
                let _ = ui.color_edit_button_srgba_unmultiplied(&mut p.stroke_color);
                if icon(
                    ui,
                    self.art_pick == Some(0),
                    true,
                    ph::EYEDROPPER,
                    "Pick the stroke colour from the canvas (Esc cancels)",
                )
                .clicked()
                {
                    self.art_pick = if self.art_pick == Some(0) { None } else { Some(0) };
                }
            });
            row(ui, crate::i18n::tr("Width"), |ui| {
                let r = ui.add(
                    egui::Slider::new(&mut p.width, 0.5..=200.0).logarithmic(true).suffix(" px"),
                );
                settled |= r.drag_stopped() || (r.changed() && !r.dragged());
            });
            row(ui, crate::i18n::tr("Fill"), |ui| {
                settled |= ui.checkbox(&mut p.fill, "").changed();
                let _ = ui.color_edit_button_srgba_unmultiplied(&mut p.fill_color);
                if icon(
                    ui,
                    self.art_pick == Some(1),
                    true,
                    ph::EYEDROPPER,
                    "Pick the fill colour from the canvas (Esc cancels)",
                )
                .clicked()
                {
                    self.art_pick = if self.art_pick == Some(1) { None } else { Some(1) };
                }
            });
            row(ui, crate::i18n::tr("Closed"), |ui| {
                settled |= ui.checkbox(&mut p.closed, "").changed();
            });
            row(ui, crate::i18n::tr("Soft edge"), |ui| {
                let r = ui.add(egui::Slider::new(&mut p.feather, 0.0..=8.0).suffix(" px"));
                settled |= r.drag_stopped() || (r.changed() && !r.dragged());
            });
            row(ui, crate::i18n::tr("Mask only"), |ui| {
                settled |= ui
                    .checkbox(&mut p.mask_only, "")
                    .on_hover_text(crate::i18n::tr(
                        "Don't draw the path (fill or stroke), only clip with it. Turns on when the path becomes a clipping mask.",
                    ))
                    .changed();
            });
            row(ui, crate::i18n::tr("Segment length"), |ui| {
                let r = ui.add(egui::Slider::new(&mut p.step, 1.0..=30.0).suffix(" px"));
                settled |= r.drag_stopped() || (r.changed() && !r.dragged());
            });
        });
        if p.fill && !p.closed {
            ui.weak(crate::i18n::tr("Fill shows once the path is closed."));
        }
        if let Some((mi, path, _)) = &selected
            && ui
                .button(crate::i18n::tr("Redraw from path"))
                .on_hover_text(crate::i18n::tr(
                    "Rebuild this mesh from its path points (fixes a path whose mesh was edited by hand)",
                ))
                .clicked()
        {
            let (mi, path) = (*mi, path.clone());
            let ctx = ui.ctx().clone();
            if let Some(s) = &mut self.session
                && s.art_path_edit(&ctx, |m, _| ayatsuri_core::edit::update_art_path(m, mi, path))
            {
                log::info!("ArtPath {mi} redrawn from its path");
            }
        }
        if let Some((mi, _, _)) = &selected {
            let on = self.clip_with == Some(*mi);
            if ui
                .selectable_label(on, crate::i18n::tr("Clip meshes with this path"))
                .on_hover_text(crate::i18n::tr(
                    "Then click meshes on the canvas: each one shows only inside this path. Click one again to take it off; Esc ends.",
                ))
                .clicked()
            {
                self.clip_with = if on { None } else { Some(*mi) };
                self.clip_pick = None;
                self.tool = Tool::Arrow;
            }
            ui.checkbox(&mut self.clip_invert, crate::i18n::tr("Inverted"))
                .on_hover_text(crate::i18n::tr(
                    "Clipped meshes show only outside the path (Alt+click flips it per mesh). Inverting applies to all of a mesh's masks, as in Cubism.",
                ));
        }
        if selected.as_ref().is_some_and(|(_, _, keyed)| *keyed) {
            ui.weak(crate::i18n::tr(
                "This path has keys: changing it redraws every keyform in its rest shape.",
            ));
        }
        let picker_open =
            egui::Popup::is_any_open(ui.ctx()) || ui.ctx().input(|i| i.pointer.any_down());
        let colors_moved =
            p.stroke_color != before.stroke_color || p.fill_color != before.fill_color;
        let pending = self.art_pending.is_some();
        if !picker_open && (colors_moved || pending) {
            settled = true;
        }
        match &selected {
            Some((mi, committed, _)) => {
                let mi = *mi;
                if *committed != p && !settled {
                    self.art_pending = Some((mi, p.clone()));
                } else if *committed != p && settled {
                    self.art_pending = None;
                    let ctx = ui.ctx().clone();
                    if let Some(s) = &mut self.session
                        && s.art_path_edit(&ctx, |m, _| {
                            ayatsuri_core::edit::update_art_path(m, mi, p)
                        })
                    {
                        log::info!("ArtPath {mi} restyled");
                    }
                } else {
                    self.art_pending = None;
                }
            }
            None => {
                if p != before {
                    if self.art_draft.is_some() {
                        self.art_draft = Some(p.clone());
                    }
                    self.art_style = ayatsuri_format::model::ArtPath {
                        anchors: Vec::new(),
                        mesh: String::new(),
                        ..p
                    };
                }
                if let Some(d) = &self.art_draft {
                    let n = d.anchors.len();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(n >= 2, egui::Button::new(crate::i18n::tr("Finish path")))
                            .clicked()
                        {
                            self.art_finish = true;
                        }
                        if ui.button(crate::i18n::tr("Cancel")).clicked() {
                            self.art_draft = None;
                        }
                    });
                }
            }
        }
    }

    fn td_brush(&mut self, ui: &mut Ui) {
        if self.tool == Tool::DeformBrush {
            section(
                ui,
                crate::i18n::tr("Deform Brush"),
                "Drag over the selected meshes, or a selected warp deformer's grid, to push their points along the stroke. Points inside the hard core move fully, the rest less toward the edge. The art follows as you paint; the stroke lands in the keyform the parameters sit on, as one undo step.\n\nB + drag or [ and ] size the brush.",
            );
            grid(ui, "td-dbrush", |ui| {
                row(ui, crate::i18n::tr("Brush size"), |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.tool_opts.brush_size, 4.0..=400.0)
                            .logarithmic(true)
                            .suffix(" px"),
                    );
                });
                row(ui, crate::i18n::tr("Strength"), |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.dbrush_strength, 0.05..=1.0)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    );
                });
                row(ui, crate::i18n::tr("Hardness"), |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.dbrush_hardness, 0.0..=0.95)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    );
                });
            });
            return;
        }
        section(
            ui,
            crate::i18n::tr("Brush selection"),
            "Paint a soft selection onto the selected meshes' points, or a selected warp's division points. Drag adds weight; Alt+drag takes it away; B+drag sizes the brush; Esc clears.\n\nThen switch to the arrow tool: dragging moves every painted point by its weight, and the box scales and rotates them the same way. Edits land in the keyform the parameters sit on.",
        );
        grid(ui, "td-brush", |ui| {
            row(ui, crate::i18n::tr("Weight"), |ui| {
                ui.add(
                    egui::Slider::new(&mut self.brush_weight, 0.01..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });
            row(ui, crate::i18n::tr("Brush size"), |ui| {
                ui.add(
                    egui::Slider::new(&mut self.tool_opts.brush_size, 2.0..=400.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
            });
        });
        if ui.button(crate::i18n::tr("Clear selection")).clicked() {
            self.brush_w.clear();
        }
    }
}
