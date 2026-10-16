// Saved preferences: theme, recent files, and the dock layouts, written as
// json under the user's config dir. Written atomically (tmp + rename) so a
// crash never half-writes the file.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::dock::DockNode;

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub dark: Option<bool>,
    pub recents: Vec<PathBuf>,
    pub workspace_modeling: Option<DockNode>,
    pub workspace_animation: Option<DockNode>,
    pub custom_workspaces: Vec<(String, DockNode)>,
    pub accent: Option<[u8; 3]>,
    pub view_bg: Option<[u8; 3]>,
    /// Alpha for the two colours above, kept apart so older files with
    /// three-channel colours still load. None is opaque.
    pub accent_alpha: Option<u8>,
    pub view_bg_alpha: Option<u8>,
    pub reopen_last: bool,
    pub toolbar_docked: bool,
    pub toolbar_pos: Option<[f32; 2]>,
    pub log_level: Option<String>,
    /// Remapped shortcuts: action name to a "Ctrl+Shift+Z" style string.
    pub keybinds: Vec<(String, String)>,
    /// Repaint cap for animation (physics, follow); 0 leaves it uncapped.
    pub fps_cap: u32,
    /// Video memory for full-size texture pages, in MB; 0 is the default (512).
    pub texture_budget_mb: u32,
    /// Model MSAA samples (1 disables); needs a restart. 0 means default.
    pub msaa: u32,
    /// Canvas overlay colours; None follows the theme (mesh lines follow
    /// the primary colour).
    pub overlay: Overlay,
    /// Discord Rich Presence is on unless turned off here. The app id is an
    /// optional override of Ayatsuri2D's own; the model's name only shows
    /// when allowed (file names can be client work).
    pub discord_off: bool,
    pub discord_app_id: String,
    pub discord_show_name: bool,
    /// Open the editor window fullscreen.
    pub start_fullscreen: bool,
    /// Follow the desktop's light or dark setting instead of `dark`.
    pub follow_system: bool,
    /// Whole-interface zoom; None is 100%.
    pub ui_scale: Option<f32>,
    /// Body text size in points; None is 12.
    pub text_size: Option<f32>,
    /// Spacing multiplier; None is 1 (compact).
    pub density: Option<f32>,
    /// Corner radius in points; None is 3.
    pub corners: Option<f32>,
    pub reduce_motion: bool,
    /// The launcher reads these: skip the splash, or keep it but still.
    pub splash_off: bool,
    pub splash_particles_off: bool,
    /// Interface language code ("ja", "ko", "zh-CN"...); None follows the
    /// operating system.
    pub language: Option<String>,
    /// Don't ask GitHub for a newer release at start (launcher and editor).
    pub update_check_off: bool,
    /// Don't offer to download ffmpeg at start when there is none.
    pub ffmpeg_prompt_off: bool,
    /// Hide the canvas rulers (guides still show).
    pub rulers_off: bool,
    pub guide_snap_off: bool,
    /// Don't record the timelapse beside each model.
    pub timelapse_off: bool,
    /// The release the user said "Later" to; its banner stays hidden.
    pub update_dismissed: Option<String>,
    /// Physics page splitters as the user left them: settings column
    /// width, parameter rail width, tabs height, model panel width.
    pub physics_layout: Option<[f32; 4]>,
    /// The Ayatsuri API (Cubism's external integration API): on/off, port
    /// (0 is Cubism's 22033), network access, and the apps the user allowed.
    pub api_enabled: bool,
    pub api_port: u16,
    pub api_remote: bool,
    pub api_clients: Vec<crate::api::protocol::Client>,
}

/// The arrow tool's display settings, RGBA.
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay {
    pub mesh_width: Option<f32>,
    pub mesh_color: Option<[u8; 4]>,
    pub warp_color: Option<[u8; 4]>,
    pub rotation_color: Option<[u8; 4]>,
    pub weight_color: Option<[u8; 4]>,
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("ayatsuri2d").join("preferences.json"))
}

impl Prefs {
    /// The saved primary colour with its alpha, if one was picked.
    pub fn accent_color(&self) -> Option<egui::Color32> {
        let a = self.accent_alpha.unwrap_or(255);
        self.accent.map(|[r, g, b]| egui::Color32::from_rgba_unmultiplied(r, g, b, a))
    }

    /// The saved canvas background with its alpha, if one was picked.
    pub fn view_bg_color(&self) -> Option<egui::Color32> {
        let a = self.view_bg_alpha.unwrap_or(255);
        self.view_bg.map(|[r, g, b]| egui::Color32::from_rgba_unmultiplied(r, g, b, a))
    }

    /// The interface look these preferences ask for.
    pub fn look(&self) -> crate::theme::Look {
        let d = crate::theme::Look::default();
        crate::theme::Look {
            text: self.text_size.unwrap_or(d.text),
            density: self.density.unwrap_or(d.density),
            corners: self.corners.unwrap_or(d.corners),
            reduce_motion: self.reduce_motion,
        }
    }

    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(bytes) = serde_json::to_vec_pretty(self) else { return };
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }

    pub fn remember(&mut self, file: PathBuf) {
        self.recents.retain(|r| r != &file);
        self.recents.insert(0, file);
        self.recents.truncate(8);
        self.save();
    }
}
