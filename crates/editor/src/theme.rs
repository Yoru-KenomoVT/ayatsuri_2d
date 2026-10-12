// Light and dark palettes. Accent is the orange from the Ayatsuri2D logo.
// Light mode follows Cubism Editor's grays; dark mode is our own.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use egui::{Color32, Context, FontId, TextStyle};

#[derive(Clone, Copy)]
pub struct Palette {
    pub chrome: Color32,
    pub panel_bg: Color32,
    pub header: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_faint: Color32,
    pub select: Color32,
    pub accent: Color32,
    pub warp_green: Color32,
    pub rotation_red: Color32,
    pub canvas_bg: Color32,
    pub canvas: Color32,
    pub key_ring: Color32,
    pub track: Color32,
    pub eye_col: Color32,
}

const fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub const ACCENT: Color32 = c(0xF0, 0x8E, 0x1E);

pub const LIGHT: Palette = Palette {
    chrome: c(0xEC, 0xEC, 0xEC),
    panel_bg: c(0xFF, 0xFF, 0xFF),
    header: c(0xE2, 0xE2, 0xE2),
    border: c(0xC8, 0xC8, 0xC8),
    text: c(0x33, 0x33, 0x33),
    text_faint: c(0x9A, 0x9A, 0x9A),
    select: c(0xFA, 0xE3, 0xC3),
    accent: ACCENT,
    warp_green: c(0x3A, 0xA6, 0x55),
    rotation_red: c(0xE0, 0x56, 0x56),
    canvas_bg: c(0xBD, 0xBD, 0xBD),
    canvas: c(0xFF, 0xFF, 0xFF),
    key_ring: c(0x8A, 0x8A, 0x8A),
    track: c(0xAF, 0xAF, 0xAF),
    eye_col: c(0xDE, 0xE8, 0xF4),
};

pub const DARK: Palette = Palette {
    chrome: c(0x1E, 0x1E, 0x21),
    panel_bg: c(0x29, 0x29, 0x2D),
    header: c(0x24, 0x24, 0x27),
    border: c(0x17, 0x17, 0x19),
    text: c(0xE4, 0xE4, 0xE8),
    text_faint: c(0x8E, 0x8E, 0x96),
    select: c(0x4A, 0x39, 0x1F),
    accent: ACCENT,
    warp_green: c(0x4C, 0xC0, 0x6A),
    rotation_red: c(0xE8, 0x6A, 0x6A),
    canvas_bg: c(0x35, 0x35, 0x3A),
    canvas: c(0xFF, 0xFF, 0xFF),
    key_ring: c(0x7A, 0x7A, 0x84),
    track: c(0x55, 0x55, 0x5E),
    eye_col: c(0x2E, 0x3A, 0x47),
};

static DARK_ON: AtomicBool = AtomicBool::new(true);
// 0 means "use the default logo orange"; anything else is bit 32 set over
// the unmultiplied 0xrrggbbaa, so a picked alpha survives
static ACCENT_OVERRIDE: AtomicU64 = AtomicU64::new(0);

pub fn set_accent(ctx: &Context, color: Option<Color32>) {
    let packed = color.map_or(0, |c| {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        1u64 << 32 | u64::from(u32::from_be_bytes([r, g, b, a]))
    });
    ACCENT_OVERRIDE.store(packed, Ordering::Relaxed);
    install(ctx);
}

pub fn accent() -> Color32 {
    let v = ACCENT_OVERRIDE.load(Ordering::Relaxed);
    if v == 0 {
        return ACCENT;
    }
    let [r, g, b, a] = (v as u32).to_be_bytes();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

pub fn is_dark() -> bool {
    DARK_ON.load(Ordering::Relaxed)
}

pub fn pal() -> Palette {
    let mut p = if is_dark() { DARK } else { LIGHT };
    p.accent = accent();
    // the selection tint follows the primary color
    let a = p.accent;
    let b = p.panel_bg;
    let mix = |x: u8, y: u8| ((x as u16 * 2 + y as u16 * 5) / 7) as u8;
    p.select = Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()));
    p
}

/// The shape of the interface, from Settings > Appearance.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    /// Body text size in points (12 is the default).
    pub text: f32,
    /// Spacing multiplier: 1 compact (the default), up to 1.6 roomy.
    pub density: f32,
    /// Widget corner radius in points; windows get a little more.
    pub corners: f32,
    /// No animated expand, fade or scroll transitions.
    pub reduce_motion: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self { text: 12.0, density: 1.0, corners: 3.0, reduce_motion: false }
    }
}

