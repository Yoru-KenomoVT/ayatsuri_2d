// Saved preferences: theme, recent files, and the dock layouts, written as
// json under the user's config dir. Written atomically (tmp + rename) so a
// crash never half-writes the file.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::dock::DockNode;

/// What the Render window keeps between sessions. Zero or None means
/// the window's own default.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct RenderPrefs {
    pub folder: Option<PathBuf>,
    pub format: Option<String>,
    pub width: u32,
    pub fps: u32,
    pub sps: f32,
    pub hold: f32,
    pub transparent: bool,
    pub gpu: Option<bool>,
    pub aa: Option<bool>,
    /// A composition's sound left out of the video.
    pub sound_off: bool,
    /// Sound bitrate for MP4 and WebM, in kbit/s.
    pub audio_kbps: u32,
}

/// One Automatic mesh generator preset, Cubism's settings by name. The
/// margins are pixels on a 1024px texture, scaled with the texture's size.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct MeshPreset {
    pub name: String,
    pub interval_outside: f32,
    pub interval_inside: f32,
    pub margin_outside: f32,
    pub margin_inside: f32,
    pub min_margin: f32,
    pub min_points: u32,
    /// Alpha value determined as transparent.
    pub alpha: u8,
    /// Ayatsuri's face layout instead of Cubism's.
    pub face: bool,
}

impl Default for MeshPreset {
    /// Cubism's Standard.
    fn default() -> Self {
        Self {
            name: "Standard".into(),
            interval_outside: 50.0,
            interval_inside: 50.0,
            margin_outside: 14.0,
            margin_inside: 14.0,
            min_margin: 5.0,
            min_points: 10,
            alpha: 0,
            face: false,
        }
    }
}

impl MeshPreset {
    /// The presets that come with the app: Cubism's three, then the face.
    pub fn built_in() -> Vec<MeshPreset> {
        let standard = MeshPreset::default();
        vec![
            standard.clone(),
            MeshPreset {
                name: "Deformation (small)".into(),
                interval_outside: 80.0,
                interval_inside: 80.0,
                min_points: 15,
                ..standard.clone()
            },
            MeshPreset {
                name: "Deformation (large)".into(),
                interval_outside: 25.0,
                interval_inside: 25.0,
                margin_outside: 3.0,
                margin_inside: 2.0,
                min_margin: 2.0,
                min_points: 5,
                ..standard.clone()
            },
            MeshPreset {
                name: "Face standard".into(),
                interval_outside: 36.0,
                interval_inside: 72.0,
                margin_outside: 6.0,
                margin_inside: 0.0,
                min_margin: 0.0,
                face: true,
                ..standard
            },
        ]
    }

