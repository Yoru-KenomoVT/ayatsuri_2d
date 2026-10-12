// Discord Rich Presence: shows "Editing a model", what part of the editor
// is in use and for how long, on the user's Discord profile. Everything
// Discord-side runs on its own thread, so a slow or absent Discord client
// never touches the frame; the editor only hands over the latest text.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{Activity, Assets, Button, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

/// Where the presence's button sends people.
pub(crate) const GITHUB: &str = "https://github.com/Yoru-KenomoVT/ayatsuri_2d";

/// Ayatsuri2D's own Discord application, so presence works with no setup.
pub const APP_ID: &str = "1559152287459446904";

enum Msg {
    Show { details: String, state: String },
    Stop,
}

/// A running presence connection. Dropping it clears the presence.
pub struct Presence {
    tx: Sender<Msg>,
    last: (String, String),
}

impl Presence {
    /// Start talking to Discord as application `app_id`. Returns None when
    /// the thread can't be spawned.
    pub fn start(app_id: String) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let spawned = std::thread::Builder::new().name("discord-rpc".into()).spawn(move || {
            // a panic inside the client crate stays on this thread
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let started = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let mut client = DiscordIpcClient::new(&app_id);
                let mut connected = false;
                let mut latest: Option<(String, String)> = None;
                let mut dirty = false;
                loop {
                    match rx.recv_timeout(Duration::from_secs(15)) {
                        Ok(Msg::Show { details, state }) => {
                            latest = Some((details, state));
                            dirty = true;
                        }
                        Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                        // the quiet tick doubles as the reconnect timer
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    if !connected {
                        connected = client.connect().is_ok();
                        if connected {
                            log::info!("discord: connected");
                            dirty = true;
                        }
                    }
                    if connected
                        && dirty
                        && let Some((details, state)) = &latest
                    {
                        let activity = Activity::new()
                            .details(details.as_str())
                            .state(state.as_str())
                            .timestamps(Timestamps::new().start(started))
                            .assets(
                                Assets::new()
                                    .large_image("logo")
                                    .large_text(concat!("Ayatsuri2D ", env!("CARGO_PKG_VERSION"))),
                            )
                            .buttons(vec![Button::new("Ayatsuri2D on GitHub", GITHUB)]);
                        if client.set_activity(activity).is_ok() {
                            dirty = false;
                        } else {
                            // Discord went away; try again on the next tick
                            log::debug!("discord: lost the connection");
                            connected = false;
                        }
                    }
                }
                if connected {
                    let _ = client.clear_activity();
                    let _ = client.close();
                }
            }));
            if run.is_err() {
                log::warn!("discord: presence thread stopped after an error");
            }
        });
        match spawned {
            Ok(_) => Some(Self { tx, last: (String::new(), String::new()) }),
            Err(e) => {
                log::warn!("discord: could not start ({e})");
                None
            }
        }
    }

    /// Show this text, if it changed since last time.
    pub fn show(&mut self, details: &str, state: &str) {
        if self.last.0 == details && self.last.1 == state {
            return;
        }
        self.last = (details.to_string(), state.to_string());
        let _ = self.tx.send(Msg::Show { details: details.to_string(), state: state.to_string() });
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}
