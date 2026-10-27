// Pen tablets. The window layer hands pen pressure over as a touch with a
// force (Windows pens do; Linux gets none from it yet), and the same pen
// also drives the pointer, so every tool works with it as with a mouse.
// Here the force is read each frame, smoothed against jitter, and shaped
// by the user's curve; the brushes ask for it as a size and a strength
// factor. Pen mode also loosens what counts as a click, since a pen hand
// drifts a few pixels on a tap.

use egui::{Event, TouchPhase};

use crate::prefs::Prefs;

/// How far the smoothed pressure moves toward a new reading each event.
const SMOOTHING: f32 = 0.5;

#[derive(Default)]
pub(crate) struct Pen {
    /// The pen is down and reporting pressure: the smoothed reading.
    down: Option<f32>,
    /// The last pressure seen, kept after the pen lifts, for the meter.
    pub(crate) last: Option<f32>,
}

impl Pen {
    /// Take this frame's pen readings from the input events.
    pub(crate) fn read(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            for e in &i.events {
                if let Event::Touch { phase, force, .. } = e {
                    self.take(*phase, *force);
                }
            }
        });
    }

    fn take(&mut self, phase: TouchPhase, force: Option<f32>) {
        match (phase, force) {
            (TouchPhase::Start | TouchPhase::Move, Some(f)) => {
                let f = f.clamp(0.0, 1.0);
                let now = match self.down {
                    Some(was) => was + (f - was) * SMOOTHING,
                    None => f,
                };
                self.down = Some(now);
                self.last = Some(now);
            }
            (TouchPhase::End | TouchPhase::Cancel, _) => self.down = None,
            _ => {}
        }
    }

    /// The shaped pressure while the pen is down, None for a mouse (or a
    /// pen whose pressure the platform does not report).
    pub(crate) fn pressure(&self, prefs: &Prefs) -> Option<f32> {
        if prefs.pen_pressure_off {
            return None;
        }
        self.down.map(|p| shape(p, prefs.pen_curve(), prefs.pen_min()))
    }

    /// What pressure does to a brush now: (size factor, strength factor),
    /// both 1 with a mouse.
    pub(crate) fn brush(&self, prefs: &Prefs) -> (f32, f32) {
        match self.pressure(prefs) {
            Some(p) => {
                (if prefs.pen_size { p } else { 1.0 }, if prefs.pen_strength_off { 1.0 } else { p })
            }
            None => (1.0, 1.0),
        }
    }
}

/// A raw pressure through the user's curve: `curve` above 1 asks for a
/// firmer press, below 1 a lighter one; the lightest touch still gives
/// `min`.
pub(crate) fn shape(p: f32, curve: f32, min: f32) -> f32 {
    let min = min.clamp(0.0, 1.0);
    min + (1.0 - min) * p.clamp(0.0, 1.0).powf(curve.max(0.05))
}

/// Pen mode loosens clicks for a pen hand: a tap may drift further and a
/// double tap may take longer. Set every frame; cheap when unchanged.
pub(crate) fn apply_click_options(ctx: &egui::Context, pen_mode: bool) {
    let (dist, double) = if pen_mode { (14.0, 0.45) } else { (6.0, 0.3) };
    let differs = ctx.options(|o| {
        o.input_options.max_click_dist != dist || o.input_options.max_double_click_delay != double
    });
    if differs {
        ctx.options_mut(|o| {
            o.input_options.max_click_dist = dist;
            o.input_options.max_double_click_delay = double;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_follows_the_pen_and_lets_go_on_lift() {
        let prefs = Prefs::default();
        let mut pen = Pen::default();
        assert_eq!(pen.brush(&prefs), (1.0, 1.0), "a mouse is full strength");
        pen.take(TouchPhase::Start, Some(0.4));
        let p = pen.pressure(&prefs).unwrap();
        assert!((p - shape(0.4, 1.0, prefs.pen_min())).abs() < 1e-6);
        // a jump is smoothed halfway
        pen.take(TouchPhase::Move, Some(0.8));
        let p = pen.pressure(&prefs).unwrap();
        assert!((p - shape(0.6, 1.0, prefs.pen_min())).abs() < 1e-6);
        // strength by default, size only when asked
        assert_eq!(pen.brush(&prefs).0, 1.0);
        assert!(pen.brush(&prefs).1 < 1.0);
        pen.take(TouchPhase::End, None);
        assert_eq!(pen.pressure(&prefs), None);
        assert!(pen.last.is_some(), "the meter keeps the last reading");
        // touches without a force (a mouse emulating one) change nothing
        pen.take(TouchPhase::Start, None);
        assert_eq!(pen.pressure(&prefs), None);
    }

    #[test]
    fn the_curve_and_the_lightest_touch_shape_pressure() {
        assert!((shape(0.5, 1.0, 0.0) - 0.5).abs() < 1e-6);
        assert!(shape(0.5, 2.0, 0.0) < 0.5, "a firm curve wants more press");
        assert!(shape(0.5, 0.5, 0.0) > 0.5, "a light curve wants less");
        assert!((shape(0.0, 1.0, 0.2) - 0.2).abs() < 1e-6);
        assert!((shape(1.0, 3.0, 0.2) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pen_events_reach_the_reader_through_egui() {
        let ctx = egui::Context::default();
        let mut pen = Pen::default();
        let touch = |phase, force| Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase,
            pos: egui::pos2(10.0, 10.0),
            force,
        };
        let input = egui::RawInput {
            events: vec![touch(TouchPhase::Start, Some(0.7))],
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| pen.read(ui.ctx()));
        out.textures_delta.clear();
        assert!(pen.pressure(&Prefs::default()).is_some());
    }
}
