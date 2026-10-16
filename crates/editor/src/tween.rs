// Between two steps of a timelapse the model is blended, so an edit plays
// as the drag it was instead of a jump: mesh points, warp grids, rotation
// angles and origins, opacities and draw orders ease from the step before
// to the step after. Objects that arrive fade in; a mesh rebuilt with a
// different point count can't be blended, so it fades in too. Objects that
// left are simply gone, since nothing in the newer model can hold them.

use ayatsuri_format::model::{DeformerKind, Model};

/// Ease in and out, the shape of a hand slowing into place.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp_all(before: &[f32], after: &mut [f32], t: f32) -> bool {
    if before.len() != after.len() {
        return false;
    }
    for (a, &b) in after.iter_mut().zip(before) {
        *a = b + (*a - b) * t;
    }
    true
}

/// The model `t` of the way from `before` to `after`, 0 being `before`'s
/// pose on `after`'s structure and 1 being `after` itself.
pub fn blend(before: &Model, after: &Model, t: f32) -> Model {
    let k = ease(t);
    let mut out = after.clone();
    if k >= 1.0 {
        return out;
    }
    for m in &mut out.art_meshes {
        match before.art_meshes.iter().find(|b| b.id == m.id) {
            Some(b)
                if b.vertex_count == m.vertex_count && b.positions.len() == m.positions.len() =>
            {
                lerp_all(&b.positions, &mut m.positions, k);
                lerp_all(&b.opacities, &mut m.opacities, k);
                lerp_all(&b.draw_orders, &mut m.draw_orders, k);
            }
            // new, or rebuilt: fade in
            _ => {
                for o in &mut m.opacities {
                    *o *= k;
                }
            }
        }
    }
    for d in &mut out.deformers {
        let Some(b) = before.deformers.iter().find(|b| b.id == d.id) else { continue };
        match (&b.kind, &mut d.kind) {
            (DeformerKind::Warp(x), DeformerKind::Warp(y)) => {
                lerp_all(&x.positions, &mut y.positions, k);
                lerp_all(&x.opacities, &mut y.opacities, k);
            }
            (DeformerKind::Rotation(x), DeformerKind::Rotation(y)) => {
                lerp_all(&x.angles, &mut y.angles, k);
                lerp_all(&x.origins_x, &mut y.origins_x, k);
                lerp_all(&x.origins_y, &mut y.origins_y, k);
                lerp_all(&x.scales, &mut y.scales, k);
                lerp_all(&x.opacities, &mut y.opacities, k);
                y.base_angle = x.base_angle + (y.base_angle - x.base_angle) * k;
            }
            _ => {}
        }
    }
    for p in &mut out.parts {
        if let Some(b) = before.parts.iter().find(|b| b.id == p.id) {
            lerp_all(&b.draw_orders, &mut p.draw_orders, k);
        }
    }
    for p in &mut out.parameters {
        if let Some(b) = before.parameters.iter().find(|b| b.id == p.id) {
            p.default = b.default + (p.default - b.default) * k;
        }
    }
    for g in &mut out.glues {
        if let Some(b) = before.glues.iter().find(|b| b.id == g.id) {
            lerp_all(&b.intensities, &mut g.intensities, k);
            lerp_all(&b.weights, &mut g.weights, k);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Binding};

    fn mesh(id: &str, x: f32) -> ArtMesh {
        ArtMesh {
            id: id.into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            vertex_count: 3,
            uvs: vec![0.0; 6],
            indices: vec![0, 1, 2],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![x, 0.0, 1.0, 0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn points_ease_and_newcomers_fade() {
        let mut a = Model::new();
        a.art_meshes.push(mesh("eye", 0.0));
        let mut b = a.clone();
        b.art_meshes[0].positions[0] = 10.0;
        b.art_meshes.push(mesh("mouth", 0.0));
        let half = blend(&a, &b, 0.5);
        assert!((half.art_meshes[0].positions[0] - 5.0).abs() < 1e-5);
        assert!((half.art_meshes[1].opacities[0] - 0.5).abs() < 1e-5);
        // eased: a quarter of the way in time is less than a quarter of the way in space
        let q = blend(&a, &b, 0.25);
        assert!(q.art_meshes[0].positions[0] < 2.5);
        assert_eq!(blend(&a, &b, 1.0).art_meshes[0].positions[0], 10.0);
        assert_eq!(blend(&a, &b, 0.0).art_meshes[0].positions[0], 0.0);
    }
}
