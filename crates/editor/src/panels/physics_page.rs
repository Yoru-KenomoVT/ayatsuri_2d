// The Physics Settings page, laid out after Cubism's Physics Settings
// window (docs.live2d.com, "About Physics"): a menu bar, the playlist bar,
// Physics / Playlist tabs, then the settings column, the parameter rail and
// the running model side by side, each column resizable. Edits apply as
// they are made, one undo step per gesture, the way Cubism's window works.

use ayatsuri_format::model::{
    PhysicsGroup, PhysicsInput, PhysicsIo, PhysicsNode, PhysicsOutput, PhysicsRig,
};
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use super::{
    EditorUi, ScrollArea, default_physics_group, input_preset, rig_groups_reach, truncate,
};
use crate::theme;

const NOT_YET: &str = "Not built yet";

/// Column widths and table selections for the page.
pub(super) struct PhysPage {
    pub(super) editor_w: f32,
    pub(super) rail_w: f32,
    in_sel: Option<usize>,
    out_sel: Option<usize>,
    node_sel: Option<usize>,
    out_clip: Vec<PhysicsOutput>,
    /// Group editing dialog: (name, id).
    group_edit: Option<(String, String)>,
    /// Preview rotation in degrees (Cubism's 45 degree turns).
    pub(super) prev_rot: f32,
    /// Heights of the settings panels; the preview takes the rest.
    h_tabs: f32,
    w_model: f32,
    /// Outputs to move into new groups, from the output right-click menu.
    split: Option<Vec<Vec<usize>>>,
}

impl Default for PhysPage {
    fn default() -> Self {
        Self {
            editor_w: 560.0,
            rail_w: 290.0,
            in_sel: None,
            out_sel: None,
            node_sel: None,
            out_clip: Vec::new(),
            group_edit: None,
            prev_rot: 0.0,
            split: None,
            h_tabs: 300.0,
            w_model: 470.0,
        }
    }
}

fn group_label(g: &PhysicsGroup) -> String {
    if g.name.is_empty() { g.id.clone() } else { g.name.clone() }
}

/// A small table header cell.
fn head(ui: &mut Ui, text: &str, w: f32) {
    ui.add_sized(
        [w, 16.0],
        egui::Label::new(egui::RichText::new(text).size(10.5).color(theme::pal().text_faint)),
    );
}

/// A row-select cell: highlights when selected, returns true on click.
fn row_pick(ui: &mut Ui, selected: bool, text: &str, w: f32) -> bool {
    ui.add_sized([w, 18.0], egui::Button::selectable(selected, text)).clicked()
}

/// Move element `i` one step up or down, returning its new index.
fn shift<T>(v: &mut [T], i: usize, up: bool) -> usize {
    if up && i > 0 && i < v.len() {
        v.swap(i, i - 1);
        i - 1
    } else if !up && i + 1 < v.len() {
        v.swap(i, i + 1);
        i + 1
    } else {
        i
    }
}

fn param_combo(ui: &mut Ui, salt: String, current: &mut String, ids: &[String], w: f32) {
    egui::ComboBox::from_id_salt(salt).selected_text(truncate(current, 16)).width(w).show_ui(
        ui,
        |ui| {
            for id in ids {
                ui.selectable_value(current, id.clone(), id);
            }
        },
    );
}

fn kind_combo(ui: &mut Ui, salt: String, kind: &mut PhysicsIo) {
    let label = match kind {
        PhysicsIo::Angle => "Angle",
        PhysicsIo::X => "Position X",
        PhysicsIo::Y => "Position Y",
    };
    egui::ComboBox::from_id_salt(salt).selected_text(label).width(80.0).show_ui(ui, |ui| {
        ui.selectable_value(kind, PhysicsIo::X, "Position X");
        ui.selectable_value(kind, PhysicsIo::Angle, "Angle");
    });
}

/// A titled group, boxed the way Cubism groups its settings: a soft
/// panel, no outline. Its content always stacks top to bottom.
fn group_box<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::pal().header.gamma_multiply(0.55))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(7))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                if !title.is_empty() {
                    ui.label(egui::RichText::new(title).size(11.5).color(theme::pal().text));
                    ui.add_space(2.0);
                }
                add(ui)
            })
            .inner
        })
        .inner
}

/// One panel of the settings stack: a soft box filling `rect`, its
/// content clipped to it and scrolling both ways.
fn panel(ui: &mut Ui, rect: Rect, id: &str, add: impl FnOnce(&mut Ui)) {
    ui.painter().rect_filled(rect, 4.0, theme::pal().header.gamma_multiply(0.55));
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink(7.0)));
    child.set_clip_rect(rect.shrink(2.0));
    ScrollArea::both().id_salt(id).auto_shrink([false, false]).show(&mut child, add);
}

/// A table on its own darker panel, no outline.
fn table<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::pal().panel_bg)
        .corner_radius(3.0)
        .inner_margin(egui::Margin::same(2))
        .show(ui, add)
        .inner
}