    /// The same settings, whatever the name.
    pub fn same_values(&self, o: &MeshPreset) -> bool {
        MeshPreset { name: String::new(), ..self.clone() }
            == MeshPreset { name: String::new(), ..o.clone() }
    }
}

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
    /// The model's canvas (the page under the art), when not the theme's.
    pub canvas_page: Option<[u8; 3]>,
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
    /// Sound output: a device by name (None follows the system default),
    /// the master volume in percent (None is 100) and the buffer size in
    /// frames (0 is the default, 1024).
    pub audio_device: Option<String>,
    pub audio_volume: Option<u32>,
    pub audio_buffer: u32,
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
    /// Leave the deformer tree off the top of the canvas right-click menu
    /// (Cubism's Show parent-child hierarchy of deformers, cleared).
    pub def_tree_off: bool,
    pub guide_snap_off: bool,
    /// Smooth movement while dragging points and objects, and how much
    /// (1 to 100; 0 is the default, 40).
    pub smooth_drag: bool,
    pub smooth_drag_amount: u32,
    /// Pen tablet: looser clicks for a pen hand (a tap may drift further,
    /// a double tap take longer).
    pub pen_mode: bool,
    /// Ignore pen pressure altogether.
    pub pen_pressure_off: bool,
    /// Pressure sets brush size (off by default) and strength (on).
    pub pen_size: bool,
    pub pen_strength_off: bool,
    /// The pressure curve, 0.3 to 3 (0 is the default, 1: linear).
    pub pen_curve: f32,
    /// What the lightest touch still gives, 1 to 100 percent (0 is the
    /// default, 10).
    pub pen_min: u32,
    /// Don't record the timelapse beside each model.
    pub timelapse_off: bool,
    /// Don't lay the atlas out again on leaving Mesh Edit when a mesh has
    /// grown past its own art (keeps a layout arranged by hand).
    pub atlas_repack_off: bool,
    /// Experimental features (Settings > Experimental): the master switch,
    /// then one per feature; a feature is on only with both.
    pub experimental: bool,
    /// Named clip groups in the Inspector's clipping.
    pub experimental_clip_groups: bool,
    /// Inherited clipping: a mesh also clipped by what its masks are
    /// clipped by.
    pub experimental_inherit_clip: bool,
    /// Stroke mesh-mapping in Mesh Edit: a mesh built along a dragged stroke.
    #[serde(default)]
    pub experimental_stroke_mesh: bool,
    /// Solo's inactive color, rgba; None for the default.
    pub solo_tint: Option<[u8; 4]>,
    /// Highlight overlapping polygons' color, rgba; None for the default.
    pub overlap_color: Option<[u8; 4]>,
    /// Automatic mesh generator presets the user added (Cubism's Add).
    pub mesh_presets: Vec<MeshPreset>,
    /// The Render window as it was last used: folder, format and settings.
    pub render: RenderPrefs,
    /// Where the last moc3 or image export went.
    pub export_folder: Option<PathBuf>,
    /// The release the user chose "Skip this version" for; the update
    /// window does not offer it again.
    pub update_dismissed: Option<String>,
    /// The version that last started, so What's new opens once per version.
    pub last_seen_version: Option<String>,
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
    Some(ayatsuri_update::config_dir()?.join("preferences.json"))
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

    /// Smooth movement's amount, 1 to 100.
    pub fn pen_curve(&self) -> f32 {
        if self.pen_curve <= 0.0 { 1.0 } else { self.pen_curve.clamp(0.3, 3.0) }
    }

    pub fn pen_min(&self) -> f32 {
        if self.pen_min == 0 { 0.1 } else { self.pen_min.min(100) as f32 / 100.0 }
    }

    pub fn smooth_drag_amount(&self) -> u32 {
        if self.smooth_drag_amount == 0 { 40 } else { self.smooth_drag_amount.min(100) }
    }

    /// How long a smoothed drag takes to catch most of the way up, in
    /// seconds; 0 when smooth movement is off. 100 is a fifth of a second.
    pub fn smooth_drag_seconds(&self) -> f32 {
        if self.smooth_drag { self.smooth_drag_amount() as f32 / 500.0 } else { 0.0 }
    }

    pub fn clip_groups_on(&self) -> bool {
        self.experimental && self.experimental_clip_groups
    }

    pub fn inherit_clip_on(&self) -> bool {
        self.experimental && self.experimental_inherit_clip
    }

    pub fn stroke_mesh_on(&self) -> bool {
        self.experimental && self.experimental_stroke_mesh
    }

    /// The saved colour of the canvas page, if one was picked.
    pub fn canvas_page_color(&self) -> Option<egui::Color32> {
        self.canvas_page.map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
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

    /// The sound output as the audio module wants it.
    pub fn audio(&self) -> crate::audio::Prefs {
        crate::audio::Prefs {
            device: self.audio_device.clone(),
            volume: self.audio_volume.unwrap_or(100).min(200) as f32 / 100.0,
            buffer_frames: match self.audio_buffer {
                0 => 1024,
                n => n,
            },
        }
    }

    pub fn load() -> Self {
        let Some(p) = path() else {
            log::warn!("settings: no config folder, using the defaults");
            return Self::default();
        };
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::debug!("settings: none saved yet, using the defaults");
                return Self::default();
            }
            Err(e) => {
                log::warn!("settings: could not read {}: {e}; using the defaults", p.display());
                return Self::default();
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(prefs) => prefs,
            Err(e) => {
                // the file stays as it is until the next save replaces it
                log::warn!("settings: {} is not valid ({e}); using the defaults", p.display());
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            log::warn!("settings: could not make {}: {e}", dir.display());
        }
        let bytes = match serde_json::to_vec_pretty(self) {
            Ok(b) => b,
            Err(e) => {
                log::error!("settings: could not encode them: {e}");
                return;
            }
        };
        log_changes(&p, &bytes);
        let tmp = p.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &p)) {
            log::warn!("settings: could not save {}: {e}", p.display());
        }
    }

    pub fn remember(&mut self, file: PathBuf) {
        self.recents.retain(|r| r != &file);
        self.recents.insert(0, file);
        self.recents.truncate(8);
        self.save();
    }
}

/// Settings that move on their own (layouts, recent files, last folders):
/// a change in them is not something the user set, so it is not logged.
const QUIET: &[&str] = &[
    "recents",
    "toolbar_pos",
    "workspace_modeling",
    "workspace_animation",
    "custom_workspaces",
    "physics_layout",
    "export_folder",
];

/// Logged by name only: the API clients hold their tokens.
const NAME_ONLY: &[&str] = &["api_clients", "discord_app_id", "keybinds", "render"];

/// The last saved settings, and which ones the last log line named and
/// when: a slider saves on every frame of a drag, which logs once.
static LAST_SAVED: std::sync::Mutex<Option<(serde_json::Value, String, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// Log which settings a save changes, against the last save (the saved file
/// the first time).
fn log_changes(path: &std::path::Path, new: &[u8]) {
    let Ok(new) = serde_json::from_slice::<serde_json::Value>(new) else { return };
    let Ok(mut last) = LAST_SAVED.lock() else { return };
    let (old, logged, at) = last.take().unwrap_or_else(|| {
        let old = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(serde_json::Value::Null);
        (old, String::new(), std::time::Instant::now())
    });
    let mut keys = Vec::new();
    let mut shown = Vec::new();
    if let (Some(old), Some(obj)) = (old.as_object(), new.as_object()) {
        for (k, v) in obj {
            if QUIET.contains(&k.as_str()) || old.get(k) == Some(v) {
                continue;
            }
            keys.push(k.as_str());
            let value = v.to_string();
            shown.push(if NAME_ONLY.contains(&k.as_str()) || value.len() > 40 {
                k.clone()
            } else {
                format!("{k} = {value}")
            });
        }
    }
    let keys = keys.join(",");
    let repeat = !keys.is_empty() && keys == logged && at.elapsed().as_secs_f32() < 1.0;
    if !shown.is_empty() && !repeat {
        log::info!("settings changed: {}", shown.join(", "));
    }
    let (logged, at) =
        if keys.is_empty() { (logged, at) } else { (keys, std::time::Instant::now()) };
    *last = Some((new, logged, at));
}
