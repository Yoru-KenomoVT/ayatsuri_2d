//! The pendulum solver behind physics3-style rigs.
//!
//! This is not a physics engine (docs/physics.md): each group is a strand of
//! nodes hanging off an anchor the inputs move. The integrator is our own,
//! built to honour the published field semantics: mobility is how much
//! momentum a node keeps, delay scales reaction speed, acceleration scales
//! the restoring force, and the rest offsets come from the authored node
//! positions. Everything is preallocated at load; stepping never allocates.

use ayatsuri_format::model::{Model, PhysicsIo, PhysicsRange};

#[derive(Clone, Copy, Default)]
struct V2 {
    x: f32,
    y: f32,
}

struct NodeSim {
    pos: V2,
    prev: V2,
    /// Unit direction from the parent at rest, and the distance kept to it.
    rest_dir: V2,
    len: f32,
    mobility: f32,
    delay: f32,
    acceleration: f32,
}

struct InputSim {
    param: usize,
    weight: f32,
    kind: PhysicsIo,
    sign: f32,
    norm: PhysicsRange,
}

struct OutputSim {
    param: usize,
    vertex: usize,
    scale: f32,
    weight: f32,
    kind: PhysicsIo,
    sign: f32,
}

struct GroupSim {
    /// Index of this group in the model's rig; some rig groups are skipped
    /// at build time, so the two sequences differ.
    rig_index: usize,
    inputs: Vec<InputSim>,
    outputs: Vec<OutputSim>,
    nodes: Vec<NodeSim>,
    /// The tilt applied on the latest step; outputs measure swing against
    /// the rest pose rotated by this, so a held tilt settles back to zero.
    last_rad: f32,
}

pub struct Physics {
    groups: Vec<GroupSim>,
    wind: V2,
    /// Fixed step in seconds, accumulated render time, parameter deltas.
    step: f32,
    acc: f32,
    deltas: Vec<f32>,
    touched: Vec<bool>,
    /// What each output parameter was last set to by physics, NaN if never.
    /// Rigs routinely feed an output back in as an input (soft follow); the
    /// input has to see the app's intent, not our own write, or the loop is
    /// unstable. When a param still holds exactly what we wrote, the cached
    /// app value is used instead.
    written: Vec<f32>,
    cache: Vec<f32>,
}

/// Map a parameter value into the group's normalized range, piecewise around
/// the defaults the way the published spec describes normalization.
fn normalize(value: f32, min: f32, max: f32, default: f32, norm: PhysicsRange) -> f32 {
    if value >= default {
        let span = max - default;
        let t = if span.abs() < 1e-6 { 0.0 } else { (value - default) / span };
        norm.default + t.clamp(0.0, 1.0) * (norm.max - norm.default)
    } else {
        let span = default - min;
        let t = if span.abs() < 1e-6 { 0.0 } else { (default - value) / span };
        norm.default + t.clamp(0.0, 1.0) * (norm.min - norm.default)
    }
}