/// Real tabs: the active one opens into the page below it.
fn tab_strip(ui: &mut Ui, labels: &[&str], active: &mut usize, enabled: &[bool]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, label) in labels.iter().enumerate() {
            let on = *active == i;
            let ok = enabled.get(i).copied().unwrap_or(true);
            let galley = ui.painter().layout_no_wrap(
                label.to_string(),
                FontId::proportional(11.5),
                theme::pal().text,
            );
            let size = vec2(galley.size().x + 22.0, 22.0);
            let (rect, resp) =
                ui.allocate_exact_size(size, if ok { Sense::click() } else { Sense::hover() });
            let fill = if on { theme::pal().panel_bg } else { theme::pal().header };
            ui.painter().rect_filled(rect, egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, fill);
            let edge = Stroke::new(1.0, theme::pal().border);
            ui.painter().line_segment([rect.left_bottom(), rect.left_top()], edge);
            ui.painter().line_segment([rect.left_top(), rect.right_top()], edge);
            ui.painter().line_segment([rect.right_top(), rect.right_bottom()], edge);
            if on {
                ui.painter().line_segment(
                    [rect.left_top() + vec2(1.0, 0.0), rect.right_top() - vec2(1.0, 0.0)],
                    Stroke::new(2.0, theme::pal().accent),
                );
            } else {
                ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], edge);
            }
            let col = if !ok {
                theme::pal().text_faint.gamma_multiply(0.5)
            } else if on {
                theme::pal().text
            } else {
                theme::pal().text_faint
            };
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                *label,
                FontId::proportional(11.5),
                col,
            );
            if ok && resp.clicked() {
                *active = i;
            }
            if !ok {
                resp.on_hover_text(NOT_YET);
            }
        }
        // the baseline the inactive tabs sit on runs to the edge
        let r = ui.available_rect_before_wrap();
        ui.painter().line_segment(
            [pos2(r.min.x, r.min.y + 22.0), pos2(r.max.x, r.min.y + 22.0)],
            Stroke::new(1.0, theme::pal().border),
        );
    });
}

impl EditorUi {
    pub(super) fn physics_page(&mut self, ui: &mut Ui) {
        let content = ui.available_rect_before_wrap();
        ui.painter().rect_filled(content, 0.0, theme::pal().chrome);

        // title strip with the way back out
        let bar = Rect::from_min_size(content.min, vec2(content.width(), 28.0));
        ui.painter().rect_filled(bar, 0.0, theme::pal().header);
        ui.painter().text(
            pos2(bar.min.x + 12.0, bar.center().y),
            Align2::LEFT_CENTER,
            "Physics Settings",
            FontId::proportional(12.5),
            theme::pal().text,
        );
        let close = Rect::from_min_size(pos2(bar.max.x - 72.0, bar.min.y + 4.0), vec2(60.0, 20.0));
        let cresp = ui.interact(close, egui::Id::new("phys-close"), Sense::click());
        ui.painter().rect_filled(close, 4.0, theme::pal().accent);
        ui.painter().text(
            close.center(),
            Align2::CENTER_CENTER,
            "Close",
            FontId::proportional(11.0),
            Color32::WHITE,
        );
        if cresp.clicked() || ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            self.close_physics_page();
            return;
        }

        // menu bar, playlist bar and the Physics / Playlist tabs
        let menus =
            Rect::from_min_size(pos2(content.min.x, bar.max.y), vec2(content.width(), 24.0));
        let mut mui = ui.new_child(UiBuilder::new().max_rect(menus.shrink2(vec2(6.0, 0.0))));
        egui::MenuBar::new().ui(&mut mui, |ui| self.physics_menus(ui));
        let tools =
            Rect::from_min_size(pos2(content.min.x, menus.max.y), vec2(content.width(), 30.0));
        let mut tui = ui.new_child(UiBuilder::new().max_rect(tools.shrink2(vec2(8.0, 3.0))));
        tui.horizontal_centered(|ui| {
            ui.add_enabled_ui(false, |ui| {
                ui.label("Playlist:");
                egui::ComboBox::from_id_salt("phys-playlist")
                    .selected_text("")
                    .width(120.0)
                    .show_ui(ui, |_| {});
                egui::ComboBox::from_id_salt("phys-pattern")
                    .selected_text("[Pattern A]")
                    .width(90.0)
                    .show_ui(ui, |_| {});
                for g in [
                    egui_phosphor::regular::PLAY,
                    egui_phosphor::regular::STOP,
                    egui_phosphor::regular::REPEAT,
                ] {
                    let _ = ui.button(g);
                }
                ui.label("Fading:");
                egui::ComboBox::from_id_salt("phys-fade")
                    .selected_text("None")
                    .width(70.0)
                    .show_ui(ui, |_| {});
                ui.label("Fading Time(ms):");
                ui.add(egui::DragValue::new(&mut 1000.0f32));
            })
            .response
            .on_disabled_hover_text("Playlists are not built yet");
        });
        let tabs =
            Rect::from_min_size(pos2(content.min.x, tools.max.y), vec2(content.width(), 24.0));
        let mut tabui = ui.new_child(UiBuilder::new().max_rect(tabs.shrink2(vec2(8.0, 2.0))));
        let mut page_tab = 0usize;
        tab_strip(&mut tabui, &["Physics", "Playlist"], &mut page_tab, &[true, false]);
        ui.painter().line_segment(
            [pos2(content.min.x, tabs.max.y), pos2(content.max.x, tabs.max.y)],
            Stroke::new(1.0, theme::pal().border),
        );

