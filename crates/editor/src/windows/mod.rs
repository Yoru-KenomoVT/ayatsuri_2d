// Floating windows: About, Settings, the API window, Project, Model
// Statistics, Export Model and Render. Docked palettes live in panels.

pub(crate) mod about;
mod api;
pub(crate) mod atlas;
mod dialog;
mod export_moc3;
mod model_info;
pub(crate) mod recovery;
pub(crate) mod render;
pub(crate) mod settings;
mod stats;

pub(crate) use dialog::{Dialog, form};

/// A time left or taken, short: "42 s", "3 min 05 s", "1 h 12 min".
pub(crate) fn span_short(secs: f32) -> String {
    use crate::i18n::trf;
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        trf("{h} h {m} min", &[("h", &(s / 3600)), ("m", &(s % 3600 / 60))])
    } else if s >= 60 {
        trf("{m} min {s} s", &[("m", &(s / 60)), ("s", &(s % 60))])
    } else {
        trf("{s} s", &[("s", &s)])
    }
}

/// A download's bar, drawn as the render queue draws a job: a thin track
/// filling in the accent colour, "12 of 190 MB" and the time left under it
/// in small faint type, the percentage on the right.
#[derive(Default)]
pub(crate) struct ByteBar {
    started: Option<std::time::Instant>,
    /// The time left as last shown, eased and refreshed once a second so
    /// it reads steadily.
    eta: Option<(std::time::Instant, f32)>,
}

impl ByteBar {
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, (done, total): (u64, u64)) {
        use crate::i18n::{tr, trf};
        use egui::{Align2, FontId, Rect, Sense, pos2, vec2};
        let pal = crate::theme::pal();
        let now = std::time::Instant::now();
        let started = *self.started.get_or_insert(now);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
        let p = ui.painter();
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 4.0));
        p.rect_filled(bar, 2.0, pal.track);
        let f = if total > 0 { (done as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
        if f > 0.0 {
            p.rect_filled(
                Rect::from_min_size(bar.min, vec2(bar.width() * f, 4.0)),
                2.0,
                pal.accent,
            );
        }
        let line = pos2(rect.min.x, rect.min.y + 10.0);
        let mb = |b: u64| format!("{:.0}", b as f64 / 1_048_576.0);
        let mut words = if total > 0 {
            trf("{done} of {total} MB", &[("done", &mb(done)), ("total", &mb(total))])
        } else {
            tr("Starting the download").to_string()
        };
        if done > 0 && total > done {
            let new =
                now.duration_since(started).as_secs_f32() / done as f32 * (total - done) as f32;
            if self.eta.is_none_or(|(at, _)| now.duration_since(at).as_secs_f32() >= 1.0) {
                let eased = self.eta.map_or(new, |(_, old)| old * 0.6 + new * 0.4);
                self.eta = Some((now, eased));
            }
        }
        if let Some((_, left)) = self.eta {
            let left = if left >= 60.0 { (left / 5.0).round() * 5.0 } else { left };
            words.push_str("   ");
            words.push_str(&trf("{time} left", &[("time", &span_short(left))]));
        }
        p.text(line, Align2::LEFT_TOP, words, FontId::proportional(11.0), pal.text_faint);
        if total > 0 {
            p.text(
                pos2(rect.max.x - 2.0, line.y),
                Align2::RIGHT_TOP,
                format!("{:.0}%", f * 100.0),
                FontId::monospace(11.0),
                pal.text,
            );
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
}