impl Physics {
    /// Build the sim from the model's rig, resolving parameter ids. Entries
    /// naming unknown parameters are dropped rather than failing the load.
    pub fn new(model: &Model) -> Option<Self> {
        let rig = model.physics.as_ref()?;
        let find = |id: &str| model.parameters.iter().position(|p| p.id == id);
        let mut groups = Vec::with_capacity(rig.groups.len());
        for (rig_index, g) in rig.groups.iter().enumerate() {
            if g.nodes.len() < 2 {
                continue;
            }
            let mut nodes = Vec::with_capacity(g.nodes.len());
            let mut at = V2::default();
            for (i, n) in g.nodes.iter().enumerate() {
                let (dx, dy) = if i == 0 { (0.0, 0.0) } else { (n.x, n.y) };
                let len = (dx * dx + dy * dy).sqrt();
                let rest_dir = if len > 1e-6 {
                    V2 { x: dx / len, y: dy / len }
                } else if i == 0 {
                    V2 { x: 0.0, y: 0.0 }
                } else {
                    V2 { x: 0.0, y: 1.0 }
                };
                at = V2 { x: at.x + rest_dir.x * len, y: at.y + rest_dir.y * len };
                nodes.push(NodeSim {
                    pos: at,
                    prev: at,
                    rest_dir,
                    len: if i == 0 { 0.0 } else { len.max(1e-3) },
                    mobility: n.mobility,
                    delay: n.delay,
                    acceleration: n.acceleration,
                });
            }
            let node_count = nodes.len();
            groups.push(GroupSim {
                rig_index,
                inputs: g
                    .inputs
                    .iter()
                    .filter_map(|i| {
                        Some(InputSim {
                            param: find(&i.param)?,
                            weight: i.weight / 100.0,
                            kind: i.kind,
                            sign: if i.reflect { -1.0 } else { 1.0 },
                            norm: match i.kind {
                                PhysicsIo::Angle => g.norm_angle,
                                _ => g.norm_position,
                            },
                        })
                    })
                    .collect(),
                outputs: g
                    .outputs
                    .iter()
                    .filter_map(|o| {
                        let vertex = (o.vertex as usize).clamp(1, node_count - 1);
                        Some(OutputSim {
                            param: find(&o.param)?,
                            vertex,
                            scale: o.scale,
                            weight: o.weight / 100.0,
                            kind: o.kind,
                            sign: if o.reflect { -1.0 } else { 1.0 },
                        })
                    })
                    .collect(),
                nodes,
                last_rad: 0.0,
            });
        }
        if groups.is_empty() {
            return None;
        }
        let fps = if rig.fps > 1.0 { rig.fps } else { 60.0 };
        Some(Self {
            groups,
            wind: V2 { x: rig.wind_x, y: rig.wind_y },
            step: 1.0 / fps,
            acc: 0.0,
            deltas: vec![0.0; model.parameters.len()],
            touched: vec![false; model.parameters.len()],
            written: vec![f32::NAN; model.parameters.len()],
            cache: model.parameters.iter().map(|p| p.default).collect(),
        })
    }

    /// The live node chain of one rig group, anchor first, for the dialog's
    /// pendulum preview. None when that group never built (under two nodes).
    pub fn group_nodes(&self, rig_group: usize) -> Option<Vec<(f32, f32)>> {
        let g = self.groups.iter().find(|g| g.rig_index == rig_group)?;
        Some(g.nodes.iter().map(|n| (n.pos.x, n.pos.y)).collect())
    }

    /// Live wind override from the physics dialog.
    pub fn set_wind(&mut self, x: f32, y: f32) {
        self.wind = V2 { x, y };
    }

    pub fn wind(&self) -> (f32, f32) {
        (self.wind.x, self.wind.y)
    }

    /// Put every strand back at rest.
    pub fn reset(&mut self) {
        for g in &mut self.groups {
            let mut at = V2::default();
            for n in &mut g.nodes {
                at = V2 { x: at.x + n.rest_dir.x * n.len, y: at.y + n.rest_dir.y * n.len };
                n.pos = at;
                n.prev = at;
            }
        }
        self.acc = 0.0;
    }