        // three columns with draggable splitters between them
        let body = Rect::from_min_max(pos2(content.min.x, tabs.max.y + 1.0), content.max);
        let total = body.width();
        self.phys_page.editor_w = self.phys_page.editor_w.clamp(300.0, (total - 420.0).max(300.0));
        self.phys_page.rail_w = self
            .phys_page
            .rail_w
            .clamp(170.0, (total - self.phys_page.editor_w - 220.0).max(170.0));
        let ex = body.min.x + self.phys_page.editor_w;
        let rx = ex + 5.0 + self.phys_page.rail_w;
        let editor_rect = Rect::from_min_max(body.min, pos2(ex, body.max.y));
        let rail_rect = Rect::from_min_max(pos2(ex + 5.0, body.min.y), pos2(rx, body.max.y));
        let view_rect = Rect::from_min_max(pos2(rx + 5.0, body.min.y), body.max);
        for (n, x) in [ex, rx].into_iter().enumerate() {
            let handle = Rect::from_min_max(pos2(x, body.min.y), pos2(x + 5.0, body.max.y));
            let r = ui.interact(handle, egui::Id::new(("phys-split", n)), Sense::drag());
            let col =
                if r.hovered() || r.dragged() { theme::pal().accent } else { theme::pal().border };
            ui.painter().line_segment(
                [pos2(x + 2.5, body.min.y), pos2(x + 2.5, body.max.y)],
                Stroke::new(1.0, col),
            );
            if r.hovered() || r.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if r.dragged() {
                let dx = r.drag_delta().x;
                if n == 0 {
                    self.phys_page.editor_w += dx;
                } else {
                    self.phys_page.rail_w += dx;
                }
            }
        }

