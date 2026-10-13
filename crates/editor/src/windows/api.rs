// The API window: whether the Ayatsuri API is up and where, which apps are
// connected, the parameter values they are driving right now, and a live
// log of the messages both ways.

use std::time::Instant;

use egui::{RichText, ScrollArea, Ui};

use crate::panels::EditorUi;
use crate::theme;

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The steady parameter stream (an app sending every frame) and its replies.
fn is_stream(method: &str) -> bool {
    matches!(method, "SetParameterValues" | "GetParameterValues")
}

fn uptime(secs: u64) -> String {
    match secs {
        0..=59 => crate::i18n::trf("{secs} s", &[("secs", &secs)]),
        60..=3599 => crate::i18n::trf(
            "{mins} min {secs} s",
            &[("mins", &(secs / 60)), ("secs", &(secs % 60))],
        ),
        _ => crate::i18n::trf(
            "{hours} h {mins} min",
            &[("hours", &(secs / 3600)), ("mins", &((secs % 3600) / 60))],
        ),
    }
}

impl EditorUi {
    pub(crate) fn api_window(&mut self, ui: &mut Ui) {
        if !self.show_api_window {
            return;
        }
        let ctx = ui.ctx().clone();
        let mut open = true;
        crate::windows::Dialog::new(crate::i18n::tr("Ayatsuri API"))
            .open(&mut open)
            .width(680.0)
            .show(&ctx, |ui| {
                self.api_window_body(ui);
            });
        self.show_api_window = open;
        // the log and the values move while it is open
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }

