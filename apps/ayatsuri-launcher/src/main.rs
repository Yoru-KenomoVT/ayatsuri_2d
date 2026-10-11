//! The splash launcher: an Adobe-style card with the logo over a drifting
//! particle field, which hands off to the real app after a beat. The window
//! is fixed size and undecorated, so tiling compositors float it.

#![forbid(unsafe_code)]

use std::time::Instant;

const W: f32 = 460.0;
const H: f32 = 300.0;
const HOLD: f32 = 2.6;

struct Launcher {
    started: Instant,
    logo: Option<egui::TextureHandle>,
    launched: bool,
    open_arg: Option<String>,
}

// a cheap deterministic hash so the field looks random but never allocates
fn fract(seed: f32) -> f32 {
    let x = (seed * 12.9898).sin() * 43758.547;
    x - x.floor()
}

impl Launcher {
    fn launch(&mut self, ctx: &egui::Context) {
        if self.launched {
            return;
        }
        self.launched = true;
        // prefer the sibling binary, fall back to PATH
        let sibling = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("ayatsuri")))
            .filter(|p| p.exists());
        let mut cmd = match sibling {
            Some(p) => std::process::Command::new(p),
            None => std::process::Command::new("ayatsuri"),
        };
        if let Some(arg) = &self.open_arg {
            cmd.arg(arg);
        }
        match cmd.spawn() {
            Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => {
                log::error!("could not start ayatsuri: {e}");
                self.launched = false;
            }
        }
    }
}

impl eframe::App for Launcher {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let rect = ctx.content_rect();
        let t = self.started.elapsed().as_secs_f32();
        let p = ui.painter();

        // the card: a deep vertical gradient under a hairline border
        let accent = egui::Color32::from_rgb(0xF0, 0x8E, 0x1E);
        let top = egui::Color32::from_rgb(0x1c, 0x17, 0x12);
        let bottom = egui::Color32::from_rgb(0x0e, 0x0b, 0x09);
        let bands = 36;
        for i in 0..bands {
            let f = i as f32 / bands as f32;
            let col = egui::Color32::from_rgb(
                (top.r() as f32 + (bottom.r() as f32 - top.r() as f32) * f) as u8,
                (top.g() as f32 + (bottom.g() as f32 - top.g() as f32) * f) as u8,
                (top.b() as f32 + (bottom.b() as f32 - top.b() as f32) * f) as u8,
            );
            p.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.min.x, rect.min.y + rect.height() * f),
                    egui::vec2(rect.width(), rect.height() / bands as f32 + 1.0),
                ),
                if i == 0 || i == bands - 1 { 14.0 } else { 0.0 },
                col,
            );
        }
        p.rect_stroke(
            rect,
            14.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x45, 0x37, 0x27)),
            egui::StrokeKind::Inside,
        );

        // slow ember drift with faint constellation links
        let mut pts = [egui::Pos2::ZERO; 56];
        for (i, pt) in pts.iter_mut().enumerate() {
            let fi = i as f32;
            let sx = fract(fi + 0.13);
            let sy = fract(fi + 7.71);
            let vx = (fract(fi + 3.3) - 0.5) * 9.0 + (t * 0.4 + fi).sin() * 2.0;
            let vy = -4.0 - fract(fi + 5.2) * 9.0;
            let x = (sx * rect.width() + vx * t).rem_euclid(rect.width());
            let y = (sy * rect.height() + vy * t).rem_euclid(rect.height());
            *pt = egui::pos2(rect.min.x + x, rect.min.y + y);
        }
        for (i, a) in pts.iter().enumerate() {
            for b in pts.get(i + 1..).unwrap_or(&[]) {
                let d = a.distance(*b);
                if d < 64.0 {
                    let fade = (1.0 - d / 64.0) * 0.07;
                    p.line_segment([*a, *b], egui::Stroke::new(0.5, accent.gamma_multiply(fade)));
                }
            }
        }
        for (i, pt) in pts.iter().enumerate() {
            let tw = 0.5 + 0.5 * ((t * 1.1 + i as f32 * 1.7).sin() * 0.5 + 0.5);
            let r = 0.8 + fract(i as f32 + 9.9) * 1.6;
            p.circle_filled(*pt, r, accent.gamma_multiply(0.08 + 0.16 * tw));
        }

        // a soft glow pooled behind the logo
        let glow_c = egui::pos2(rect.center().x, rect.center().y - 16.0);
        for i in 0..10 {
            let f = 1.0 - i as f32 / 10.0;
            p.circle_filled(glow_c, 36.0 + i as f32 * 14.0, accent.gamma_multiply(0.015 * f));
        }

        // the logo, faded in over the first moments
        let fade = (t / 0.7).clamp(0.0, 1.0);
        match &self.logo {
            Some(tex) => {
                let size = tex.size_vec2();
                let h = 72.0;
                let w = if size.y > 0.0 { h * size.x / size.y } else { h * 4.0 };
                let draw = egui::Rect::from_center_size(
                    egui::pos2(rect.center().x, rect.center().y - 16.0),
                    egui::vec2(w.min(rect.width() - 60.0), h),
                );
                p.image(
                    tex.id(),
                    draw,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE.gamma_multiply(fade),
                );
            }
            None => {
                p.text(
                    egui::pos2(rect.center().x, rect.center().y - 16.0),
                    egui::Align2::CENTER_CENTER,
                    "操り2D",
                    egui::FontId::proportional(52.0),
                    accent.gamma_multiply(fade),
                );
            }
        }
        p.text(
            egui::pos2(rect.center().x, rect.max.y - 64.0),
            egui::Align2::CENTER_CENTER,
            format!("Ayatsuri2D  ·  {}", env!("CARGO_PKG_VERSION")),
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(0xc4).gamma_multiply(fade),
        );
        p.text(
            egui::pos2(rect.center().x, rect.max.y - 44.0),
            egui::Align2::CENTER_CENTER,
            "open 2D puppet studio",
            egui::FontId::proportional(10.5),
            egui::Color32::from_gray(0x8a).gamma_multiply(fade),
        );

        // the progress: a dark track with a rounded accent fill
        let prog = (t / HOLD).clamp(0.0, 1.0);
        let track = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + 28.0, rect.max.y - 26.0),
            egui::vec2(rect.width() - 56.0, 3.0),
        );
        p.rect_filled(track, 2.0, egui::Color32::from_gray(0x24));
        let fill = egui::Rect::from_min_size(track.min, egui::vec2(track.width() * prog, 3.0));
        p.rect_filled(fill, 2.0, accent.gamma_multiply(0.95));
        // a soft head glow on the moving edge
        p.circle_filled(egui::pos2(fill.max.x, fill.center().y), 4.0, accent.gamma_multiply(0.35));

        if ui.input(|i| i.pointer.any_click()) || prog >= 1.0 {
            self.launch(&ctx);
        }
        ctx.request_repaint();
    }
}

fn load_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let bytes = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/logo-dark.png"));
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, &img.into_raw());
    Some(ctx.load_texture("logo", color, egui::TextureOptions::LINEAR))
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ayatsuri2D")
            .with_inner_size([W, H])
            .with_min_inner_size([W, H])
            .with_max_inner_size([W, H])
            .with_resizable(false)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top(),
        ..Default::default()
    };
    let open_arg = std::env::args().nth(1);
    eframe::run_native(
        "ayatsuri2d-launcher",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Launcher {
                started: Instant::now(),
                logo: load_logo(&cc.egui_ctx),
                launched: false,
                open_arg,
            }))
        }),
    )
}
