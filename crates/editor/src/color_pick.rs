// The colour menu behind every label colour (parameters, folders, keys): a
// palette of presets, and the same colour button as Settings for anything
// else.

use std::f32::consts::TAU;

use egui::ecolor::Hsva;
use egui::{Color32, Mesh, Sense, Shape, Stroke, Ui, vec2};

use crate::theme;

/// Rows of presets: vivid, pastel, deep, and greys.
pub const PRESETS: [[u8; 3]; 32] = [
    [0xE5, 0x48, 0x48],
    [0xF0, 0x8E, 0x1E],
    [0xE8, 0xC6, 0x3B],
    [0x5F, 0xB3, 0x5F],
    [0x3B, 0xB3, 0xA8],
    [0x4E, 0x8F, 0xD9],
    [0x9B, 0x6B, 0xD3],
    [0xE5, 0x60, 0x9B],
    [0xF4, 0xA3, 0xA3],
    [0xF7, 0xC4, 0x8E],
    [0xF2, 0xE2, 0x9B],
    [0xA8, 0xD8, 0xA8],
    [0x9E, 0xD9, 0xD3],
    [0xA3, 0xC4, 0xEF],
    [0xC9, 0xB1, 0xEA],
    [0xF2, 0xAF, 0xCB],
    [0x8E, 0x2A, 0x2A],
    [0x9A, 0x53, 0x10],
    [0x8C, 0x74, 0x17],
    [0x2F, 0x6E, 0x33],
    [0x1E, 0x6B, 0x64],
    [0x28, 0x4F, 0x86],
    [0x5A, 0x3B, 0x85],
    [0x8A, 0x2F, 0x5B],
    [0xFF, 0xFF, 0xFF],
    [0xD6, 0xD6, 0xD8],
    [0xA6, 0xA6, 0xAA],
    [0x7A, 0x7A, 0x80],
    [0x55, 0x55, 0x5B],
    [0x3A, 0x3A, 0x3E],
    [0x25, 0x25, 0x28],
    [0x00, 0x00, 0x00],
];

fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let [r, g, b] = Hsva::new(h, s, v, 1.0).to_srgb();
    Color32::from_rgb(r, g, b)
}

/// The tile that opens the colour window: a square filled with a colour
/// wheel (hue around, white in the middle).
fn wheel_tile(ui: &mut Ui) -> egui::Response {
    let side = 18.0;
    let (r, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    let c = r.center();
    let mut m = Mesh::default();
    let g = 10u32;
    for y in 0..=g {
        for x in 0..=g {
            let p = r.min + vec2(x as f32, y as f32) * (side / g as f32);
            let d = p - c;
            let hue = (d.y.atan2(d.x) / TAU).rem_euclid(1.0);
            let sat = (d.length() / (side / 2.0)).min(1.0);
            m.colored_vertex(p, hsv(hue, sat, 1.0));
        }
    }
    for y in 0..g {
        for x in 0..g {
            let i = y * (g + 1) + x;
            m.add_triangle(i, i + 1, i + g + 1);
            m.add_triangle(i + 1, i + g + 2, i + g + 1);
        }
    }
    ui.painter().add(Shape::mesh(m));
    let edge = if resp.hovered() {
        Stroke::new(2.0, theme::pal().accent)
    } else {
        Stroke::new(1.0, theme::pal().border)
    };
    ui.painter().rect_stroke(r, 3.0, edge, egui::StrokeKind::Outside);
    resp.on_hover_text(crate::i18n::tr("Custom colour…"))
}

pub fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// Text that reads on a background of `rgb`.
pub fn text_on([r, g, b]: [u8; 3]) -> Color32 {
    let luma = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    if luma > 150.0 { Color32::from_gray(0x1A) } else { Color32::WHITE }
}

/// What the colour menu was asked for.
pub enum Pick {
    /// A colour, or None for Clear.
    Set(Option<[u8; 3]>),
    /// Open the window with Settings' colour button.
    Custom,
}

/// The palette, then Settings' colour button for any other colour.
/// `current` marks the colour now. A preset or Clear closes the menu; the
/// picker applies the colour as it changes.
pub fn picker(ui: &mut Ui, current: Option<[u8; 3]>) -> Option<Pick> {
    let mut out = None;
    // debug: log when the menu first shows, not every frame it stays up
    let shown_id = ui.id().with("colour-menu-shown");
    let frame = ui.ctx().cumulative_pass_nr();
    let last = ui.data(|d| d.get_temp::<u64>(shown_id));
    if last.is_none_or(|l| l + 1 < frame) {
        log::debug!("colour menu: shown, current {}", current.map_or("none".into(), hex));
    }
    ui.data_mut(|d| d.insert_temp(shown_id, frame));
    for row in PRESETS.chunks(8) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for &c in row {
                let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                ui.painter().rect_filled(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]));
                let edge = if current == Some(c) || resp.hovered() {
                    Stroke::new(2.0, theme::pal().accent)
                } else {
                    Stroke::new(1.0, theme::pal().border)
                };
                ui.painter().rect_stroke(r, 3.0, edge, egui::StrokeKind::Outside);
                if resp.on_hover_text(hex(c)).clicked() {
                    log::debug!("colour menu: preset {}", hex(c));
                    out = Some(Pick::Set(Some(c)));
                    ui.close();
                }
            }
        });
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // any other colour: Settings' colour button, in a window of its
        // own (its picker can't live inside a menu)
        if wheel_tile(ui).clicked() {
            log::debug!("colour menu: custom colour");
            out = Some(Pick::Custom);
            ui.close();
        }
        if ui.small_button(crate::i18n::tr("Clear")).clicked() {
            log::debug!("colour menu: clear");
            out = Some(Pick::Set(None));
            ui.close();
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_reads_as_designers_write_it() {
        assert_eq!(hex([0xF0, 0x8E, 0x1E]), "#F08E1E");
    }

    #[test]
    fn text_stays_readable() {
        assert_eq!(text_on([0xFF, 0xFF, 0xFF]), Color32::from_gray(0x1A));
        assert_eq!(text_on([0x28, 0x4F, 0x86]), Color32::WHITE);
    }
}