        // each column draws only inside itself
        let mut editor_ui = ui.new_child(UiBuilder::new().max_rect(editor_rect.shrink(8.0)));
        editor_ui.set_clip_rect(editor_rect);
        self.physics_settings_column(&mut editor_ui);
        let mut rail_ui = ui.new_child(UiBuilder::new().max_rect(rail_rect.shrink(6.0)));
        rail_ui.set_clip_rect(rail_rect);
        self.physics_param_rail(&mut rail_ui);
        // the canvas here only shows the model moving: no tool, selection or
        // mesh edit reaches it, and all of them come back on the way out
        let mut view_ui = ui.new_child(UiBuilder::new().max_rect(view_rect));
        let saved = (
            self.tool,
            self.selected.take(),
            std::mem::take(&mut self.multi_sel),
            self.mesh_edit.take(),
        );
        self.tool = super::Tool::TextureAtlas;
        self.viewport(&mut view_ui, view_rect);
        self.tool = saved.0;
        self.selected = saved.1;
        self.multi_sel = saved.2;
        self.mesh_edit = saved.3;
        self.physics_group_dialog(ui);
    }

    fn physics_menus(&mut self, ui: &mut Ui) {
        ui.menu_button("Edit", |ui| {
            if ui.add(egui::Button::new("Undo").shortcut_text(self.sc("undo"))).clicked() {
                ui.close();
                if let Some(s) = &mut self.session {
                    s.undo();
                }
                self.phys_edit = None;
            }
            if ui.add(egui::Button::new("Redo").shortcut_text(self.sc("redo"))).clicked() {
                ui.close();
                if let Some(s) = &mut self.session {
                    s.redo();
                }
                self.phys_edit = None;
            }
        });
        ui.menu_button("Physics", |ui| {
            let rig = self.phys_edit.get_or_insert_with(|| {
                self.session
                    .as_ref()
                    .and_then(|s| s.puppet.model().physics.clone())
                    .unwrap_or_default()
            });
            if ui.button("Add group").clicked() {
                let n = rig.groups.len() + 1;
                rig.groups.push(default_physics_group(n));
                self.phys_sel = rig.groups.len() - 1;
                ui.close();
            }
            if ui
                .add_enabled(self.phys_sel < rig.groups.len(), egui::Button::new("Duplicate group"))
                .clicked()
            {
                if let Some(g) = rig.groups.get(self.phys_sel).cloned() {
                    rig.groups.push(PhysicsGroup {
                        id: format!("{}Copy", g.id),
                        name: format!("{} copy", group_label(&g)),
                        ..g
                    });
                    self.phys_sel = rig.groups.len() - 1;
                }
                ui.close();
            }
            if ui
                .add_enabled(self.phys_sel < rig.groups.len(), egui::Button::new("Delete group"))
                .clicked()
            {
                rig.groups.remove(self.phys_sel);
                self.phys_sel = self.phys_sel.min(rig.groups.len().saturating_sub(1));
                ui.close();
            }
            ui.separator();
            if ui.button("Initialize FPS").clicked() {
                rig.fps = 60.0;
                ui.close();
            }
        });
        ui.menu_button("Playlist", |ui| {
            ui.add_enabled(false, egui::Button::new("Playlists are not built yet"));
        });
        ui.menu_button("Preview", |ui| {
            if ui.checkbox(&mut self.physics_on, "Run physics").changed() {
                ui.close();
            }
            if ui.checkbox(&mut self.follow_cursor, "Follow the cursor").changed() {
                ui.close();
            }
            if ui
                .add(
                    egui::Button::new("Reset Parameter to Default")
                        .shortcut_text(self.sc("reset-params")),
                )
                .clicked()
            {
                ui.close();
                self.reset_parameters();
            }
            ui.separator();
            ui.label(egui::RichText::new("Wind").weak());
            let rig = self.phys_edit.get_or_insert_with(|| {
                self.session
                    .as_ref()
                    .and_then(|s| s.puppet.model().physics.clone())
                    .unwrap_or_default()
            });
            ui.add(egui::Slider::new(&mut rig.wind_x, -10.0..=10.0).text("X"));
            ui.add(egui::Slider::new(&mut rig.wind_y, -10.0..=10.0).text("Y"));
        });
        ui.menu_button("Help", |ui| {
            if ui.button("About Physics (online manual)").clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(
                    "https://docs.live2d.com/en/cubism-editor-manual/physics-operation/",
                ));
                ui.close();
            }
        });
    }

    /// The left column: FPS, group, input or output settings, and the
    /// physical model with its preview.
    fn physics_settings_column(&mut self, ui: &mut Ui) {
        let Some(saved) =
            self.session.as_ref().map(|s| s.puppet.model().physics.clone().unwrap_or_default())
        else {
            self.show_physics = false;
            return;
        };
        let param_ids: Vec<String> = self
            .session
            .as_ref()
            .map(|s| s.puppet.model().parameters.iter().map(|p| p.id.clone()).collect())
            .unwrap_or_default();
        let peaks: Vec<(usize, f32)> = if self.phys_auto_max {
            self.session
                .as_ref()
                .and_then(|s| s.puppet.physics_output_peaks(self.phys_sel))
                .unwrap_or_default()
        } else {
            self.phys_peak_cache.clone()
        };
        if self.phys_auto_max {
            self.phys_peak_cache = peaks.clone();
        }
        let group_on =
            self.session.as_ref().is_none_or(|s| s.puppet.physics_group_enabled(self.phys_sel));
        let mut rig: PhysicsRig = self.phys_edit.take().unwrap_or_else(|| saved.clone());
        let mut toggle_enable = false;
        let mut reset_peaks = false;
        ui.spacing_mut().item_spacing = vec2(5.0, 4.0);

        ui.horizontal(|ui| {
            ui.label("Calculate FPS:");
            let mut fps = if rig.fps > 0.0 { rig.fps } else { 60.0 };
            egui::ComboBox::from_id_salt("phys-fps")
                .selected_text(format!("{fps:.0}"))
                .width(56.0)
                .show_ui(ui, |ui| {
                    for v in [15.0f32, 30.0, 60.0, 120.0] {
                        ui.selectable_value(&mut fps, v, format!("{v:.0}"));
                    }
                });
            rig.fps = fps;
        });
        ui.horizontal(|ui| {
            ui.label("Group:");
            let current =
                rig.groups.get(self.phys_sel).map(group_label).unwrap_or_else(|| "(none)".into());
            egui::ComboBox::from_id_salt("phys-group")
                .selected_text(truncate(&current, 18))
                .width(140.0)
                .show_ui(ui, |ui| {
                    for (gi, g) in rig.groups.iter().enumerate() {
                        if ui
                            .selectable_label(self.phys_sel == gi, truncate(&group_label(g), 24))
                            .clicked()
                        {
                            self.phys_sel = gi;
                        }
                    }
                });
            let mut on = group_on;
            if ui
                .checkbox(&mut on, "Enable")
                .on_hover_text("Temporarily mute this group; resets when the page closes")
                .changed()
            {
                toggle_enable = true;
            }
            if ui.button("Add…").clicked() {
                let n = rig.groups.len() + 1;
                rig.groups.push(default_physics_group(n));
                self.phys_sel = rig.groups.len() - 1;
            }
            if ui
                .add_enabled(self.phys_sel < rig.groups.len(), egui::Button::new("Duplicate…"))
                .clicked()
                && let Some(g) = rig.groups.get(self.phys_sel).cloned()
            {
                rig.groups.push(PhysicsGroup {
                    id: format!("{}Copy", g.id),
                    name: format!("{} copy", group_label(&g)),
                    ..g
                });
                self.phys_sel = rig.groups.len() - 1;
            }
            if ui
                .add_enabled(self.phys_sel < rig.groups.len(), egui::Button::new("Group editing…"))
                .clicked()
                && let Some(g) = rig.groups.get(self.phys_sel)
            {
                self.phys_page.group_edit = Some((g.name.clone(), g.id.clone()));
            }
        });
        ui.add_space(4.0);

        let reach = rig_groups_reach(&rig, self.phys_sel);
        let sel = self.phys_sel;
        let Some(g) = rig.groups.get_mut(sel) else {
            ui.weak("Add a group to begin (Add… above, or Physics > Add group).");
            self.phys_edit = Some(rig);
            return;
        };

        // Input / Output settings across the top; Physics model settings and
        // the pendulum preview side by side below. Drag the dividers to
        // resize: the one under the top box, and the one between the two
        // lower boxes. Each box scrolls inside itself.
        let area = ui.available_rect_before_wrap();
        let (x0, x1) = (area.min.x, area.max.x);
        let gap = 7.0;
        // clamped for this frame only: storing it would let a tiny first
        // frame shrink the boxes for good
        let h_tabs = self.phys_page.h_tabs.clamp(80.0, (area.height() - gap - 120.0).max(80.0));
        let w_model = self.phys_page.w_model.clamp(160.0, (area.width() - gap - 120.0).max(160.0));
        let r_tabs = Rect::from_min_size(area.min, vec2(area.width(), h_tabs));
        let low_y = r_tabs.max.y + gap;
        let r_model = Rect::from_min_max(pos2(x0, low_y), pos2(x0 + w_model, area.max.y));
        let r_prev = Rect::from_min_max(pos2(r_model.max.x + gap, low_y), pos2(x1, area.max.y));
        let splits = [
            (Rect::from_min_max(pos2(x0, r_tabs.max.y), pos2(x1, low_y)), true),
            (Rect::from_min_max(pos2(r_model.max.x, low_y), pos2(r_prev.min.x, area.max.y)), false),
        ];
        for (n, (handle, vertical)) in splits.into_iter().enumerate() {
            let r = ui.interact(handle, egui::Id::new(("phys-split2", n)), Sense::drag());
            let col =
                if r.hovered() || r.dragged() { theme::pal().accent } else { theme::pal().border };
            let c = handle.center();
            let line = if vertical {
                [pos2(x0 + 30.0, c.y), pos2(x1 - 30.0, c.y)]
            } else {
                [pos2(c.x, handle.min.y + 30.0), pos2(c.x, handle.max.y - 30.0)]
            };
            ui.painter().line_segment(line, Stroke::new(1.0, col));
            if r.hovered() || r.dragged() {
                ui.ctx().set_cursor_icon(if vertical {
                    egui::CursorIcon::ResizeVertical
                } else {
                    egui::CursorIcon::ResizeHorizontal
                });
            }
            if r.dragged() {
                if vertical {
                    self.phys_page.h_tabs = h_tabs + r.drag_delta().y;
                } else {
                    self.phys_page.w_model = w_model + r.drag_delta().x;
                }
            }
        }

        panel(ui, r_tabs, "phys-p-tabs", |ui| {
            tab_strip(
                ui,
                &["Input settings", "Output settings"],
                &mut self.phys_tab,
                &[true, true],
            );
            ui.add_space(4.0);
            if self.phys_tab == 0 {
                self.physics_inputs(ui, g, &param_ids);
            } else {
                reset_peaks |= self.physics_outputs(ui, g, &param_ids, &peaks);
            }
        });
        panel(ui, r_model, "phys-p-model", |ui| {
            ui.label(egui::RichText::new("Physics model settings").size(11.5));
            self.physics_model(ui, g);
        });
        let nodes_copy = g.nodes.clone();
        let prev_size = vec2((r_prev.width() - 16.0).max(60.0), (r_prev.height() - 52.0).max(60.0));
        panel(ui, r_prev, "phys-p-prev", |ui| {
            ui.label(egui::RichText::new("Pendulum preview").size(11.5));
            self.pendulum_preview(ui, reach, &nodes_copy, prev_size);
        });

        // a separate-as-group request from the output menu
        if let Some(split) = self.phys_page.split.take() {
            let base = rig.groups.get(sel).cloned();
            if let Some(base) = base {
                let mut moved = Vec::new();
                for (n, idxs) in split.into_iter().enumerate() {
                    let outs: Vec<PhysicsOutput> =
                        idxs.iter().filter_map(|&i| base.outputs.get(i).cloned()).collect();
                    if outs.is_empty() {
                        continue;
                    }
                    moved.extend(idxs);
                    rig.groups.push(PhysicsGroup {
                        id: format!("{}Split{n}", base.id),
                        name: format!("{} split {}", group_label(&base), n + 1),
                        outputs: outs,
                        ..base.clone()
                    });
                }
                if let Some(g) = rig.groups.get_mut(sel) {
                    let mut i = 0;
                    g.outputs.retain(|_| {
                        let keep = !moved.contains(&i);
                        i += 1;
                        keep
                    });
                }
            }
        }

        if toggle_enable && let Some(s) = &mut self.session {
            s.puppet.set_physics_group_enabled(self.phys_sel, !group_on);
            s.dirty = true;
        }
        if reset_peaks && let Some(s) = &mut self.session {
            s.puppet.reset_physics_output_peaks();
            self.phys_peak_cache.clear();
        }
        // edits apply as they are made, like Cubism's window: once the
        // mouse is up, any difference from the model becomes one undo step
        // (no groups on either side is no change: an empty rig only differs
        // in the FPS box's default)
        let empty = rig.groups.is_empty() && saved.groups.is_empty();
        let changed = !empty && format!("{rig:?}") != format!("{saved:?}");
        let mouse_up = !ui.ctx().input(|i| i.pointer.any_down());
        if changed
            && mouse_up
            && let Some(s) = &mut self.session
        {
            let keep = if rig.groups.is_empty() { None } else { Some(rig.clone()) };
            if s.edit(|m| {
                m.physics = keep.clone();
                true
            }) {
                log::debug!("physics settings applied");
            }
        }
        self.phys_edit = Some(rig);
    }

    fn physics_inputs(&mut self, ui: &mut Ui, g: &mut PhysicsGroup, ids: &[String]) {
        // preset and normalization on the left, the input table on the right,
        // as in Cubism's Input settings tab
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                group_box(ui, "Preset", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        let names = ["Head input", "Torso input", "Chest input"];
                        egui::ComboBox::from_id_salt("phys-in-preset")
                            .selected_text(
                                names.get(self.phys_preset).copied().unwrap_or("Head input"),
                            )
                            .width(110.0)
                            .show_ui(ui, |ui| {
                                for (i, n) in names.iter().enumerate() {
                                    ui.selectable_value(&mut self.phys_preset, i, *n);
                                }
                            });
                        if ui.button("Import").clicked() {
                            g.inputs = input_preset(self.phys_preset);
                        }
                    });
                    ui.horizontal(|ui| {
                        for b in ["Add…", "Overwrite Save", "Rename…", "Delete"] {
                            ui.add_enabled(false, egui::Button::new(b))
                                .on_disabled_hover_text("Custom presets are not built yet");
                        }
                    });
                });
                ui.add_space(4.0);
                group_box(ui, "Normalization of input", |ui| {
                    egui::Grid::new("phys-norm").num_columns(4).spacing([6.0, 2.0]).show(
                        ui,
                        |ui| {
                            head(ui, "", 64.0);
                            head(ui, "Minimum", 60.0);
                            head(ui, "Center", 60.0);
                            head(ui, "Maximum", 60.0);
                            ui.end_row();
                            for (label, r) in [
                                ("Angle:", &mut g.norm_angle),
                                ("Position X:", &mut g.norm_position),
                            ] {
                                ui.label(label);
                                ui.add_sized(
                                    [60.0, 18.0],
                                    egui::DragValue::new(&mut r.min).speed(0.5),
                                );
                                ui.add_sized(
                                    [60.0, 18.0],
                                    egui::DragValue::new(&mut r.default).speed(0.5),
                                );
                                ui.add_sized(
                                    [60.0, 18.0],
                                    egui::DragValue::new(&mut r.max).speed(0.5),
                                );
                                ui.end_row();
                            }
                        },
                    );
                });
            });
            ui.add_space(6.0);
            group_box(ui, "", |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Add…").clicked() {
                        g.inputs.push(PhysicsInput {
                            param: ids.first().cloned().unwrap_or_default(),
                            weight: 100.0,
                            kind: PhysicsIo::X,
                            reflect: false,
                        });
                        self.phys_page.in_sel = Some(g.inputs.len() - 1);
                    }
                    let sel = self.phys_page.in_sel.filter(|&i| i < g.inputs.len());
                    if ui.add_enabled(sel.is_some(), egui::Button::new("Delete")).clicked()
                        && let Some(i) = sel
                    {
                        g.inputs.remove(i);
                        self.phys_page.in_sel = None;
                    }
                    if ui.add_enabled(sel.is_some(), egui::Button::new("↑")).clicked()
                        && let Some(i) = sel
                    {
                        self.phys_page.in_sel = Some(shift(&mut g.inputs, i, true));
                    }
                    if ui.add_enabled(sel.is_some(), egui::Button::new("↓")).clicked()
                        && let Some(i) = sel
                    {
                        self.phys_page.in_sel = Some(shift(&mut g.inputs, i, false));
                    }
                });
                table(ui, |ui| {
                    egui::Grid::new("phys-in")
                        .num_columns(5)
                        .spacing([4.0, 2.0])
                        .striped(true)
                        .show(ui, |ui| {
                            head(ui, "", 18.0);
                            head(ui, "Input", 126.0);
                            head(ui, "Type", 84.0);
                            head(ui, "Effectiveness", 70.0);
                            head(ui, "Reflect", 40.0);
                            ui.end_row();
                            for (i, inp) in g.inputs.iter_mut().enumerate() {
                                if row_pick(
                                    ui,
                                    self.phys_page.in_sel == Some(i),
                                    &(i + 1).to_string(),
                                    18.0,
                                ) {
                                    self.phys_page.in_sel = Some(i);
                                }
                                param_combo(ui, format!("pi{i}"), &mut inp.param, ids, 122.0);
                                kind_combo(ui, format!("pit{i}"), &mut inp.kind);
                                ui.add_sized(
                                    [70.0, 18.0],
                                    egui::DragValue::new(&mut inp.weight).range(0.0..=100.0),
                                );
                                ui.checkbox(&mut inp.reflect, "");
                                ui.end_row();
                            }
                        });
                });
            });
        });
    }

    /// Returns true when the output maxima should be reset.
    fn physics_outputs(
        &mut self,
        ui: &mut Ui,
        g: &mut PhysicsGroup,
        ids: &[String],
        peaks: &[(usize, f32)],
    ) -> bool {
        let mut reset = false;
        let n_nodes = g.nodes.len().saturating_sub(1).max(1) as u32;
        ui.horizontal(|ui| {
            if ui.button("Add…").clicked() {
                g.outputs.push(PhysicsOutput {
                    param: ids.first().cloned().unwrap_or_default(),
                    vertex: n_nodes,
                    scale: 1.0,
                    weight: 100.0,
                    kind: PhysicsIo::Angle,
                    reflect: false,
                });
                self.phys_page.out_sel = Some(g.outputs.len() - 1);
            }
            let sel = self.phys_page.out_sel.filter(|&i| i < g.outputs.len());
            if ui.add_enabled(sel.is_some(), egui::Button::new("Delete")).clicked()
                && let Some(i) = sel
            {
                g.outputs.remove(i);
                self.phys_page.out_sel = None;
            }
            if ui.add_enabled(sel.is_some(), egui::Button::new("↑")).clicked()
                && let Some(i) = sel
            {
                self.phys_page.out_sel = Some(shift(&mut g.outputs, i, true));
            }
            if ui.add_enabled(sel.is_some(), egui::Button::new("↓")).clicked()
                && let Some(i) = sel
            {
                self.phys_page.out_sel = Some(shift(&mut g.outputs, i, false));
            }
            ui.checkbox(&mut self.phys_auto_max, "Auto Update the Output Maximum");
        });
        let mut menu_cmd: Option<u8> = None;
        table(ui, |ui| {
            egui::Grid::new("phys-out").num_columns(6).spacing([4.0, 2.0]).striped(true).show(
                ui,
                |ui| {
                    head(ui, "Pendulum No.", 72.0);
                    head(ui, "Output", 126.0);
                    head(ui, "Effectiveness", 66.0);
                    head(ui, "Reflect", 40.0);
                    head(ui, "Scale", 56.0);
                    head(ui, "Output Max (%)", 76.0);
                    ui.end_row();
                    for (i, out) in g.outputs.iter_mut().enumerate() {
                        let r = ui.add_sized(
                            [72.0, 18.0],
                            egui::DragValue::new(&mut out.vertex).range(1..=n_nodes.max(1)),
                        );
                        if r.clicked() {
                            self.phys_page.out_sel = Some(i);
                        }
                        param_combo(ui, format!("po{i}"), &mut out.param, ids, 122.0);
                        ui.add_sized(
                            [66.0, 18.0],
                            egui::DragValue::new(&mut out.weight).range(0.0..=100.0),
                        );
                        ui.checkbox(&mut out.reflect, "");
                        ui.add_sized(
                            [56.0, 18.0],
                            egui::DragValue::new(&mut out.scale).speed(0.05),
                        );
                        let peak =
                            peaks.iter().find(|(oi, _)| *oi == i).map(|(_, p)| *p).unwrap_or(0.0);
                        let col = if peak > 100.5 {
                            theme::pal().rotation_red
                        } else {
                            theme::pal().text
                        };
                        let cell = ui.add_sized(
                            [76.0, 18.0],
                            egui::Button::selectable(
                                self.phys_page.out_sel == Some(i),
                                egui::RichText::new(format!("{peak:.0}")).color(col),
                            ),
                        );
                        if cell.clicked() {
                            self.phys_page.out_sel = Some(i);
                        }
                        cell.context_menu(|ui| {
                            self.phys_page.out_sel = Some(i);
                            for (n, label) in [
                                "Add",
                                "Separate as a group in a batch",
                                "Separate as individual groups",
                                "Delete",
                                "Copy",
                                "Paste",
                            ]
                            .iter()
                            .enumerate()
                            {
                                if ui.button(*label).clicked() {
                                    menu_cmd = Some(n as u8);
                                    ui.close();
                                }
                            }
                        });
                        ui.end_row();
                    }
                },
            )
        });
        let sel = self.phys_page.out_sel.filter(|&i| i < g.outputs.len());
        match (menu_cmd, sel) {
            (Some(0), _) => g.outputs.push(PhysicsOutput {
                param: ids.first().cloned().unwrap_or_default(),
                vertex: n_nodes,
                scale: 1.0,
                weight: 100.0,
                kind: PhysicsIo::Angle,
                reflect: false,
            }),
            (Some(1), Some(i)) => self.phys_page.split = Some(vec![vec![i]]),
            (Some(2), Some(i)) => self.phys_page.split = Some(vec![vec![i]]),
            (Some(3), Some(i)) => {
                g.outputs.remove(i);
                self.phys_page.out_sel = None;
            }
            (Some(4), Some(i)) => {
                self.phys_page.out_clip = g.outputs.get(i).cloned().into_iter().collect()
            }
            (Some(5), _) => g.outputs.extend(self.phys_page.out_clip.iter().cloned()),
            _ => {}
        }
        ui.horizontal(|ui| {
            let adjust = |outs: &mut Vec<PhysicsOutput>, up: bool| {
                for (i, o) in outs.iter_mut().enumerate() {
                    let Some(&(_, peak)) = peaks.iter().find(|(oi, _)| *oi == i) else { continue };
                    if peak > 1e-3 && ((up && peak < 100.0) || (!up && peak > 100.0)) {
                        o.scale *= 100.0 / peak;
                    }
                }
            };
            if ui
                .button("Increase Output (up)")
                .on_hover_text("Scale outputs that peak under 100% up to 100%")
                .clicked()
            {
                adjust(&mut g.outputs, true);
                reset = true;
            }
            if ui
                .button("Increase Output (down)")
                .on_hover_text("Scale outputs that overshoot 100% down to it")
                .clicked()
            {
                adjust(&mut g.outputs, false);
                reset = true;
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Reset scale").clicked() {
                for o in &mut g.outputs {
                    o.scale = 1.0;
                }
            }
            if ui.button("Reset maximum output").clicked() {
                reset = true;
            }
        });
        reset
    }

    fn physics_model(&mut self, ui: &mut Ui, g: &mut PhysicsGroup) {
        group_box(ui, "Preset", |ui| {
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.add_enabled_ui(false, |ui| {
                    egui::ComboBox::from_id_salt("phys-model-preset")
                        .selected_text("")
                        .width(110.0)
                        .show_ui(ui, |_| {});
                    let _ = ui.button("Import");
                });
            });
            ui.horizontal(|ui| {
                for b in ["Add…", "Overwrite Save", "Rename…", "Delete"] {
                    ui.add_enabled(false, egui::Button::new(b))
                        .on_disabled_hover_text("Custom presets are not built yet");
                }
            });
        });
        ui.add_space(4.0);
        group_box(ui, "Pendulum settings", |ui| {
            ui.horizontal(|ui| {
                if ui.button("Add").clicked() {
                    let last = g.nodes.last().cloned().unwrap_or(PhysicsNode {
                        x: 0.0,
                        y: 10.0,
                        mobility: 0.95,
                        delay: 0.9,
                        acceleration: 1.5,
                        radius: 10.0,
                    });
                    g.nodes.push(last);
                }
                let sel = self.phys_page.node_sel.filter(|&i| i >= 1 && i < g.nodes.len());
                if ui.add_enabled(sel.is_some(), egui::Button::new("Duplicate")).clicked()
                    && let Some(i) = sel
                    && let Some(n) = g.nodes.get(i).cloned()
                {
                    g.nodes.push(n);
                }
                if ui
                    .add_enabled(sel.is_some() && g.nodes.len() > 2, egui::Button::new("Delete"))
                    .clicked()
                    && let Some(i) = sel
                {
                    g.nodes.remove(i);
                    self.phys_page.node_sel = None;
                    let last = g.nodes.len().saturating_sub(1) as u32;
                    for o in &mut g.outputs {
                        o.vertex = o.vertex.min(last.max(1));
                    }
                }
                if ui.add_enabled(sel.is_some_and(|i| i > 1), egui::Button::new("↑")).clicked()
                    && let Some(i) = sel
                {
                    g.nodes.swap(i, i - 1);
                    self.phys_page.node_sel = Some(i - 1);
                }
                if ui
                    .add_enabled(sel.is_some_and(|i| i + 1 < g.nodes.len()), egui::Button::new("↓"))
                    .clicked()
                    && let Some(i) = sel
                {
                    g.nodes.swap(i, i + 1);
                    self.phys_page.node_sel = Some(i + 1);
                }
            });
            table(ui, |ui| {
                egui::Grid::new("phys-nodes").num_columns(5).spacing([4.0, 2.0]).striped(true).show(
                    ui,
                    |ui| {
                        head(ui, "No.", 30.0);
                        head(ui, "Duration", 64.0);
                        head(ui, "Ease of swinging", 92.0);
                        head(ui, "Reaction time", 78.0);
                        head(ui, "Speed of convergence", 112.0);
                        ui.end_row();
                        for (i, n) in g.nodes.iter_mut().enumerate().skip(1) {
                            if row_pick(
                                ui,
                                self.phys_page.node_sel == Some(i),
                                &i.to_string(),
                                30.0,
                            ) {
                                self.phys_page.node_sel = Some(i);
                            }
                            let mut len = (n.x * n.x + n.y * n.y).sqrt();
                            let before = len;
                            if ui
                                .add_sized(
                                    [64.0, 18.0],
                                    egui::DragValue::new(&mut len).range(0.1..=200.0).speed(0.1),
                                )
                                .changed()
                            {
                                if before > 1e-6 {
                                    let f = len / before;
                                    n.x *= f;
                                    n.y *= f;
                                } else {
                                    n.y = len;
                                }
                                n.radius = len;
                            }
                            ui.add_sized(
                                [92.0, 18.0],
                                egui::DragValue::new(&mut n.mobility).range(0.0..=1.0).speed(0.005),
                            );
                            ui.add_sized(
                                [78.0, 18.0],
                                egui::DragValue::new(&mut n.delay).range(0.0..=10.0).speed(0.01),
                            );
                            ui.add_sized(
                                [112.0, 18.0],
                                egui::DragValue::new(&mut n.acceleration)
                                    .range(0.0..=10.0)
                                    .speed(0.01),
                            );
                            ui.end_row();
                        }
                    },
                )
            });
        });
    }

    /// Group editing: rename the group or change its id, or delete it.
    fn physics_group_dialog(&mut self, ui: &mut Ui) {
        let Some((name, id)) = &mut self.phys_page.group_edit else { return };
        let mut open = true;
        let (mut ok, mut delete) = (false, false);
        crate::windows::Dialog::new("Group editing").open(&mut open).show(ui.ctx(), |ui| {
            crate::windows::form(ui, "pg-edit", |ui| {
                ui.label("Name");
                ui.text_edit_singleline(name);
                ui.end_row();
                ui.label("ID");
                ui.text_edit_singleline(id);
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ok = ui.button("OK").clicked();
                delete = ui.button("Delete group").clicked();
            });
        });
        if ok || delete {
            let (name, id) = (name.clone(), id.clone());
            self.phys_page.group_edit = None;
            let rig = self.phys_edit.get_or_insert_with(|| {
                self.session
                    .as_ref()
                    .and_then(|s| s.puppet.model().physics.clone())
                    .unwrap_or_default()
            });
            if delete {
                if self.phys_sel < rig.groups.len() {
                    rig.groups.remove(self.phys_sel);
                    self.phys_sel = self.phys_sel.min(rig.groups.len().saturating_sub(1));
                }
            } else if let Some(g) = rig.groups.get_mut(self.phys_sel) {
                g.name = name;
                if !id.trim().is_empty() {
                    g.id = id.trim().to_string();
                }
            }
        } else if !open {
            self.phys_page.group_edit = None;
        }
    }
}
