// The Settings window: the same frameless, rail-and-page shape as About.
// Every option is one row, label and a short hint on the left, the control
// right-aligned, so pages read as a tidy column whatever they hold.

use egui::{
    Align, Align2, Color32, FontId, Layout, Rect, RichText, ScrollArea, Sense, Ui, pos2, vec2,
};

use crate::logging;
use crate::panels::{EditorUi, KEY_ACTIONS, format_shortcut};
use crate::theme;

const W: f32 = 760.0;
const H: f32 = 520.0;
const RAIL: f32 = 170.0;
const PAGES: [&str; 5] = ["General", "Appearance", "Performance", "Keyboard", "API"];

/// Primary colour presets: the logo orange first.
const SWATCHES: [(&str, [u8; 3]); 7] = [
    ("Ember (default)", [0xF0, 0x8E, 0x1E]),
    ("Sakura", [0xE8, 0x6A, 0x9A]),
    ("Violet", [0xA0, 0x6C, 0xF0]),
    ("Magenta", [0xD8, 0x1E, 0xF0]),
    ("Sky", [0x3C, 0x9A, 0xF0]),
    ("Mint", [0x3C, 0xC8, 0x9A]),
    ("Lemon", [0xE6, 0xC8, 0x32]),
];

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).size(11.5).strong().color(theme::pal().accent));
    ui.add_space(4.0);
}

/// One setting: label (and a faint hint) left, the control right-aligned.
pub(crate) fn row(ui: &mut Ui, label: &str, hint: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(220.0);
            ui.label(label);
            if !hint.is_empty() {
                ui.label(RichText::new(hint).size(10.5).color(theme::pal().text_faint));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
    ui.add_space(6.0);
}

/// A row of mutually exclusive choices; true when the pick changed.
fn segmented<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, options: &[(&str, T)]) -> bool {
    let mut changed = false;
    // right-to-left layout, so lay them out reversed to read left to right
    for (label, v) in options.iter().rev() {
        if ui.selectable_label(*current == *v, *label).clicked() && *current != *v {
            *current = *v;
            changed = true;
        }
    }
    changed
}

