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
    /// What overridden parameters held before the first override, so they
    /// return when the override lapses.
    base: HashMap<usize, f32>,
    /// Parameter id -> index for the model the snapshot describes.
    index: HashMap<String, usize>,
    /// Identity of the model the snapshot metadata was built from.
    meta_key: u64,
    mode: &'static str,
    /// The API window: a frozen copy of the log while paused, whether the
    /// steady parameter stream is hidden, the message picked for detail.
    pub(crate) win_frozen: Option<Vec<crate::api::server::Traffic>>,
    pub(crate) win_show_stream: bool,
    pub(crate) win_pick: Option<usize>,
    /// Messages per second, measured once a second.
    pub(crate) win_rate: (f32, f32),
    pub(crate) win_rate_mark: Option<(Instant, u64, u64)>,
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
                    log::error!("Ayatsuri API could not listen on port {port}: {e}");
                    self.api.failed = Some(crate::i18n::trf(
                        "could not listen on port {port}: {error}",
                        &[("port", &port), ("error", &e)],
                    ));
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
        let mut model_changed = false;
        let n = s.puppet.model().parameters.len();
        let key = uid_of(&s.name, &path, n);
        {
            let mut snap = lock(&shared.snap);
            snap.edit_mode = mode;
            if key != self.api.meta_key || snap.params.len() != n {
                // the model changed: rebuild what clients list, then tell
                // them (below) so they ask for the new model's id
                self.api.meta_key = key;
                model_changed = true;
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

        // clients (VTube Studio among them) only re-read the model id when
        // a view event arrives, the way Cubism sends one on switching views
        if model_changed && let Some(srv) = &self.api.server {
            srv.notify("NotifyChangeEditMode", serde_json::json!({ "EditMode": mode }));
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

    /// The edit mode last reported to clients.
    fn api_mode(&self) -> &'static str {
        if self.api.mode.is_empty() { "Modeling" } else { self.api.mode }
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

    /// The API menu in the menu bar: switch it on or off, open its window.
    pub(crate) fn api_menu(&mut self, ui: &mut Ui) {
        let status = match (&self.api.server, &self.api.failed) {
            (Some(s), _) => {
                crate::i18n::trf("Listening on port {port}", &[("port", &s.addr.port())])
            }
            (None, Some(_)) => crate::i18n::tr("Could not start; see the API window").into(),
            (None, None) => crate::i18n::tr("Off").into(),
        };
        if ui.checkbox(&mut self.prefs.api_enabled, crate::i18n::tr("Ayatsuri API")).changed() {
            self.api.failed = None;
            self.prefs.save();
        }
        ui.label(RichText::new(status).size(10.5).color(theme::pal().text_faint));
        ui.separator();
        if ui.button(crate::i18n::tr("API Window…")).clicked() {
            self.show_api_window = true;
            ui.close();
        }
        if ui.button(crate::i18n::tr("API Settings…")).clicked() {
            self.show_settings = true;
            self.settings_tab = crate::windows::settings::TAB_API;
            ui.close();
        }
    }

    /// Port and network switches, shared by the window and Settings.
    pub(crate) fn api_settings_rows(&mut self, ui: &mut Ui) -> bool {
        use crate::windows::settings::row;
        let mut save = false;
        row(
            ui,
            crate::i18n::tr("Ayatsuri API"),
            crate::i18n::tr("Cubism Editor's integration API, for VTube Studio and others"),
            |ui| {
                if ui.checkbox(&mut self.prefs.api_enabled, "").changed() {
                    self.api.failed = None;
                    save = true;
                }
            },
        );
        let mut port = wanted_port(self.prefs.api_port);
        row(
            ui,
            crate::i18n::tr("Port"),
            crate::i18n::tr("22033 is Cubism's; change it if Cubism runs too"),
            |ui| {
                if ui.small_button(crate::i18n::tr("Default")).clicked() {
                    self.prefs.api_port = 0;
                    self.api.failed = None;
                    save = true;
                }
                if ui.add(egui::DragValue::new(&mut port).range(1024..=65535)).changed() {
                    self.prefs.api_port = port;
                    self.api.failed = None;
                    save = true;
                }
            },
        );
        row(
            ui,
            crate::i18n::tr("Allow remote connections"),
            crate::i18n::tr("Off: only apps on this computer can connect"),
            |ui| {
                if ui.checkbox(&mut self.prefs.api_remote, "").changed() {
                    // takes effect on the next start
                    self.api.server = None;
                    self.api.failed = None;
                    save = true;
                }
            },
        );
        save
    }

    /// Registered apps with their Allow switch, Disconnect and Forget.
    pub(crate) fn api_clients_ui(&mut self, ui: &mut Ui) {
        let Some(server) = &self.api.server else {
            ui.weak(crate::i18n::tr("Turn the API on to see apps."));
            return;
        };
        let shared = server.shared.clone();
        let live = lock(&shared.live).clone();
        let mut clients = lock(&shared.clients).clone();
        if clients.is_empty() {
            ui.weak(crate::i18n::tr("No apps yet. They appear here when they connect."));
            return;
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
                ui.label(RichText::new("●").color(dot)).on_hover_text(if online.is_empty() {
                    crate::i18n::tr("not connected").to_string()
                } else {
                    crate::i18n::trf("connected from {peers}", &[("peers", &peers)])
                });
                changed |= ui.checkbox(&mut c.allowed, &c.name).on_hover_text(&c.path).changed();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(crate::i18n::tr("Forget"))
                        .on_hover_text(crate::i18n::tr("Remove; it must ask again"))
                        .clicked()
                    {
                        lock(&shared.kick).extend(online.iter().copied());
                        forget = Some(i);
                    }
                    if !online.is_empty()
                        && ui.small_button(crate::i18n::tr("Disconnect")).clicked()
                    {
                        lock(&shared.kick).extend(online.iter().copied());
                    }
                });
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
        crate::windows::Dialog::new(crate::i18n::tr("Allow connection")).width(360.0).show(ctx, |ui| {
            ui.label(RichText::new(&client.name).strong().size(14.0));
            ui.label(crate::i18n::tr("wants to connect to the Ayatsuri API. It will be able to read and drive this model's parameters."));
            if !client.path.is_empty() {
                ui.label(RichText::new(&client.path).size(10.5).color(theme::pal().text_faint));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new(crate::i18n::tr("Allow")).strong()).clicked() {
                    answer = Some(true);
                }
                if ui.button(crate::i18n::tr("Deny")).clicked() {
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
                // an app that asked for the model before it was allowed got
                // an error and waits for a view event to try again; send one
                if let Some(srv) = &self.api.server {
                    srv.notify(
                        "NotifyChangeEditMode",
                        serde_json::json!({ "EditMode": self.api_mode() }),
                    );
                }
            } else {
                clients.retain(|c| c.token != token);
                log::info!("api: denied {}", client.name);
            }
            self.prefs.api_clients = clients.clone();
            drop(clients);
            self.prefs.save();
        }
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
