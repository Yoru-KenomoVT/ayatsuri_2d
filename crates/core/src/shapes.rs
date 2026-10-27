//! Blend shapes on any object (see `ayatsuri_format::model::BlendShape`):
//! the editing side. An object's keyforms are numbered normal ones first,
//! then its shapes' keyforms, shape by shape in list order, key by key, so
//! a tool that edits "keyform k" edits a shape keyform when the parameters
//! sit on one (see `keyform_at`).

use ayatsuri_format::model::{Binding, BlendShape, DeformerKind, Model};

use crate::blend;
use crate::blendshape::{self, Channel};
use crate::edit::Target;

/// The object's blend shapes.
pub fn shapes(model: &Model, target: Target) -> Option<&Vec<BlendShape>> {
    Some(match target {
        Target::ArtMesh(i) => &model.art_meshes.get(i)?.blend_shapes,
        Target::Deformer(i) => &model.deformers.get(i)?.blend_shapes,
        Target::Part(i) => &model.parts.get(i)?.blend_shapes,
    })
}

pub fn shapes_mut(model: &mut Model, target: Target) -> Option<&mut Vec<BlendShape>> {
    Some(match target {
        Target::ArtMesh(i) => &mut model.art_meshes.get_mut(i)?.blend_shapes,
        Target::Deformer(i) => &mut model.deformers.get_mut(i)?.blend_shapes,
        Target::Part(i) => &mut model.parts.get_mut(i)?.blend_shapes,
    })
}

fn binding(model: &Model, target: Target) -> Option<&Binding> {
    Some(match target {
        Target::ArtMesh(i) => &model.art_meshes.get(i)?.binding,
        Target::Deformer(i) => &model.deformers.get(i)?.binding,
        Target::Part(i) => &model.parts.get(i)?.binding,
    })
}

/// The object's normal pool for `channel` and its floats per keyform, when
/// its kind has that channel (an empty colour pool is still one: neutral).
pub fn normal_pool(model: &Model, target: Target, channel: Channel) -> Option<(&[f32], usize)> {
    use Channel::*;
    match target {
        Target::ArtMesh(i) => {
            let m = model.art_meshes.get(i)?;
            match channel {
                Positions => Some((&m.positions, m.vertex_count as usize * 2)),
                Opacity => Some((&m.opacities, 1)),
                Multiply => Some((&m.multiply, 3)),
                Screen => Some((&m.screen, 3)),
                _ => None,
            }
        }
        Target::Deformer(i) => {
            let d = model.deformers.get(i)?;
            match (&d.kind, channel) {
                (_, Multiply) => Some((&d.multiply, 3)),
                (_, Screen) => Some((&d.screen, 3)),
                (DeformerKind::Warp(w), Positions) => Some((&w.positions, w.grid_points() * 2)),
                (DeformerKind::Warp(w), Opacity) => Some((&w.opacities, 1)),
                (DeformerKind::Rotation(r), Opacity) => Some((&r.opacities, 1)),
                (DeformerKind::Rotation(r), Angle) => Some((&r.angles, 1)),
                (DeformerKind::Rotation(r), OriginX) => Some((&r.origins_x, 1)),
                (DeformerKind::Rotation(r), OriginY) => Some((&r.origins_y, 1)),
                (DeformerKind::Rotation(r), Scale) => Some((&r.scales, 1)),
                _ => None,
            }
        }
        Target::Part(i) => {
            let p = model.parts.get(i)?;
            match channel {
                DrawOrder => Some((&p.draw_orders, 1)),
                Opacity => Some((&p.offscreen.as_ref()?.opacities, 1)),
                _ => None,
            }
        }
    }
}

/// The channels a new shape starts with on this kind of object; opacity
/// and colour join the first time they are edited.
fn main_channels(model: &Model, target: Target) -> &'static [Channel] {
    match target {
        Target::ArtMesh(_) => &[Channel::Positions],
        Target::Deformer(i) => match model.deformers.get(i).map(|d| &d.kind) {
            Some(DeformerKind::Rotation(_)) => {
                &[Channel::Angle, Channel::OriginX, Channel::OriginY, Channel::Scale]
            }
            _ => &[Channel::Positions],
        },
        Target::Part(_) => &[Channel::DrawOrder],
    }
}

