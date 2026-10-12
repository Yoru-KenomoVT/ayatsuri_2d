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
fn section(ui: &mut Ui, title: &str, info: &str) {
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
                .on_hover_text("How this works");
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
fn icon(ui: &mut Ui, on: bool, enabled: bool, glyph: &str, tip: &str) -> egui::Response {
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
    let tip = if enabled { tip.to_string() } else { format!("{tip} ({NOT_YET})") };
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

fn greyed(ui: &mut Ui, label: &str) {
    ui.add_enabled(false, egui::Checkbox::new(&mut false, label)).on_disabled_hover_text(NOT_YET);
}

impl EditorUi {
    pub(super) fn tool_details_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(self.tool.label()).size(12.5).strong());
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
                Tool::ArtPath => td_artpath(ui),
                Tool::TextureAtlas => {
                    section(ui, "Texture atlas", "The texture atlas editor is not built yet.");
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
            "Selection",
            "Click selects. Shift or Ctrl+click adds to the selection. Alt+click walks up the deformer chain.\n\nLasso: with a mesh or warp selected, draw around points to pick them (Shift adds), then use the box to move, scale or rotate them. With nothing selected it selects whole meshes.",
        );

        section(
            ui,
            "Warp deformer",
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
            grid(ui, "td-warp", |ui| {
                row(ui, "Conversion division", |ui| {
                    ui.label(format!("{cols} × {rows}"));
                });
                row(ui, "Bezier division", |ui| {
                    ui.add_enabled_ui(self.edit_level == 2, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut bw).range(1..=cols.max(1)));
                            ui.label("×");
                            ui.add(egui::DragValue::new(&mut bh).range(1..=rows.max(1)));
                        });
                    });
                });
            });
            if (bw, bh) != cur {
                self.bezier_div.insert(di, (bw, bh));
            }
        }

        self.overlay_settings(ui);
    }

    /// The overlay display settings, shared by Tool Details and Settings >
    /// View. Changes are saved straight away.
    pub(super) fn overlay_settings(&mut self, ui: &mut Ui) {
        let rgba = |c: Color32| {
            let [r, g, b, a] = c.to_srgba_unmultiplied();
            Some([r, g, b, a])
        };
        let mut d = self.disp;
        let mut changed = false;
        section(
            ui,
            "ArtMesh display settings",
            "Mesh line colour follows your primary colour until you pick one here.",
        );
        grid(ui, "td-disp-mesh", |ui| {
            row(ui, "Mesh line width", |ui| {
                if ui
                    .add(egui::DragValue::new(&mut d.mesh_width).range(0.5..=4.0).speed(0.05))
                    .changed()
                {
                    self.prefs.overlay.mesh_width = Some(d.mesh_width);
                    changed = true;
                }
            });
            row(ui, "Mesh line color", |ui| {
                if ui.color_edit_button_srgba(&mut d.mesh_color).changed() {
                    self.prefs.overlay.mesh_color = rgba(d.mesh_color);
                    changed = true;
                }
            });
        });
        section(ui, "Warp deformer display settings", "");
        grid(ui, "td-disp-warp", |ui| {
            row(ui, "Grid color", |ui| {
                if ui.color_edit_button_srgba(&mut d.warp_color).changed() {
                    self.prefs.overlay.warp_color = rgba(d.warp_color);
                    changed = true;
                }
            });
        });
        section(ui, "Rotation deformer display settings", "");
        grid(ui, "td-disp-rot", |ui| {
            row(ui, "Handle color", |ui| {
                if ui.color_edit_button_srgba(&mut d.rotation_color).changed() {
                    self.prefs.overlay.rotation_color = rgba(d.rotation_color);
                    changed = true;
                }
            });
        });
        section(ui, "Brush selection display settings", "");
        grid(ui, "td-disp-weight", |ui| {
            row(ui, "Weight color", |ui| {
                if ui.color_edit_button_srgba(&mut d.weight_color).changed() {
                    self.prefs.overlay.weight_color = rgba(d.weight_color);
                    changed = true;
                }
            });
        });
        if ui.small_button("Reset display settings").clicked() {
            self.prefs.overlay = crate::prefs::Overlay::default();
            changed = true;
        }
        if changed {
            self.prefs.save();
            self.disp = DisplayOpts::from_prefs(&self.prefs.overlay);
        }
    }

    fn td_mesh(&mut self, ui: &mut Ui, action: &mut Option<MeshAction>) {
        section(
            ui,
            "Edit mesh manually",
            "Select / Edit drags points. Eraser deletes everything it sweeps ([ and ] resize it).\n\nThe actions work on the mesh being edited, or on the selected meshes.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon(ui, self.mesh_mode == 0, true, ph::CURSOR, "Select / Edit: drag points")
                .clicked()
            {
                self.mesh_mode = 0;
            }
            let _ = icon(ui, false, false, ph::LASSO, "Lasso: pick points with a lasso");
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
            let _ =
                icon(ui, false, false, ph::PAINT_BRUSH, "Add more vertices: paint to add points");
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
        });
        grid(ui, "td-mesh", |ui| {
            row(ui, "Auto connect", |ui| {
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.connect_rebuild, "Mesh reconstruction").on_hover_text(
                        "On: redo every face from the points. Off: keep the lines you built",
                    );
                    ui.checkbox(&mut self.connect_protect, "Protect edge exterior").on_hover_text(
                        "No faces over empty texture, so hair and lashes don't get bridged",
                    );
                });
            });
            row(ui, "New points", |ui| {
                ui.checkbox(&mut self.mesh_autoconnect, "Connect automatically");
            });
            row(ui, "Selection range on click", |ui| {
                ui.add(egui::DragValue::new(&mut self.pick_radius).range(2.0..=40.0).suffix(" px"));
            });
        });

        section(ui, "Vertices and edges", "");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (mode, glyph, tip) in [
                (1u8, ph::PLUS_CIRCLE, "Add vertex"),
                (2, ph::MINUS_CIRCLE, "Delete vertex"),
                (3, ph::LINE_SEGMENT, "Connect two points with an edge"),
                (4, ph::SCISSORS, "Delete edge (line)"),
            ] {
                if icon(ui, self.mesh_mode == mode, true, glyph, tip).clicked() {
                    self.mesh_mode = mode;
                }
            }
        });
        if self.mesh_mode == 5 {
            section(ui, "Eraser", "");
            grid(ui, "td-eraser", |ui| {
                row(ui, "Size", |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.tool_opts.brush_size, 4.0..=400.0)
                            .logarithmic(true)
                            .suffix(" px"),
                    );
                });
            });
        }

        section(ui, "Mirror edit", "");
        greyed(ui, "Mirror edit");

        section(
            ui,
            "Automatic mesh",
            "Traces the outline of the layer's pixels and fills it with points. Outline spacing sets the gap between outline points, inner spacing the gap inside, margin how far the outline stands off the art.",
        );
        let o = &mut self.tool_opts;
        ui.horizontal_wrapped(|ui| {
            for (i, (name, outline, inner, margin)) in Self::AUTO_PRESETS.iter().enumerate() {
                if ui.selectable_label(o.auto_preset == i, *name).clicked() {
                    o.auto_preset = i;
                    o.auto_spacing = *outline;
                    o.auto_inner = *inner;
                    o.auto_margin = *margin;
                }
            }
        });
        grid(ui, "td-auto", |ui| {
            row(ui, "Outline spacing", |ui| {
                ui.add(egui::DragValue::new(&mut o.auto_spacing).range(4.0..=256.0).suffix(" px"));
            });
            row(ui, "Inner spacing", |ui| {
                ui.add(egui::DragValue::new(&mut o.auto_inner).range(4.0..=512.0).suffix(" px"));
            });
            row(ui, "Margin", |ui| {
                ui.add(egui::DragValue::new(&mut o.auto_margin).range(0.0..=128.0).suffix(" px"));
            });
        });
    }

    fn td_glue(&mut self, ui: &mut Ui, action: &mut Option<MeshAction>) {
        section(
            ui,
            "Edit glue",
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
            row(ui, "Weight", |ui| {
                ui.add(
                    egui::Slider::new(&mut self.glue_strength, 0.01..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });
            row(ui, "Range", |ui| {
                ui.add(
                    egui::Slider::new(&mut self.tool_opts.brush_size, 4.0..=400.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
            });
            row(ui, "Opacity of weight", |ui| {
                ui.add_enabled(false, egui::Slider::new(&mut 1.0f32, 0.0..=1.0))
                    .on_disabled_hover_text(NOT_YET);
            });
            row(ui, "Bind distance", |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.glue_tolerance).range(0.1..=64.0).suffix(" px"),
                )
                .on_hover_text("How close two points must be to bind");
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
            ui.weak(format!("Editing Glue{gi}"));
        }
    }

    fn td_path(&mut self, ui: &mut Ui) {
        section(
            ui,
            "Deform path edit",
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
            row(ui, "Deform path width", |ui| {
                ui.add(
                    egui::Slider::new(&mut self.path_width, 4.0..=600.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
            });
            row(ui, "Hardness", |ui| {
                ui.add(
                    egui::Slider::new(&mut self.path_hardness, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });
        });
        if self.edit_level < 2 {
            ui.weak("Edit level 1 hides deform paths; press 2 or 3.");
        }
    }

    fn td_create(&mut self, ui: &mut Ui) {
        let warp = self.tool == Tool::WarpCreate;
        section(
            ui,
            if warp { "New warp deformer" } else { "New rotation deformer" },
            if warp {
                "Create opens the dialog: part, name, and whether the deformer wraps the selection, goes under the selected deformer, or under a parent you pick. Clicking the canvas with objects selected opens it too."
            } else {
                "Click the canvas to place the origin, or use Create. The dialog picks the part, name and parent."
            },
        );
        if warp {
            let o = &mut self.tool_opts;
            grid(ui, "td-create", |ui| {
                row(ui, "Conversion division", |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut o.warp_cols).range(1..=32));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut o.warp_rows).range(1..=32));
                    });
                });
            });
        }
        if ui.button("Create…").clicked() {
            self.pending_create = Some((warp, None));
        }
    }

    fn td_brush(&mut self, ui: &mut Ui) {
        if self.tool == Tool::DeformBrush {
            section(ui, "Deform brushes", "The deform brushes are not built yet.");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let _ = icon(ui, false, false, ph::PAINT_BRUSH, "Deform brush");
                let _ = icon(ui, false, false, ph::GRID_NINE, "Warp deformer shaping brush");
            });
            return;
        }
        section(
            ui,
            "Brush selection",
            "Paint a soft selection onto the selected meshes' points, or a selected warp's division points. Drag adds weight; Alt+drag takes it away; B+drag sizes the brush; Esc clears.\n\nThen switch to the arrow tool: dragging moves every painted point by its weight, and the box scales and rotates them the same way. Edits land in the keyform the parameters sit on.",
        );
        grid(ui, "td-brush", |ui| {
            row(ui, "Weight", |ui| {
                ui.add(
                    egui::Slider::new(&mut self.brush_weight, 0.01..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });
            row(ui, "Brush size", |ui| {
                ui.add(
                    egui::Slider::new(&mut self.tool_opts.brush_size, 2.0..=400.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
            });
        });
        if ui.button("Clear selection").clicked() {
            self.brush_w.clear();
        }
    }
}

fn td_artpath(ui: &mut Ui) {
    section(ui, "ArtPath tools", "ArtPaths are not built yet.");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (glyph, tip) in [
            (ph::PLUS_CIRCLE, "Add control point"),
            (ph::MINUS_CIRCLE, "Delete control point"),
            (ph::LINE_SEGMENTS, "Fold line"),
            (ph::DROP_HALF, "Control point opacity"),
            (ph::LINE_VERTICAL, "Control point thickness"),
            (ph::PEN_NIB, "Stroke ArtPath"),
            (ph::PENCIL_LINE, "Re-stroke ArtPath"),
        ] {
            let _ = icon(ui, false, false, glyph, tip);
        }
    });
}