impl EditorUi {
    pub(crate) fn settings_window(&mut self, ui: &mut Ui) {
        if !self.show_settings {
            return;
        }
        let ctx = ui.ctx().clone();
        let mut open = true;
        egui::Window::new(crate::i18n::tr("Settings"))
            .title_bar(false)
            // only the header strip moves it; egui's own drag-anywhere
            // fought the pinned position and snapped back
            .movable(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size(vec2(W, H))
            .frame(egui::Frame::window(ui.style()).inner_margin(0.0).stroke(egui::Stroke::NONE))
            .pivot(Align2::CENTER_CENTER)
            .current_pos(self.settings_pos.unwrap_or(ctx.content_rect().center()))
            .show(&ctx, |ui| {
                ui.set_min_size(vec2(W, H));
                let full = ui.max_rect();
                // the whole top strip moves the window; tabs and the close
                // button sit on top of it and still take their clicks
                let handle = Rect::from_min_size(full.min, vec2(W, 44.0));
                let drag = ui.interact(handle, ui.id().with("settings-drag"), Sense::drag());
                if drag.dragged() {
                    let at = self.settings_pos.unwrap_or(ctx.content_rect().center());
                    self.settings_pos = Some(at + drag.drag_delta());
                }
                let rail = Rect::from_min_size(full.min, vec2(RAIL, H));
                ui.painter().rect_filled(rail, 0.0, theme::pal().panel_bg);
                ui.painter().text(
                    rail.min + vec2(20.0, 24.0),
                    Align2::LEFT_CENTER,
                    crate::i18n::tr("Settings"),
                    FontId::proportional(16.0),
                    theme::pal().text,
                );
                for (i, name) in PAGES.iter().enumerate() {
                    let r = Rect::from_min_size(
                        rail.min + vec2(10.0, 52.0 + i as f32 * 30.0),
                        vec2(RAIL - 20.0, 28.0),
                    );
                    let resp = ui.interact(r, ui.id().with(("set-tab", i)), Sense::click());
                    let on = self.settings_tab == i;
                    if on || resp.hovered() {
                        ui.painter().rect_filled(
                            r,
                            4.0,
                            if on { theme::pal().select } else { theme::pal().header },
                        );
                    }
                    if on {
                        ui.painter().rect_filled(
                            Rect::from_min_size(r.min + vec2(0.0, 6.0), vec2(2.0, 16.0)),
                            1.0,
                            theme::pal().accent,
                        );
                    }
                    ui.painter().text(
                        r.left_center() + vec2(12.0, 0.0),
                        Align2::LEFT_CENTER,
                        crate::i18n::tr(name),
                        FontId::proportional(13.0),
                        if on { theme::pal().text } else { theme::pal().text_faint },
                    );
                    if resp.clicked() {
                        self.settings_tab = i;
                    }
                }
                // close, top right
                let close =
                    Rect::from_center_size(full.right_top() + vec2(-20.0, 20.0), vec2(24.0, 24.0));
                let resp = ui.interact(close, ui.id().with("settings-close"), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(close, 4.0, theme::pal().select);
                }
                ui.painter().text(
                    close.center(),
                    Align2::CENTER_CENTER,
                    egui_phosphor::regular::X,
                    FontId::proportional(14.0),
                    theme::pal().text,
                );
                if resp.clicked() {
                    open = false;
                }

                let page = Rect::from_min_max(
                    pos2(rail.max.x + 24.0, full.min.y + 14.0),
                    full.max - vec2(16.0, 14.0),
                );
                let mut page_ui = ui.new_child(
                    egui::UiBuilder::new().max_rect(page).layout(Layout::top_down(Align::Min)),
                );
                let title = PAGES.get(self.settings_tab).copied().unwrap_or("Keyboard");
                page_ui.label(
                    RichText::new(crate::i18n::tr(title)).size(18.0).color(theme::pal().text),
                );
                page_ui.add_space(2.0);
                ScrollArea::vertical()
                    .id_salt(("settings-page", self.settings_tab))
                    .auto_shrink([false, false])
                    .show(&mut page_ui, |ui| {
                        ui.set_width(ui.available_width() - 12.0);
                        match self.settings_tab {
                            0 => self.settings_general(ui),
                            1 => self.settings_appearance(ui),
                            2 => self.settings_performance(ui),
                            3 => self.settings_keyboard(ui),
                            _ => self.settings_api(ui),
                        }
                    });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.rebind.is_none() {
            open = false;
        }
        self.show_settings = open;
    }

    fn settings_general(&mut self, ui: &mut Ui) {
        let mut save = false;
        section(ui, crate::i18n::tr("LANGUAGE"));
        let langs = crate::i18n::languages();
        let system = crate::i18n::system_language();
        let system_name = langs
            .iter()
            .find(|l| l.code == system)
            .map_or("English".to_string(), |l| l.name.clone());
        let chosen = self.prefs.language.clone();
        let label = match &chosen {
            None => crate::i18n::trf("System ({language})", &[("language", &system_name)]),
            Some(code) => {
                langs.iter().find(|l| &l.code == code).map_or(code.clone(), |l| l.name.clone())
            }
        };
        let mut pick: Option<Option<String>> = None;
        row(
            ui,
            crate::i18n::tr("Language"),
            crate::i18n::tr("System follows your computer's language"),
            |ui| {
                egui::ComboBox::from_id_salt("language").selected_text(label).show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            chosen.is_none(),
                            crate::i18n::trf("System ({language})", &[("language", &system_name)]),
                        )
                        .clicked()
                    {
                        pick = Some(None);
                    }
                    for l in &langs {
                        if ui
                            .selectable_label(chosen.as_deref() == Some(l.code.as_str()), &l.name)
                            .clicked()
                        {
                            pick = Some(Some(l.code.clone()));
                        }
                    }
                });
            },
        );
        ui.label(
            egui::RichText::new(crate::i18n::tr(
                "Languages other than English were translated with tools (AI and machine \
                 translation such as Google Translate) and may have mistakes; corrections are \
                 welcome on GitHub.",
            ))
            .size(11.0)
            .color(crate::theme::pal().text_faint),
        );
        if let Some(choice) = pick {
            self.prefs.language = choice;
            crate::i18n::set_language(self.prefs.language.as_deref().unwrap_or("system"));
            // fonts follow the language: its script's face goes first
            crate::theme::set_look(ui.ctx(), self.prefs.look());
            save = true;
        }

        section(ui, crate::i18n::tr("STARTUP"));
        row(
            ui,
            crate::i18n::tr("Reopen last model"),
            crate::i18n::tr("Open whatever you had open when you quit"),
            |ui| {
                save |= ui.checkbox(&mut self.prefs.reopen_last, "").changed();
            },
        );
        let mut full = self.prefs.start_fullscreen;
        row(ui, crate::i18n::tr("Start in fullscreen"), "", |ui| {
            if ui.checkbox(&mut full, "").changed() {
                self.prefs.start_fullscreen = full;
                save = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(full));
            }
        });
        let mut splash = !self.prefs.splash_off;
        row(
            ui,
            crate::i18n::tr("Splash screen"),
            crate::i18n::tr("Shown by the launcher while the editor loads"),
            |ui| {
                if ui.checkbox(&mut splash, "").changed() {
                    self.prefs.splash_off = !splash;
                    save = true;
                }
            },
        );
        let mut particles = !self.prefs.splash_particles_off;
        row(
            ui,
            crate::i18n::tr("Splash particles"),
            crate::i18n::tr("The drifting network behind the splash"),
            |ui| {
                ui.add_enabled_ui(splash, |ui| {
                    if ui.checkbox(&mut particles, "").changed() {
                        self.prefs.splash_particles_off = !particles;
                        save = true;
                    }
                });
            },
        );

