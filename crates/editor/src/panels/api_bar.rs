// The editor side of the Ayatsuri API: keeps the server running to match
// the preferences, refreshes the snapshot clients read, applies the
// parameter overrides they send, asks the user about new clients, and the
// API control in the top bar.

use std::collections::HashMap;
use std::time::Instant;

use egui::{RichText, Ui};

use super::{EditorUi, Workspace};
use crate::api::DEFAULT_PORT;
use crate::api::protocol::{OVERRIDE_LIFE_SECS, ParamInfo};
use crate::api::server::Server;
use crate::theme;

/// Editor-side API state.
#[derive(Default)]
pub(crate) struct ApiState {
    pub(crate) server: Option<Server>,
    /// Why the server is not running although it should be.
    pub(crate) failed: Option<String>,
    /// Port field in the menu, applied on Enter or Apply.
    pub(crate) port_text: String,
    /// What overridden parameters held before the first override, so they
    /// return when the override lapses.
    base: HashMap<usize, f32>,
    /// Parameter id -> index for the model the snapshot describes.
    index: HashMap<String, usize>,
    /// Identity of the model the snapshot metadata was built from.
    meta_key: u64,
    mode: &'static str,
}

/// The port the preferences ask for.
fn wanted_port(p: u16) -> u16 {
    if p == 0 { DEFAULT_PORT } else { p }
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A stable id for the open model, as the decimal string the API uses.
fn uid_of(name: &str, path: &str, params: usize) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes().chain(path.bytes()).chain(params.to_le_bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    // keep it well inside what a JSON number would hold, should a client
    // ever parse it as one
    h >> 12
}

impl EditorUi {
    /// Once per frame, before physics: server lifecycle, snapshot,
    /// overrides, events, client bookkeeping.
    pub(crate) fn api_frame(&mut self) {
        let want = self.prefs.api_enabled;
        let port = wanted_port(self.prefs.api_port);
        let running_ok = self
            .api
            .server
            .as_ref()
            .is_some_and(|s| s.addr.port() == port || self.prefs.api_port == 0);
        if want && self.api.server.is_none() && self.api.failed.is_none() {
            match Server::start(port, self.prefs.api_remote, self.prefs.api_clients.clone()) {
                Ok(s) => self.api.server = Some(s),
                Err(e) => {
                    let msg = format!("could not listen on port {port}: {e}");
                    log::error!("Ayatsuri API {msg}");
                    self.api.failed = Some(msg);
                }
            }
        } else if (!want || !running_ok) && self.api.server.is_some() {
            self.api.server = None;
            self.api_restore_all();
        }
        if !want {
            self.api.failed = None;
        }
        let Some(server) = &self.api.server else { return };
        let shared = server.shared.clone();

        // save clients a connection registered
        if shared.clients_changed.swap(false, std::sync::atomic::Ordering::Relaxed) {
            self.prefs.api_clients = lock(&shared.clients).clone();
            self.prefs.save();
        }

        let mode = if self.show_physics {
            "Physics"
        } else if self.mesh_edit.is_some() {
            "ModelingMeshEdit"
        } else if self.workspace == Workspace::Animation {
            "Animation"
        } else {
            "Modeling"
        };
        if mode != self.api.mode {
            self.api.mode = mode;
            if let Some(s) = &self.api.server {
                s.notify("NotifyChangeEditMode", serde_json::json!({ "EditMode": mode }));
            }
        }

        let Some(s) = &mut self.session else {
            let mut snap = lock(&shared.snap);
            snap.model_uid = None;
            snap.edit_mode = mode;
            return;
        };
        let path = s.aom_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        let n = s.puppet.model().parameters.len();
        let key = uid_of(&s.name, &path, n);
        {
            let mut snap = lock(&shared.snap);
            snap.edit_mode = mode;
            if key != self.api.meta_key || snap.params.len() != n {
                // the model changed: rebuild what clients list
                self.api.meta_key = key;
                self.api.base.clear();
                self.api.index.clear();
                let group_of: HashMap<usize, usize> = s
                    .groups
                    .iter()
                    .enumerate()
                    .flat_map(|(gi, g)| g.params.iter().map(move |&p| (p, gi)))
                    .collect();
                snap.params = s
                    .puppet
                    .model()
                    .parameters
                    .iter()
                    .enumerate()
                    .map(|(i, p)| ParamInfo {
                        id: p.id.clone(),
                        name: s.param_names.get(i).cloned().unwrap_or_else(|| p.id.clone()),
                        group: group_of
                            .get(&i)
                            .map(|gi| format!("ParamGroup{gi}"))
                            .unwrap_or_default(),
                        min: p.min,
                        max: p.max,
                        default: p.default,
                        repeat: p.repeat,
                    })
                    .collect();
                snap.groups = s
                    .groups
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| !g.name.is_empty())
                    .map(|(gi, g)| (format!("ParamGroup{gi}"), g.name.clone()))
                    .collect();
                for (i, p) in s.puppet.model().parameters.iter().enumerate() {
                    self.api.index.insert(p.id.clone(), i);
                }
                snap.model_uid = Some(key.to_string());
                snap.document_uid = (key ^ 0x5a5a).to_string();
                snap.file_path = path;
            }
            snap.values.clear();
            snap.values.extend((0..n).map(|i| s.puppet.value(i)));
        }

        // apply live overrides; restore what lapsed
        let now = Instant::now();
        let mut ov = lock(&shared.overrides);
        let mut changed = false;
        ov.retain(|id, (v, at)| {
            let Some(&pi) = self.api.index.get(id) else { return false };
            if now.duration_since(*at).as_secs_f32() > OVERRIDE_LIFE_SECS {
                if let Some(base) = self.api.base.remove(&pi) {
                    s.puppet.set_value(pi, base);
                    changed = true;
                }
                return false;
            }
            self.api.base.entry(pi).or_insert_with(|| s.puppet.value(pi));
            if s.puppet.value(pi) != *v {
                s.puppet.set_value(pi, *v);
                changed = true;
            }
            // an app driving a physics output wins over the rig
            s.params_held.push(pi);
            true
        });
        // a cleared buffer (ClearParameterValues) restores everything
        if ov.is_empty() && !self.api.base.is_empty() {
            for (pi, v) in self.api.base.drain() {
                s.puppet.set_value(pi, v);
            }
            changed = true;
        }
        if changed {
            s.dirty = true;
        }
    }

    /// Put back every value an override replaced (the server stopped).
    fn api_restore_all(&mut self) {
        if let Some(s) = &mut self.session {
            for (pi, v) in self.api.base.drain() {
                s.puppet.set_value(pi, v);
            }
            s.dirty = true;
        }
        self.api.base.clear();
    }

    /// Tell subscribed clients about an export, as Cubism does.
    pub(crate) fn api_notify(&self, method: &str, data: serde_json::Value) {
        if let Some(s) = &self.api.server {
            s.notify(method, data);
        }
    }

    /// The top-bar control: state at a glance, settings and clients in
    /// its menu.
    pub(crate) fn api_bar(&mut self, ui: &mut Ui) {
        let (icon_col, label) = match (&self.api.server, &self.api.failed) {
            (Some(s), _) => (theme::pal().warp_green, format!("API {}", s.addr.port())),
            (None, Some(_)) => (theme::pal().rotation_red, "API".to_string()),
            (None, None) => (theme::pal().text_faint, "API off".to_string()),
        };
        let text =
            RichText::new(format!("{} {label}", egui_phosphor::regular::BROADCAST)).color(icon_col);
        ui.menu_button(text, |ui| self.api_menu(ui)).response.on_hover_text(
            "Ayatsuri API: Cubism Editor's integration API, for VTube Studio and others",
        );
    }

    fn api_menu(&mut self, ui: &mut Ui) {
        ui.set_min_width(300.0);
        let mut save = false;
        if ui.checkbox(&mut self.prefs.api_enabled, "Ayatsuri API").changed() {
            save = true;
        }
        ui.label(
            RichText::new("Speaks Cubism Editor's External Application Integration API")
                .size(10.5)
                .color(theme::pal().text_faint),
        );
        ui.add_space(6.0);
        if self.api.port_text.is_empty() {
            self.api.port_text = wanted_port(self.prefs.api_port).to_string();
        }
        ui.horizontal(|ui| {
            ui.label("Port");
            let edit =
                ui.add(egui::TextEdit::singleline(&mut self.api.port_text).desired_width(70.0));
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Apply").clicked() || enter {
                match self.api.port_text.trim().parse::<u16>() {
                    Ok(p) if p >= 1024 => {
                        self.prefs.api_port = p;
                        self.api.failed = None;
                        save = true;
                    }
                    _ => self.api.port_text = wanted_port(self.prefs.api_port).to_string(),
                }
            }
            if ui.small_button("Default").on_hover_text("Cubism's 22033").clicked() {
                self.prefs.api_port = 0;
                self.api.port_text = DEFAULT_PORT.to_string();
                self.api.failed = None;
                save = true;
            }
        });
        if ui
            .checkbox(&mut self.prefs.api_remote, "Allow remote connections")
            .on_hover_text("Off: only apps on this computer can connect")
            .changed()
        {
            // takes effect on the next start
            self.api.server = None;
            self.api.failed = None;
            save = true;
        }
        ui.add_space(4.0);
        let status = match (&self.api.server, &self.api.failed) {
            (Some(s), _) => format!("Listening on ws://{}", s.addr),
            (None, Some(e)) => e.clone(),
            (None, None) => "Not running".into(),
        };
        ui.label(RichText::new(status).size(10.5).color(theme::pal().text_faint));

        ui.separator();
        ui.label(RichText::new("Clients").strong());
        let Some(server) = &self.api.server else {
            ui.weak("Start the API to see clients.");
            if save {
                self.prefs.save();
            }
            return;
        };
        let shared = server.shared.clone();
        let live = lock(&shared.live).clone();
        let mut clients = lock(&shared.clients).clone();
        if clients.is_empty() {
            ui.weak("None yet. Apps appear here when they connect.");
        }
        let mut forget: Option<usize> = None;
        let mut changed = false;
        for (i, c) in clients.iter_mut().enumerate() {
            let here: Vec<&crate::api::server::LiveConn> =
                live.iter().filter(|l| l.token.as_ref() == Some(&c.token)).collect();
            let online: Vec<u64> = here.iter().map(|l| l.id).collect();
            let peers = here.iter().map(|l| l.peer.as_str()).collect::<Vec<_>>().join(", ");
            ui.horizontal(|ui| {
                let dot = if online.is_empty() {
                    theme::pal().text_faint
                } else {
                    theme::pal().warp_green
                };
                ui.label(RichText::new("●").color(dot));
                changed |= ui.checkbox(&mut c.allowed, &c.name).on_hover_text(&c.path).changed();
                if !online.is_empty()
                    && ui.small_button("Disconnect").on_hover_text(&peers).clicked()
                {
                    lock(&shared.kick).extend(online.iter().copied());
                }
                if ui.small_button("Forget").clicked() {
                    lock(&shared.kick).extend(online.iter().copied());
                    forget = Some(i);
                }
            });
        }
        if let Some(i) = forget
            && i < clients.len()
        {
            clients.remove(i);
            changed = true;
        }
        if changed {
            *lock(&shared.clients) = clients.clone();
            self.prefs.api_clients = clients;
            save = true;
        }
        if save {
            self.prefs.save();
        }
    }

    /// "<App> wants to connect": the user allows or denies each new client.
    pub(crate) fn api_prompt_window(&mut self, ctx: &egui::Context) {
        let Some(server) = &self.api.server else { return };
        let shared = server.shared.clone();
        let Some(token) = lock(&shared.asks).first().cloned() else { return };
        let Some(client) = lock(&shared.clients).iter().find(|c| c.token == token).cloned() else {
            lock(&shared.asks).retain(|t| t != &token);
            return;
        };
        let mut answer: Option<bool> = None;
        crate::windows::Dialog::new("Allow connection").width(360.0).show(ctx, |ui| {
            ui.label(RichText::new(&client.name).strong().size(14.0));
            ui.label("wants to connect to the Ayatsuri API. It will be able to read and drive this model's parameters.");
            if !client.path.is_empty() {
                ui.label(RichText::new(&client.path).size(10.5).color(theme::pal().text_faint));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Allow").strong()).clicked() {
                    answer = Some(true);
                }
                if ui.button("Deny").clicked() {
                    answer = Some(false);
                }
            });
        });
        if let Some(allow) = answer {
            lock(&shared.asks).retain(|t| t != &token);
            let mut clients = lock(&shared.clients);
            if allow {
                if let Some(c) = clients.iter_mut().find(|c| c.token == token) {
                    c.allowed = true;
                }
                log::info!("api: allowed {}", client.name);
            } else {
                clients.retain(|c| c.token != token);
                log::info!("api: denied {}", client.name);
            }
            self.prefs.api_clients = clients.clone();
            drop(clients);
            self.prefs.save();
        }
    }

    /// Settings > General: the same switches as the top-bar menu.
    pub(crate) fn api_settings_rows(&mut self, ui: &mut Ui) -> bool {
        use crate::windows::settings::row;
        let mut save = false;
        row(
            ui,
            "Ayatsuri API",
            "Cubism Editor's integration API, for VTube Studio and others",
            |ui| {
                save |= ui.checkbox(&mut self.prefs.api_enabled, "").changed();
            },
        );
        let mut port = wanted_port(self.prefs.api_port);
        row(ui, "Port", "22033 is Cubism's; change it if Cubism runs too", |ui| {
            if ui.add(egui::DragValue::new(&mut port).range(1024..=65535)).changed() {
                self.prefs.api_port = port;
                self.api.port_text = port.to_string();
                self.api.failed = None;
                save = true;
            }
        });
        row(ui, "Allow remote connections", "Off: only apps on this computer", |ui| {
            if ui.checkbox(&mut self.prefs.api_remote, "").changed() {
                self.api.server = None;
                save = true;
            }
        });
        save
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_uid_is_stable_and_distinct() {
        assert_eq!(uid_of("ariu", "/a.aom", 113), uid_of("ariu", "/a.aom", 113));
        assert_ne!(uid_of("ariu", "/a.aom", 113), uid_of("ariu", "/b.aom", 113));
        // fits a JSON number exactly, should a client parse it as one
        assert!(uid_of("x", "y", 1) < (1u64 << 53));
    }

    #[test]
    fn port_zero_means_cubism_default() {
        assert_eq!(wanted_port(0), 22033);
        assert_eq!(wanted_port(25000), 25000);
    }
}