    fn api_window_body(&mut self, ui: &mut Ui) {
        use crate::windows::settings::row;
        // ---- status ----
        let (state, col) = match (&self.api.server, &self.api.failed) {
            (Some(s), _) => (
                crate::i18n::trf("Listening on ws://{address}", &[("address", &s.addr)]),
                theme::pal().warp_green,
            ),
            (None, Some(e)) => (e.clone(), theme::pal().rotation_red),
            (None, None) => (crate::i18n::tr("Off").into(), theme::pal().text_faint),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").color(col));
            ui.label(RichText::new(&state).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.checkbox(&mut self.prefs.api_enabled, crate::i18n::tr("Enabled")).changed() {
                    self.api.failed = None;
                    self.prefs.save();
                }
            });
        });
        let Some(server) = &self.api.server else {
            ui.add_space(6.0);
            ui.weak(crate::i18n::tr("Turn the API on to accept apps such as VTube Studio."));
            return;
        };
        let shared = server.shared.clone();
        let started = server.started;
        let addr = server.addr;
        let (ins, outs) = (
            shared.count_in.load(std::sync::atomic::Ordering::Relaxed),
            shared.count_out.load(std::sync::atomic::Ordering::Relaxed),
        );
        // messages per second, re-measured each second
        let now = Instant::now();
        match self.api.win_rate_mark {
            Some((at, i0, o0)) if now.duration_since(at).as_secs_f32() >= 1.0 => {
                let dt = now.duration_since(at).as_secs_f32();
                self.api.win_rate =
                    ((ins.saturating_sub(i0)) as f32 / dt, (outs.saturating_sub(o0)) as f32 / dt);
                self.api.win_rate_mark = Some((now, ins, outs));
            }
            None => self.api.win_rate_mark = Some((now, ins, outs)),
            _ => {}
        }
        let (rin, rout) = self.api.win_rate;
        let faint = |s: String| RichText::new(s).color(theme::pal().text_faint);
        row(ui, crate::i18n::tr("Up for"), "", |ui| {
            ui.label(faint(uptime(started.elapsed().as_secs())));
        });
        row(
            ui,
            crate::i18n::tr("Messages"),
            crate::i18n::tr("In and out since it started, and per second now"),
            |ui| {
                ui.label(faint(crate::i18n::trf(
                    "in {ins} ({rate_in}/s)   out {outs} ({rate_out}/s)",
                    &[
                        ("ins", &ins),
                        ("rate_in", &format!("{rin:.0}")),
                        ("outs", &outs),
                        ("rate_out", &format!("{rout:.0}")),
                    ],
                )));
            },
        );
        row(ui, crate::i18n::tr("Reachable from"), "", |ui| {
            ui.label(faint(if addr.ip().is_loopback() {
                crate::i18n::tr("this computer only").into()
            } else {
                crate::i18n::tr("the network").into()
            }));
        });

        // ---- apps ----
        ui.add_space(6.0);
        ui.label(
            RichText::new(crate::i18n::tr("APPS")).size(11.5).strong().color(theme::pal().accent),
        );
        self.api_clients_ui(ui);

        // ---- incoming values ----
        ui.add_space(6.0);
        ui.label(
            RichText::new(crate::i18n::tr("PARAMETERS BEING DRIVEN"))
                .size(11.5)
                .strong()
                .color(theme::pal().accent),
        );
        let mut driven: Vec<(String, f32, f32)> = lock(&shared.overrides)
            .iter()
            .map(|(id, (v, at))| (id.clone(), *v, now.duration_since(*at).as_secs_f32()))
            .collect();
        driven.sort_by(|a, b| a.0.cmp(&b.0));
        if driven.is_empty() {
            ui.weak(crate::i18n::tr(
                "Nothing right now. Values an app sends show here while they are live.",
            ));
        } else {
            ScrollArea::vertical().id_salt("api-driven").max_height(150.0).show(ui, |ui| {
                egui::Grid::new("api-driven-grid")
                    .num_columns(3)
                    .spacing([24.0, 2.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for (id, v, age) in driven.iter().take(500) {
                            ui.label(RichText::new(id).monospace().size(11.0));
                            ui.label(RichText::new(format!("{v:>9.3}")).monospace().size(11.0));
                            ui.label(
                                faint(crate::i18n::trf(
                                    "{time} ms ago",
                                    &[("time", &format!("{:.0}", age * 1000.0))],
                                ))
                                .size(10.5),
                            );
                            ui.end_row();
                        }
                    });
            });
        }

        // ---- traffic ----
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(crate::i18n::tr("TRAFFIC"))
                    .size(11.5)
                    .strong()
                    .color(theme::pal().accent),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(crate::i18n::tr("Clear")).clicked() {
                    lock(&shared.traffic).clear();
                    self.api.win_frozen = None;
                    self.api.win_pick = None;
                }
                let mut paused = self.api.win_frozen.is_some();
                if ui.checkbox(&mut paused, crate::i18n::tr("Pause")).changed() {
                    self.api.win_frozen =
                        paused.then(|| lock(&shared.traffic).iter().cloned().collect());
                }
                ui.checkbox(
                    &mut self.api.win_show_stream,
                    crate::i18n::tr("Show parameter stream"),
                )
                .on_hover_text(crate::i18n::tr(
                    "SetParameterValues and friends, which apps send every frame",
                ));
            });
        });
        let log: Vec<crate::api::server::Traffic> = match &self.api.win_frozen {
            Some(f) => f.clone(),
            None => lock(&shared.traffic).iter().cloned().collect(),
        };
        let shown: Vec<(usize, &crate::api::server::Traffic)> = log
            .iter()
            .enumerate()
            .filter(|(_, t)| self.api.win_show_stream || !is_stream(&t.method))
            .collect();
        ScrollArea::vertical()
            .id_salt("api-traffic")
            .max_height(200.0)
            .stick_to_bottom(true)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if shown.is_empty() {
                    ui.weak(crate::i18n::tr("No messages yet."));
                }
                for (i, t) in &shown {
                    let at = t.at.saturating_duration_since(started).as_secs_f32();
                    let (arrow, col) = if t.incoming {
                        ("→", theme::pal().text)
                    } else if t.kind == "Error" {
                        ("←", theme::pal().rotation_red)
                    } else {
                        ("←", theme::pal().text_faint)
                    };
                    let line =
                        format!("{at:>9.3}s {arrow} {:<22} {:<8} {}", t.method, t.kind, t.peer);
                    let picked = self.api.win_pick == Some(*i);
                    if ui
                        .selectable_label(
                            picked,
                            RichText::new(line).monospace().size(10.5).color(col),
                        )
                        .clicked()
                    {
                        self.api.win_pick = if picked { None } else { Some(*i) };
                    }
                }
            });
        if let Some(t) = self.api.win_pick.and_then(|i| log.get(i)) {
            ui.add_space(4.0);
            // pretty-print what parses, show the rest as it came
            let body = serde_json::from_str::<serde_json::Value>(&t.text)
                .ok()
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                .unwrap_or_else(|| t.text.clone());
            ScrollArea::vertical().id_salt("api-detail").max_height(160.0).show(ui, |ui| {
                ui.label(RichText::new(body).monospace().size(10.5));
            });
        }
    }
}
