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
    /// Where this node sits in the untranslated rest chain; X/Y outputs
    /// read displacement from here, which settles at the anchor shift.
    rest_abs: V2,
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
    /// Index of this output in the rig group, for the dialog's tables.
    rig_output: usize,
    vertex: usize,
    scale: f32,
    weight: f32,
    kind: PhysicsIo,
    sign: f32,
    /// The latest computed value (scaled, signed), re-applied on frames
    /// where no fixed step ticks.
    last: f32,
    /// Peak |excursion| as a percentage of the parameter's half range,
    /// the dialog's Output Maximum (%).
    peak: f32,
}

struct GroupSim {
    /// Index of this group in the model's rig; some rig groups are skipped
    /// at build time, so the two sequences differ.
    rig_index: usize,
    /// The dialog's per-group Valid checkbox; off skips the whole group.
    enabled: bool,
    inputs: Vec<InputSim>,
    outputs: Vec<OutputSim>,
    nodes: Vec<NodeSim>,
}

pub struct Physics {
    groups: Vec<GroupSim>,
    wind: V2,
    /// Fixed step in seconds and the accumulated render time.
    step: f32,
    acc: f32,
    /// Outputs exist to apply; false until the first full step.
    primed: bool,
}

/// Map a parameter value into the group's normalized range. Per the manual,
/// the pivot is the midpoint of the parameter's definition, not its default:
/// "the value halfway between the smallest and largest parameter definition
/// is the center of normalization".
fn normalize(value: f32, min: f32, max: f32, norm: PhysicsRange) -> f32 {
    let mid = (min + max) * 0.5;
    if value >= mid {
        let span = max - mid;
        let t = if span.abs() < 1e-6 { 0.0 } else { (value - mid) / span };
        norm.default + t.clamp(0.0, 1.0) * (norm.max - norm.default)
    } else {
        let span = mid - min;
        let t = if span.abs() < 1e-6 { 0.0 } else { (mid - value) / span };
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
                    rest_abs: at,
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
                    .enumerate()
                    .filter_map(|(rig_output, o)| {
                        let vertex = (o.vertex as usize).clamp(1, node_count - 1);
                        Some(OutputSim {
                            param: find(&o.param)?,
                            rig_output,
                            vertex,
                            scale: o.scale,
                            weight: o.weight / 100.0,
                            kind: o.kind,
                            sign: if o.reflect { -1.0 } else { 1.0 },
                            last: 0.0,
                            peak: 0.0,
                        })
                    })
                    .collect(),
                nodes,
                enabled: true,
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
            primed: false,
        })
    }

    /// The live node chain of one rig group, anchor first, for the dialog's
    /// pendulum preview. None when that group never built (under two nodes).
    pub fn group_nodes(&self, rig_group: usize) -> Option<Vec<(f32, f32)>> {
        let g = self.groups.iter().find(|g| g.rig_index == rig_group)?;
        Some(g.nodes.iter().map(|n| (n.pos.x, n.pos.y)).collect())
    }

    /// The dialog's per-group Valid checkbox: a disabled group neither
    /// simulates nor writes its outputs. Resets to enabled on rebuild.
    pub fn set_group_enabled(&mut self, rig_group: usize, on: bool) {
        if let Some(g) = self.groups.iter_mut().find(|g| g.rig_index == rig_group) {
            g.enabled = on;
        }
    }

    pub fn group_enabled(&self, rig_group: usize) -> bool {
        self.groups.iter().find(|g| g.rig_index == rig_group).is_none_or(|g| g.enabled)
    }

    /// Each output's Output Maximum (%) so far, keyed by its index in the
    /// rig group's output list.
    pub fn output_peaks(&self, rig_group: usize) -> Option<Vec<(usize, f32)>> {
        let g = self.groups.iter().find(|g| g.rig_index == rig_group)?;
        Some(g.outputs.iter().map(|o| (o.rig_output, o.peak)).collect())
    }

    pub fn reset_output_peaks(&mut self) {
        for g in &mut self.groups {
            for o in &mut g.outputs {
                o.peak = 0.0;
            }
        }
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
        let mut steps = 0;
        while self.acc >= self.step && steps < 8 {
            self.acc -= self.step;
            steps += 1;
            for g in &mut self.groups {
                if !g.enabled {
                    continue;
                }
                // fold the inputs into an anchor translation and a tilt
                let (mut tx, mut ty, mut ta) = (0.0f32, 0.0f32, 0.0f32);
                for i in &g.inputs {
                    let (min, max, default) = meta_of(i.param);
                    // rigs feed outputs back in as inputs on purpose (soft
                    // follow); with radian outputs the loop gain stays under
                    // one, so reading the live value decays instead of
                    // blowing up
                    let v = normalize(
                        values.get(i.param).copied().unwrap_or(default),
                        min,
                        max,
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
                    // a real constrained pendulum: gravity pulls along the
                    // rotated hang direction, mobility keeps momentum, delay
                    // stretches the node's personal clock, and the distance
                    // constraint does the rest. The gravity constant makes a
                    // ten-unit strand swing with a half-second-ish period,
                    // which is the hair feel the reference has.
                    let vel = V2 {
                        x: (n.pos.x - n.prev.x) * n.mobility,
                        y: (n.pos.y - n.prev.y) * n.mobility,
                    };
                    let dtq = self.step * n.delay.clamp(0.01, 10.0);
                    let grav = 1500.0 * n.acceleration;
                    n.prev = n.pos;
                    let mut next = V2 {
                        x: n.pos.x + vel.x + hang.x * grav * dtq * dtq + self.wind.x * dtq,
                        y: n.pos.y + vel.y + hang.y * grav * dtq * dtq + self.wind.y * dtq,
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
        // computed values refresh on stepped frames; the application runs
        // every frame regardless, or the raw app-written inputs leak through
        // for one frame and the view flickers between the two
        if steps > 0 {
            self.primed = true;
            for g in &mut self.groups {
                if !g.enabled {
                    continue;
                }
                for oi in 0..g.outputs.len() {
                    let Some(o) = g.outputs.get(oi) else { break };
                    let (node, parent) = match (g.nodes.get(o.vertex), g.nodes.get(o.vertex - 1)) {
                        (Some(n), Some(p)) => (n, p),
                        _ => continue,
                    };
                    let value = match o.kind {
                        PhysicsIo::Angle => {
                            // the manual: "the angle of the pendulum
                            // (relative to the parent pendulum) ... in
                            // radians", so each stage reads its bend
                            // against the stage above, and stage one reads
                            // against the suspension
                            let dx = node.pos.x - parent.pos.x;
                            let dy = node.pos.y - parent.pos.y;
                            let cur = dy.atan2(dx);
                            let rest = node.rest_dir.y.atan2(node.rest_dir.x);
                            let reference = if o.vertex <= 1 {
                                rest
                            } else {
                                let above = g
                                    .nodes
                                    .get(o.vertex - 2)
                                    .map(|a| {
                                        let px = parent.pos.x - a.pos.x;
                                        let py = parent.pos.y - a.pos.y;
                                        py.atan2(px)
                                    })
                                    .unwrap_or(rest);
                                let parent_rest = parent.rest_dir.y.atan2(parent.rest_dir.x);
                                above + (rest - parent_rest)
                            };
                            let mut diff = cur - reference;
                            while diff > core::f32::consts::PI {
                                diff -= core::f32::consts::TAU;
                            }
                            while diff < -core::f32::consts::PI {
                                diff += core::f32::consts::TAU;
                            }
                            diff
                        }
                        PhysicsIo::X => node.pos.x - node.rest_abs.x,
                        PhysicsIo::Y => node.pos.y - node.rest_abs.y,
                    };
                    if let Some(o) = g.outputs.get_mut(oi) {
                        o.last = value * o.scale * o.sign;
                    }
                }
            }
        }
        if !self.primed {
            return false;
        }
        // per the manual, each output blends into the current parameter
        // value by its influence (overwrite at 100%), group by group in
        // order, so later groups see what earlier ones wrote
        let mut any = false;
        for g in &mut self.groups {
            if !g.enabled {
                continue;
            }
            for o in &mut g.outputs {
                let Some(slot) = values.get_mut(o.param) else { continue };
                let (min, max, default) = meta_of(o.param);
                let target = (default + o.last).clamp(min, max);
                *slot += (target - *slot) * o.weight;
                let half = if o.last >= 0.0 { max - default } else { default - min };
                if half > 1e-6 {
                    o.peak = o.peak.max(o.last.abs() / half * 100.0);
                }
                any = true;
            }
        }
        any
    }
}