        section(ui, crate::i18n::tr("UPDATES"));
        let mut check = !self.prefs.update_check_off;
        row(
            ui,
            crate::i18n::tr("Check for updates"),
            crate::i18n::tr(
                "Asks GitHub for a newer release when Ayatsuri2D starts; never installs anything",
            ),
            |ui| {
                if ui.checkbox(&mut check, "").changed() {
                    self.prefs.update_check_off = !check;
                    save = true;
                }
            },
        );

        section(ui, crate::i18n::tr("DISCORD"));
        let mut on = !self.prefs.discord_off;
        row(
            ui,
            crate::i18n::tr("Rich Presence"),
            crate::i18n::tr("Needs the Discord app; reconnects on its own"),
            |ui| {
                if ui.checkbox(&mut on, "").changed() {
                    self.prefs.discord_off = !on;
                    save = true;
                }
            },
        );
        row(
            ui,
            crate::i18n::tr("Show the model's name"),
            crate::i18n::tr("Off shows \"Editing a model\" instead"),
            |ui| {
                save |= ui.checkbox(&mut self.prefs.discord_show_name, "").changed();
            },
        );
        row(
            ui,
            crate::i18n::tr("Application ID"),
            crate::i18n::tr("Leave empty for Ayatsuri2D's own"),
            |ui| {
                save |= ui
                    .add(
                        egui::TextEdit::singleline(&mut self.prefs.discord_app_id)
                            .hint_text(crate::i18n::tr("Ayatsuri2D"))
                            .desired_width(180.0),
                    )
                    .changed();
            },
        );

        section(ui, crate::i18n::tr("FILES"));
        let n = self.prefs.recents.len();
        row(
            ui,
            crate::i18n::tr("Recent files"),
            &crate::i18n::trf("{count} remembered", &[("count", &n)]),
            |ui| {
                if ui.add_enabled(n > 0, egui::Button::new(crate::i18n::tr("Clear"))).clicked() {
                    self.prefs.recents.clear();
                    save = true;
                }
            },
        );