/// How many normal keyforms the object has.
pub fn normal_count(model: &Model, target: Target) -> usize {
    binding(model, target).map_or(0, |b| b.keyform_count())
}

/// The object's blend of `channel` at every parameter's default: the base
/// its shapes are differences from. None when its binding does not cover
/// the defaults. A colour the object does not have is neutral.
pub fn base(model: &Model, target: Target, channel: Channel) -> Option<Vec<f32>> {
    let (pool, n) = normal_pool(model, target, channel)?;
    if pool.is_empty() {
        // a colour the object does not have: neutral, if the defaults are keyed
        let neutral = if channel == Channel::Multiply { 1.0 } else { 0.0 };
        return default_blend(model, binding(model, target)?, &[], 0).map(|_| vec![neutral; n]);
    }
    default_blend(model, binding(model, target)?, pool, n)
}

/// A pool's blend (`n` floats per keyform) at every parameter's default,
/// for anything with a binding. None when the binding does not cover the
/// defaults.
pub fn default_blend(model: &Model, b: &Binding, pool: &[f32], n: usize) -> Option<Vec<f32>> {
    let mut corners = Vec::new();
    let at_default = blend::corners(
        b,
        |p| model.parameters.get(p).map_or(0.0, |q| q.default),
        |p| {
            let snap = 0.1f32.powi(model.parameters.get(p).map_or(3, |q| q.decimals).clamp(0, 10));
            (snap, snap * 1.5)
        },
        &mut corners,
    );
    if !at_default {
        return None;
    }
    let mut out = vec![0.0; n];
    for &(k, w) in &corners {
        let start = k.saturating_mul(n);
        let Some(src) = pool.get(start..start.saturating_add(n)) else { continue };
        for (o, s) in out.iter_mut().zip(src) {
            *o += s * w;
        }
    }
    Some(out)
}

/// Shape and key of extended keyform `k`, when it is a shape keyform.
pub fn locate(model: &Model, target: Target, k: usize) -> Option<(usize, usize)> {
    let mut rest = k.checked_sub(normal_count(model, target))?;
    for (si, s) in shapes(model, target)?.iter().enumerate() {
        if rest < s.keys.len() {
            return Some((si, rest));
        }
        rest -= s.keys.len();
    }
    None
}

/// The extended keyform the values sit on through blend shapes: exactly
/// one blend shape parameter off its base key and on one of its keys
/// (Cubism edits one blend shape keyform per object at a time).
/// `Some(None)` when every one is at its base key, so the normal keyforms
/// are the ones to edit; None when nothing can be edited, or the shape's
/// weight limit holds it under 100% there.
pub fn keyform_at(model: &Model, target: Target, values: &[f32]) -> Option<Option<usize>> {
    let Some(list) = shapes(model, target) else { return Some(None) };
    let mut found = None;
    let mut first = normal_count(model, target);
    for s in list {
        let p = model.parameters.get(s.parameter as usize)?;
        let v = values.get(s.parameter as usize).copied().unwrap_or(p.default);
        let tol = ((p.max - p.min).abs() * 0.01).max(1e-4);
        if v.abs() > tol {
            let i = s.keys.iter().position(|k| (k - v).abs() <= tol)?;
            if found.is_some()
                || blendshape::limit(&model.parameters, &s.limits, values) < 1.0 - 1e-6
            {
                return None;
            }
            found = Some(first + i);
        }
        first += s.keys.len();
    }
    Some(found)
}

/// One channel of a shape keyform, editable. A channel the shape does not
/// have yet is filled first with the base at every key, so it adds nothing
/// until edited.
pub fn channel_mut(
    model: &mut Model,
    target: Target,
    k: usize,
    channel: Channel,
) -> Option<&mut [f32]> {
    let (si, ki) = locate(model, target, k)?;
    let (_, n) = normal_pool(model, target, channel)?;
    let needs_fill =
        shapes(model, target)?.get(si).is_some_and(|s| channel.of(s).len() != s.keys.len() * n);
    let fill = if needs_fill { Some(base(model, target, channel)?) } else { None };
    let shape = shapes_mut(model, target)?.get_mut(si)?;
    let keys = shape.keys.len();
    let pool = channel.of_mut(shape);
    if let Some(b) = fill {
        pool.clear();
        for _ in 0..keys {
            pool.extend_from_slice(&b);
        }
    }
    let start = ki.checked_mul(n)?;
    pool.get_mut(start..start.checked_add(n)?)
}