static LOOK: std::sync::Mutex<Option<Look>> = std::sync::Mutex::new(None);

pub fn look() -> Look {
    LOOK.lock().ok().and_then(|g| *g).unwrap_or_default()
}

pub fn set_look(ctx: &Context, look: Look) {
    if let Ok(mut g) = LOOK.lock() {
        *g = Some(look);
    }
    install(ctx);
}

pub fn set_dark(ctx: &Context, dark: bool) {
    DARK_ON.store(dark, Ordering::Relaxed);
    install(ctx);
}

pub fn install(ctx: &Context) {
    let p = pal();
    let dark = is_dark();
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let v = &mut style.visuals;
    v.panel_fill = p.chrome;
    v.window_fill = p.panel_bg;
    v.extreme_bg_color = if dark { c(0x1A, 0x1A, 0x1D) } else { Color32::WHITE };
    v.faint_bg_color = p.header;
    v.selection.bg_fill = p.select;
    v.selection.stroke = egui::Stroke::new(1.0, p.text);
    v.hyperlink_color = p.accent;
    v.widgets.noninteractive.bg_fill = p.chrome;
    v.widgets.noninteractive.fg_stroke.color = p.text;
    v.widgets.inactive.fg_stroke.color = p.text;
    v.widgets.hovered.fg_stroke.color = if dark { Color32::WHITE } else { Color32::BLACK };
    v.widgets.active.fg_stroke.color = if dark { Color32::WHITE } else { Color32::BLACK };
    v.override_text_color = Some(p.text);
    // labels are not selectable: dragging a window or a slider across
    // text kept highlighting it; text boxes still select as normal
    style.interaction.selectable_labels = false;
    style.interaction.multi_widget_text_select = false;
    let look = look();
    let d = look.density.clamp(0.8, 2.0);
    style.spacing.item_spacing = egui::vec2(6.0 * d, 4.0 * d);
    style.spacing.button_padding = egui::vec2(6.0 * d, 2.0 * d);
    style.spacing.interact_size.y = (18.0 * d.sqrt()).max(16.0);
    let t = look.text.clamp(9.0, 18.0);
    style.text_styles.insert(TextStyle::Body, FontId::proportional(t));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(t));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(t - 1.5));
    style.text_styles.insert(TextStyle::Heading, FontId::proportional(t + 1.0));
    let r = egui::CornerRadius::same(look.corners.clamp(0.0, 12.0) as u8);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }
    let big = egui::CornerRadius::same((look.corners * 2.0).clamp(0.0, 16.0) as u8);
    v.window_corner_radius = big;
    v.menu_corner_radius = big;
    if look.reduce_motion {
        style.animation_time = 0.0;
        style.scroll_animation = egui::style::ScrollAnimation::none();
    }
    ctx.set_style_of(theme, style);
    // a press becomes a drag after 3 px instead of egui's 6, so a dragged
    // point barely snaps to the pointer when the drag kicks in; still
    // loose enough that a slightly shaky click stays a click
    ctx.options_mut(|o| o.input_options.max_click_dist = 3.0);
    // one deterministic font set: defaults + phosphor icons + a system CJK
    // face; built together so nothing resets anything else
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    if let Some(data) = system_cjk() {
        fonts.font_data.insert("system-cjk".into(), std::sync::Arc::new(data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push("system-cjk".into());
            }
        }
    }
    ctx.set_fonts(fonts);
}

// Names in model files are usually Japanese or Chinese and egui's default
// fonts have neither. Candidate paths per platform; packaging will embed a
// face instead.
fn system_cjk() -> Option<egui::epaint::text::FontData> {
    const CANDIDATES: &[(&str, u32)] = &[
        ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 0),
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
        ("/usr/share/fonts/noto-cjk/NotoSansCJKjp-Regular.otf", 0),
        ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
        ("C:/Windows/Fonts/YuGothM.ttc", 0),
        ("C:/Windows/Fonts/msyh.ttc", 0),
    ];
    for (path, index) in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            return Some(egui::epaint::text::FontData {
                font: std::borrow::Cow::Owned(bytes),
                index: *index,
                tweak: Default::default(),
            });
        }
    }
    None
}
