//! Breath: a few parameters swing on slow sine waves so the model idles in
//! the preview and physics has something to react to, as the Cubism SDK's
//! CubismBreath does. Each frame a row adds
//! `(offset + peak * sin(2 * pi * t / cycle)) * weight` to its parameter.
//! Only live values move; keyforms and exports never see it.

use ayatsuri_core::Puppet;
use serde::{Deserialize, Serialize};

/// One parameter's breathing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct BreathRow {
    pub param: String,
    pub offset: f32,
    pub peak: f32,
    /// Seconds per swing.
    pub cycle: f32,
    pub weight: f32,
}

impl Default for BreathRow {
    fn default() -> Self {
        Self { param: String::new(), offset: 0.0, peak: 1.0, cycle: 4.0, weight: 1.0 }
    }
}

/// The SDK sample model's breathing.
pub fn sdk_rows() -> Vec<BreathRow> {
    [
        ("ParamAngleX", 0.0, 15.0, 6.5345, 0.5),
        ("ParamAngleY", 0.0, 8.0, 3.5345, 0.5),
        ("ParamAngleZ", 0.0, 10.0, 5.5345, 0.5),
        ("ParamBodyAngleX", 0.0, 4.0, 15.5345, 0.5),
        ("ParamBreath", 0.5, 0.5, 3.2345, 1.0),
    ]
    .into_iter()
    .map(|(param, offset, peak, cycle, weight)| BreathRow {
        param: param.into(),
        offset,
        peak,
        cycle,
        weight,
    })
    .collect()
}

/// The running breath: its clock, and what it added to each parameter last
/// frame, taken back before the next is added so the base value stays put.
#[derive(Default)]
pub struct Breath {
    time: f32,
    applied: Vec<(usize, f32)>,
}

impl Breath {
    /// Advance by `dt` seconds and move the parameters. Returns whether any
    /// parameter is breathing.
    pub fn step(&mut self, puppet: &mut Puppet, rows: &[BreathRow], dt: f32) -> bool {
        self.time += dt.max(0.0);
        self.release(puppet);
        let tau = std::f32::consts::TAU;
        for r in rows {
            let Some((pi, p)) =
                puppet.model().parameters.iter().enumerate().find(|(_, p)| p.id == r.param)
            else {
                continue;
            };
            let (lo, hi) = (p.min.min(p.max), p.max.max(p.min));
            let swing = if r.cycle > 0.0 { (tau * self.time / r.cycle).sin() } else { 0.0 };
            let add = (r.offset + r.peak * swing) * r.weight;
            let before = puppet.value(pi);
            let after = (before + add).clamp(lo, hi);
            puppet.set_value(pi, after);
            // what actually landed, so the clamp is taken back exactly
            self.applied.push((pi, after - before));
        }
        !self.applied.is_empty()
    }

    /// Take back what the last frame added, leaving the parameters where
    /// they were before breathing.
    pub fn release(&mut self, puppet: &mut Puppet) {
        for (pi, add) in self.applied.drain(..).rev() {
            puppet.set_value(pi, puppet.value(pi) - add);
        }
    }

    /// Stop: parameters back, clock back to zero.
    pub fn stop(&mut self, puppet: &mut Puppet) {
        self.release(puppet);
        self.time = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::Model;
    use ayatsuri_format::model::Parameter;

    fn puppet() -> Puppet {
        let mut m = Model::new();
        m.parameters.push(Parameter {
            id: "ParamBreath".into(),
            min: 0.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
            blend_shape: false,
        });
        Puppet::new(m)
    }

    #[test]
    fn breath_follows_the_sdk_formula_and_lets_go_cleanly() {
        let mut p = puppet();
        let rows = vec![BreathRow {
            param: "ParamBreath".into(),
            offset: 0.5,
            peak: 0.5,
            cycle: 4.0,
            weight: 1.0,
        }];
        let mut b = Breath::default();
        // a quarter cycle in: offset + peak
        assert!(b.step(&mut p, &rows, 1.0));
        assert!((p.value(0) - 1.0).abs() < 1e-5);
        // three quarters: offset - peak
        b.step(&mut p, &rows, 2.0);
        assert!(p.value(0).abs() < 1e-5);
        b.stop(&mut p);
        assert!(p.value(0).abs() < 1e-6);
    }

    #[test]
    fn a_value_clamped_at_the_end_still_comes_back_where_it_was() {
        let mut p = puppet();
        p.set_value(0, 0.9);
        let rows =
            vec![BreathRow { param: "ParamBreath".into(), offset: 0.5, ..Default::default() }];
        let mut b = Breath::default();
        b.step(&mut p, &rows, 0.0);
        assert!((p.value(0) - 1.0).abs() < 1e-6);
        b.stop(&mut p);
        assert!((p.value(0) - 0.9).abs() < 1e-6);
    }
}