/// Blend shape keys: every value off the base key gains a keyform on the
/// object's blend shape for `param` (made when it has none). Each channel
/// gets the form the object shows there now, so nothing moves.
pub fn add_keys(model: &mut Model, target: Target, param: usize, values: &[f32]) -> bool {
    let fresh =
        shapes(model, target).is_some_and(|l| !l.iter().any(|s| s.parameter as usize == param));
    // the channels to carry: the shape's own, or a new shape's main ones
    let channels: Vec<Channel> = match shapes(model, target)
        .and_then(|l| l.iter().find(|s| s.parameter as usize == param))
    {
        Some(s) => ALL.iter().copied().filter(|c| !c.of(s).is_empty()).collect(),
        None => main_channels(model, target).to_vec(),
    };
    let mut bases = Vec::with_capacity(channels.len());
    for &c in &channels {
        let Some(b) = base(model, target, c) else { return false };
        bases.push(b);
    }
    let Some(list) = shapes_mut(model, target) else { return false };
    if fresh {
        list.push(BlendShape { parameter: param as u32, ..Default::default() });
    }
    let Some(at) = list.iter().position(|s| s.parameter as usize == param) else { return false };
    let Some(shape) = list.get_mut(at) else { return false };
    let mut changed = false;
    for &v in values {
        if v.abs() < 1e-5 || shape.keys.iter().any(|k| (k - v).abs() < 1e-5) {
            continue;
        }
        let i = shape.keys.partition_point(|&k| k < v);
        for (&c, b) in channels.iter().zip(&bases) {
            // the base plus whatever the shape already adds at `v`
            let n = b.len();
            let mut form = b.clone();
            let pool = c.of(shape);
            for (k, w) in blendshape::weights(v, &shape.keys) {
                let start = k.saturating_mul(n);
                if w > 0.0
                    && let Some(f) = pool.get(start..start.saturating_add(n))
                {
                    blendshape::add(&mut form, f, b, w);
                }
            }
            let pool = c.of_mut(shape);
            let start = i.saturating_mul(n).min(pool.len());
            pool.splice(start..start, form);
        }
        shape.keys.insert(i, v);
        changed = true;
    }
    if shape.keys.is_empty() {
        list.remove(at);
    }
    changed
}

/// Remove the blend shape key nearest `value`; the base key cannot go. The
/// last key going takes the blend shape with it.
pub fn delete_key(model: &mut Model, target: Target, param: usize, value: f32) -> bool {
    let sizes: Vec<(Channel, usize)> =
        ALL.iter().filter_map(|&c| Some((c, normal_pool(model, target, c)?.1))).collect();
    let Some(list) = shapes_mut(model, target) else { return false };
    let Some(at) = list.iter().position(|s| s.parameter as usize == param) else { return false };
    let Some(shape) = list.get_mut(at) else { return false };
    let Some((i, k)) = shape
        .keys
        .iter()
        .copied()
        .enumerate()
        .min_by(|(_, a), (_, b)| (a - value).abs().total_cmp(&(b - value).abs()))
    else {
        return false;
    };
    if value.abs() < (k - value).abs() {
        return false;
    }
    for (c, n) in sizes {
        let pool = c.of_mut(shape);
        if pool.is_empty() {
            continue;
        }
        let start = i.saturating_mul(n).min(pool.len());
        let end = start.saturating_add(n).min(pool.len());
        pool.drain(start..end);
    }
    shape.keys.remove(i);
    if shape.keys.is_empty() {
        list.remove(at);
    }
    true
}

/// Drop the object's blend shape on `param` (at its base key it adds
/// nothing, so the default shape stays).
pub fn remove(model: &mut Model, target: Target, param: usize) -> bool {
    let Some(list) = shapes_mut(model, target) else { return false };
    let before = list.len();
    list.retain(|s| s.parameter as usize != param);
    list.len() != before
}

const ALL: [Channel; 10] = [
    Channel::Positions,
    Channel::Opacity,
    Channel::Multiply,
    Channel::Screen,
    Channel::Angle,
    Channel::OriginX,
    Channel::OriginY,
    Channel::Scale,
    Channel::DrawOrder,
    Channel::Intensity,
];