    /// Advance by `dt` seconds against the current parameter values and
    /// write the output deltas back into them. Returns true when any
    /// parameter moved.
    pub fn step(
        &mut self,
        dt: f32,
        values: &mut [f32],
        meta_of: impl Fn(usize) -> (f32, f32, f32),
    ) -> bool {
        self.acc += dt.clamp(0.0, 0.25);
        // settle which value each input actually sees this frame
        for (p, slot) in self.cache.iter_mut().enumerate() {
            let cur = values.get(p).copied().unwrap_or(0.0);
            let ours = self.written.get(p).copied().unwrap_or(f32::NAN);
            if !(ours.is_finite() && (cur - ours).abs() < 1e-5) {
                *slot = cur;
            }
        }
        let mut steps = 0;
        while self.acc >= self.step && steps < 8 {
            self.acc -= self.step;
            steps += 1;
            for g in &mut self.groups {
                // fold the inputs into an anchor translation and a tilt
                let (mut tx, mut ty, mut ta) = (0.0f32, 0.0f32, 0.0f32);
                for i in &g.inputs {
                    let (min, max, default) = meta_of(i.param);
                    let v = normalize(
                        self.cache.get(i.param).copied().unwrap_or(default),
                        min,
                        max,
                        default,
                        i.norm,
                    ) * i.sign
                        * i.weight;
                    match i.kind {
                        PhysicsIo::X => tx += v,
                        PhysicsIo::Y => ty += v,
                        PhysicsIo::Angle => ta += v,
                    }
                }
                let rad = ta.to_radians();
                g.last_rad = rad;
                let (sin, cos) = rad.sin_cos();
                if let Some(anchor) = g.nodes.first_mut() {
                    anchor.prev = anchor.pos;
                    anchor.pos = V2 { x: tx, y: ty };
                }
                for i in 1..g.nodes.len() {
                    let parent = g.nodes.get(i - 1).map(|n| n.pos).unwrap_or_default();
                    let Some(n) = g.nodes.get_mut(i) else { break };
                    // the tilt swings the direction the strand settles toward
                    let hang = V2 {
                        x: n.rest_dir.x * cos - n.rest_dir.y * sin,
                        y: n.rest_dir.x * sin + n.rest_dir.y * cos,
                    };
                    let target = V2 { x: parent.x + hang.x * n.len, y: parent.y + hang.y * n.len };
                    // keep momentum by mobility, pull to the target by delay
                    // and acceleration, then re-fix the distance. The pull is
                    // deliberately strong: the reference feel is a tight
                    // follow with a short soft lag, not a free pendulum.
                    let vel = V2 {
                        x: (n.pos.x - n.prev.x)
                            * n.mobility
                            * (1.0 - n.delay * 0.05).clamp(0.5, 1.0),
                        y: (n.pos.y - n.prev.y)
                            * n.mobility
                            * (1.0 - n.delay * 0.05).clamp(0.5, 1.0),
                    };
                    let k = (n.delay * n.acceleration * self.step * 30.0 * 0.55).clamp(0.0, 1.0);
                    n.prev = n.pos;
                    let mut next = V2 {
                        x: n.pos.x + vel.x + (target.x - n.pos.x) * k + self.wind.x * self.step,
                        y: n.pos.y + vel.y + (target.y - n.pos.y) * k + self.wind.y * self.step,
                    };
                    let dx = next.x - parent.x;
                    let dy = next.y - parent.y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist > 1e-6 {
                        next =
                            V2 { x: parent.x + dx / dist * n.len, y: parent.y + dy / dist * n.len };
                    } else {
                        next = target;
                    }
                    n.pos = next;
                }
            }
        }
        if steps == 0 {
            return false;
        }

        // outputs: each driven parameter collects its weighted deltas, then
        // lands at default + delta, clamped by set_value downstream
        self.deltas.iter_mut().for_each(|d| *d = 0.0);
        self.touched.iter_mut().for_each(|t| *t = false);
        for g in &self.groups {
            for o in &g.outputs {
                let (node, parent) = match (g.nodes.get(o.vertex), g.nodes.get(o.vertex - 1)) {
                    (Some(n), Some(p)) => (n, p),
                    _ => continue,
                };
                let (sin, cos) = g.last_rad.sin_cos();
                let hang_x = node.rest_dir.x * cos - node.rest_dir.y * sin;
                let hang_y = node.rest_dir.x * sin + node.rest_dir.y * cos;
                let value = match o.kind {
                    PhysicsIo::Angle => {
                        let dx = node.pos.x - parent.pos.x;
                        let dy = node.pos.y - parent.pos.y;
                        let cur = dy.atan2(dx);
                        let rest = hang_y.atan2(hang_x);
                        let mut diff = cur - rest;
                        while diff > core::f32::consts::PI {
                            diff -= core::f32::consts::TAU;
                        }
                        while diff < -core::f32::consts::PI {
                            diff += core::f32::consts::TAU;
                        }
                        diff.to_degrees()
                    }
                    PhysicsIo::X => node.pos.x - parent.pos.x - hang_x * node.len,
                    PhysicsIo::Y => node.pos.y - parent.pos.y - hang_y * node.len,
                };
                if let (Some(d), Some(t)) =
                    (self.deltas.get_mut(o.param), self.touched.get_mut(o.param))
                {
                    *d += value * o.scale * o.sign * o.weight;
                    *t = true;
                }
            }
        }
        let mut any = false;
        for (param, (&delta, &touched)) in self.deltas.iter().zip(self.touched.iter()).enumerate() {
            if touched && let Some(slot) = values.get_mut(param) {
                let (min, max, default) = meta_of(param);
                *slot = (default + delta).clamp(min, max);
                if let Some(w) = self.written.get_mut(param) {
                    *w = *slot;
                }
                any = true;
            }
        }
        any
    }
}