        section(ui, crate::i18n::tr("LOGGING"));
        let mut level = self.prefs.log_level.clone().unwrap_or_else(|| "info".into());
        row(
            ui,
            crate::i18n::tr("Log level"),
            crate::i18n::tr("More detail helps bug reports"),
            |ui| {
                let opts: Vec<(&str, &str)> =
                    ["error", "warn", "info", "debug", "trace"].iter().map(|n| (*n, *n)).collect();
                let mut cur = level.as_str();
                if segmented(ui, &mut cur, &opts) {
                    level = cur.to_string();
                    logging::set_level(logging::level_from_str(&level));
                    self.prefs.log_level = Some(level.clone());
                    save = true;
                }
            },
        );
        if let Some(p) = logging::log_path() {
            row(ui, crate::i18n::tr("Log file"), &p.display().to_string(), |ui| {
                if ui.button(crate::i18n::tr("Copy path")).clicked() {
                    ui.ctx().copy_text(p.display().to_string());
                }
            });
        }
        if save {
            self.prefs.save();
        }
    }

    fn settings_appearance(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut save = false;
        section(ui, crate::i18n::tr("THEME"));
        #[derive(Clone, Copy, PartialEq)]
        enum Mode {
            Dark,
            Light,
            System,
        }
        let mut mode = if self.prefs.follow_system {
            Mode::System
        } else if theme::is_dark() {
            Mode::Dark
        } else {
            Mode::Light
        };
        row(
            ui,
            crate::i18n::tr("Theme"),
            crate::i18n::tr("System follows your desktop's light or dark setting"),
            |ui| {
                if segmented(
                    ui,
                    &mut mode,
                    &[
                        (crate::i18n::tr("Dark"), Mode::Dark),
                        (crate::i18n::tr("Light"), Mode::Light),
                        (crate::i18n::tr("System"), Mode::System),
                    ],
                ) {
                    self.prefs.follow_system = mode == Mode::System;
                    let dark = match mode {
                        Mode::Dark => true,
                        Mode::Light => false,
                        Mode::System => ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark),
                    };
                    self.prefs.dark = Some(dark);
                    theme::set_dark(&ctx, dark);
                    save = true;
                }
            },
        );
        row(
            ui,
            crate::i18n::tr("Primary colour"),
            crate::i18n::tr("Accents, selection, the launcher bar"),
            |ui| {
                // the same RGBA picker as the overlay colours
                let c = theme::accent();
                let mut pick = c;
                if ui.color_edit_button_srgba(&mut pick).changed() {
                    let [r, g, b, a] = pick.to_srgba_unmultiplied();
                    theme::set_accent(&ctx, Some(pick));
                    self.prefs.accent = Some([r, g, b]);
                    self.prefs.accent_alpha = (a < 255).then_some(a);
                    save = true;
                }
                ui.add_space(6.0);
                for (name, sw) in SWATCHES.iter().rev() {
                    let col = Color32::from_rgb(sw[0], sw[1], sw[2]);
                    let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    let picked = c == col;
                    ui.painter().circle_filled(r.center(), 7.0, col);
                    if picked || resp.hovered() {
                        ui.painter().circle_stroke(
                            r.center(),
                            8.5,
                            egui::Stroke::new(1.5, theme::pal().text),
                        );
                    }
                    if resp.on_hover_text(crate::i18n::tr(name)).clicked() {
                        let default = sw == &[0xF0, 0x8E, 0x1E];
                        theme::set_accent(&ctx, if default { None } else { Some(col) });
                        self.prefs.accent = if default { None } else { Some(*sw) };
                        self.prefs.accent_alpha = None;
                        save = true;
                    }
                }
            },
        );

        section(ui, crate::i18n::tr("LAYOUT"));
        let mut scale = self.prefs.ui_scale.unwrap_or(1.0);
        row(
            ui,
            crate::i18n::tr("Interface scale"),
            crate::i18n::tr("Zooms everything; good for high-DPI screens"),
            |ui| {
                let opts =
                    [("90%", 0.9), ("100%", 1.0), ("110%", 1.1), ("125%", 1.25), ("150%", 1.5)];
                if segmented(ui, &mut scale, &opts) {
                    self.prefs.ui_scale = (scale != 1.0).then_some(scale);
                    ctx.set_zoom_factor(scale);
                    save = true;
                }
            },
        );
        let mut look = self.prefs.look();
        let before = look;
        row(ui, crate::i18n::tr("Text size"), "", |ui| {
            segmented(
                ui,
                &mut look.text,
                &[
                    (crate::i18n::tr("Small"), 11.0),
                    (crate::i18n::tr("Normal"), 12.0),
                    (crate::i18n::tr("Large"), 13.5),
                    (crate::i18n::tr("Larger"), 15.0),
                ],
            );
        });
        row(
            ui,
            crate::i18n::tr("Density"),
            crate::i18n::tr("Space between rows and controls"),
            |ui| {
                segmented(
                    ui,
                    &mut look.density,
                    &[
                        (crate::i18n::tr("Compact"), 1.0),
                        (crate::i18n::tr("Comfortable"), 1.3),
                        (crate::i18n::tr("Roomy"), 1.6),
                    ],
                );
            },
        );
        row(
            ui,
            crate::i18n::tr("Corners"),
            crate::i18n::tr("Rounding on buttons, menus and windows"),
            |ui| {
                segmented(
                    ui,
                    &mut look.corners,
                    &[
                        (crate::i18n::tr("Sharp"), 0.0),
                        (crate::i18n::tr("Subtle"), 3.0),
                        (crate::i18n::tr("Round"), 6.0),
                    ],
                );
            },
        );

        section(ui, crate::i18n::tr("MOTION"));
        row(
            ui,
            crate::i18n::tr("Reduce motion"),
            crate::i18n::tr("No animated menus, fades or smooth scrolling"),
            |ui| {
                ui.checkbox(&mut look.reduce_motion, "");
            },
        );
        if look != before {
            self.prefs.text_size = Some(look.text);
            self.prefs.density = Some(look.density);
            self.prefs.corners = Some(look.corners);
            self.prefs.reduce_motion = look.reduce_motion;
            theme::set_look(&ctx, look);
            save = true;
        }
        if save {
            self.prefs.save();
            save = false;
        }
        // what used to be the View page: the canvas and its overlays
        self.settings_canvas(ui);
        ui.add_space(12.0);
        if ui.button(crate::i18n::tr("Reset appearance")).clicked() {
            self.prefs.follow_system = false;
            self.prefs.dark = Some(true);
            self.prefs.accent = None;
            self.prefs.accent_alpha = None;
            // the canvas and overlays live on this page too
            self.prefs.view_bg = None;
            self.prefs.view_bg_alpha = None;
            self.prefs.overlay = Default::default();
            self.prefs.ui_scale = None;
            self.prefs.text_size = None;
            self.prefs.density = None;
            self.prefs.corners = None;
            self.prefs.reduce_motion = false;
            theme::set_look(&ctx, self.prefs.look());
            theme::set_accent(&ctx, None);
            theme::set_dark(&ctx, true);
            ctx.set_zoom_factor(1.0);
            save = true;
        }
        if save {
            self.prefs.save();
        }
    }

    fn settings_canvas(&mut self, ui: &mut Ui) {
        let mut save = false;
        section(ui, crate::i18n::tr("CANVAS"));
        row(
            ui,
            crate::i18n::tr("Background"),
            crate::i18n::tr("Behind the model's canvas"),
            |ui| {
                if ui.small_button(crate::i18n::tr("Reset")).clicked() {
                    self.prefs.view_bg = None;
                    self.prefs.view_bg_alpha = None;
                    save = true;
                }
                let mut pick = self.prefs.view_bg_color().unwrap_or(theme::pal().canvas_bg);
                if ui.color_edit_button_srgba(&mut pick).changed() {
                    let [r, g, b, a] = pick.to_srgba_unmultiplied();
                    self.prefs.view_bg = Some([r, g, b]);
                    self.prefs.view_bg_alpha = (a < 255).then_some(a);
                    save = true;
                }
            },
        );
        section(ui, crate::i18n::tr("OVERLAYS"));
        ui.label(
            RichText::new(crate::i18n::tr(
                "Mesh lines, deformer handles and weights on the canvas.",
            ))
            .size(10.5)
            .color(theme::pal().text_faint),
        );
        ui.add_space(4.0);
        self.overlay_settings(ui);
        if save {
            self.prefs.save();
        }
    }

    fn settings_performance(&mut self, ui: &mut Ui) {
        let mut save = false;
        section(ui, crate::i18n::tr("THIS MODEL"));
        match &self.session {
            Some(s) => {
                let verts: usize = s.puppet.drawables().map(|d| d.positions.len() / 2).sum();
                for (k, v) in [
                    (
                        "Last evaluate",
                        crate::i18n::trf(
                            "{time} ms",
                            &[("time", &format!("{:.2}", s.last_update_ms))],
                        ),
                    ),
                    ("Visible meshes", s.puppet.drawables().count().to_string()),
                    ("Vertices", verts.to_string()),
                ] {
                    row(ui, crate::i18n::tr(k), "", |ui| {
                        ui.label(RichText::new(v).color(theme::pal().text_faint));
                    });
                }
            }
            None => {
                ui.label(
                    RichText::new(crate::i18n::tr("No model loaded."))
                        .color(theme::pal().text_faint),
                );
            }
        }
        section(ui, crate::i18n::tr("LIMITS"));
        row(
            ui,
            crate::i18n::tr("Animation frame cap"),
            crate::i18n::tr("How often physics and follow redraw; 30 is easy on laptops"),
            |ui| {
                save |= segmented(
                    ui,
                    &mut self.prefs.fps_cap,
                    &[(crate::i18n::tr("Off"), 0u32), ("30", 30), ("60", 60), ("120", 120)],
                );
            },
        );
        let mut msaa = match self.prefs.msaa {
            1 | 2 | 4 | 8 => self.prefs.msaa,
            _ => 4,
        };
        row(
            ui,
            crate::i18n::tr("Model anti-aliasing"),
            crate::i18n::tr("Applies after a restart; steps down if the GPU can't do it"),
            |ui| {
                if segmented(
                    ui,
                    &mut msaa,
                    &[(crate::i18n::tr("Off"), 1u32), ("2x", 2), ("4x", 4), ("8x", 8)],
                ) {
                    self.prefs.msaa = msaa;
                    save = true;
                }
            },
        );
        row(ui, crate::i18n::tr("Renderer"), "", |ui| {
            let r = if self.gpu {
                crate::i18n::tr("wgpu (GPU), shared with the UI")
            } else {
                crate::i18n::tr("software fallback")
            };
            ui.label(RichText::new(r).color(theme::pal().text_faint));
        });
        if save {
            self.prefs.save();
        }
    }

    fn settings_api(&mut self, ui: &mut Ui) {
        section(ui, crate::i18n::tr("SERVER"));
        if self.api_settings_rows(ui) {
            self.prefs.save();
        }
        row(
            ui,
            crate::i18n::tr("API window"),
            crate::i18n::tr("Status, connected apps, the data coming in"),
            |ui| {
                if ui.button(crate::i18n::tr("Open…")).clicked() {
                    self.show_api_window = true;
                }
            },
        );
        section(ui, crate::i18n::tr("ALLOWED APPS"));
        self.api_clients_ui(ui);
    }

    fn settings_keyboard(&mut self, ui: &mut Ui) {
        // an armed row eats the next real key press
        if let Some(action) = self.rebind {
            let captured = ui.ctx().input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
                        Some((*key, *modifiers))
                    } else {
                        None
                    }
                })
            });
            if let Some((key, modifiers)) = captured {
                if key == egui::Key::Escape {
                    self.rebind = None;
                } else {
                    let sc = egui::KeyboardShortcut::new(modifiers, key);
                    let text = format_shortcut(&sc);
                    self.prefs.keybinds.retain(|(a, _)| a != action);
                    self.prefs.keybinds.push((action.to_string(), text.clone()));
                    self.prefs.save();
                    log::info!("{action} rebound to {text}");
                    self.rebind = None;
                }
            }
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.key_filter)
                    .hint_text(format!(
                        "{}  {}",
                        egui_phosphor::regular::MAGNIFYING_GLASS,
                        crate::i18n::tr("Search actions or keys")
                    ))
                    .desired_width(280.0),
            );
            if !self.key_filter.is_empty() && ui.small_button(crate::i18n::tr("Clear")).clicked() {
                self.key_filter.clear();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.prefs.keybinds.is_empty(),
                        egui::Button::new(crate::i18n::tr("Reset all")),
                    )
                    .clicked()
                {
                    self.prefs.keybinds.clear();
                    self.prefs.save();
                }
            });
        });
        ui.label(
            RichText::new(crate::i18n::tr(
                "Click a shortcut, then press the new keys. Esc cancels.",
            ))
            .size(10.5)
            .color(theme::pal().text_faint),
        );

        // a binding used twice gets flagged, so clashes are easy to spot
        let texts: Vec<String> =
            KEY_ACTIONS.iter().map(|a| format_shortcut(&self.bound(a.1))).collect();
        let needle = self.key_filter.to_lowercase();
        let matches = |cat: &str, label: &str, keys: &str| {
            needle.is_empty()
                || label.to_lowercase().contains(&needle)
                || cat.to_lowercase().contains(&needle)
                || keys.to_lowercase().contains(&needle)
        };
        let mut current = "";
        for (n, &(cat, action, label, _, _)) in KEY_ACTIONS.iter().enumerate() {
            let (cat, label) = (crate::i18n::tr(cat), crate::i18n::tr(label));
            let text = texts.get(n).cloned().unwrap_or_default();
            if !matches(cat, label, &text) {
                continue;
            }
            if cat != current {
                current = cat;
                section(ui, &cat.to_uppercase());
            }
            let armed = self.rebind == Some(action);
            let clash = texts.iter().filter(|t| **t == text).count() > 1;
            let changed = self.prefs.keybinds.iter().any(|(a, _)| a == action);
            row(ui, label, "", |ui| {
                if changed && ui.small_button(crate::i18n::tr("Reset")).clicked() {
                    self.prefs.keybinds.retain(|(a, _)| a != action);
                    self.prefs.save();
                }
                let shown = if armed { crate::i18n::tr("press keys…").to_string() } else { text };
                let rich = if armed {
                    RichText::new(shown).color(theme::pal().accent)
                } else if clash {
                    RichText::new(shown).color(theme::pal().rotation_red)
                } else {
                    RichText::new(shown)
                };
                let btn = ui.add_sized(vec2(150.0, 20.0), egui::Button::new(rich));
                let btn = if clash {
                    btn.on_hover_text(crate::i18n::tr("Also used by another action"))
                } else {
                    btn
                };
                if btn.clicked() {
                    self.rebind = Some(action);
                }
            });
        }

        // fixed gestures, listed for reference but not rebindable
        let mut header = false;
        for (label, keys) in BUILT_IN {
            let label = crate::i18n::tr(label);
            if !matches(crate::i18n::tr("Built in"), label, keys) {
                continue;
            }
            if !header {
                header = true;
                section(ui, crate::i18n::tr("BUILT IN"));
            }
            row(ui, label, "", |ui| {
                ui.add_enabled_ui(false, |ui| {
                    ui.add_sized(vec2(150.0, 20.0), egui::Button::new(*keys));
                })
                .response
                .on_disabled_hover_text(crate::i18n::tr("Built in, can't be changed"));
            });
        }
    }
}

/// Gestures handled directly by the canvas and palettes.
const BUILT_IN: &[(&str, &str)] = &[
    ("Pan the canvas", "Space+Drag"),
    ("Zoom the canvas", "Scroll"),
    ("Walk up the deformer chain", "Alt+Click"),
    ("Cancel or clear selection", "Esc"),
    ("Delete the selection", "Delete"),
    ("Smaller / larger brush", "[  /  ]"),
    ("Size the selection brush", "B+Drag"),
    ("Key the selected parameter", "K"),
];
