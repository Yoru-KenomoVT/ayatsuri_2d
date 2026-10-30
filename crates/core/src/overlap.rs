//! Overlapping polygons inside one art mesh (Cubism editor manual, Show
//! menu: "Highlight overlapping polygons"). With one of 5.3's advanced
//! blend modes, triangles of the same mesh that cover each other blend
//! twice where they overlap, and flicker as the mesh moves; Normal, Over
//! and the pre-5.3 Add and Multiply draw them the same either way.

use ayatsuri_format::model::{ArtMesh, Blend};

/// Whether this mesh's blend is one where its own overlaps show: anything
/// but Normal with Over, or the pre-5.3 Add and Multiply.
pub fn can_flicker(m: &ArtMesh) -> bool {
    !Blend::of(m.blend, m.flags).is_simple()
}

type P = [f32; 2];

/// How far two triangles may press into each other and still count as
/// only touching (shared edges, points on an edge).
const EPS: f32 = 1e-5;

fn tri(positions: &[f32], t: &[u16; 3]) -> Option<[P; 3]> {
    let p = |i: u16| -> Option<P> {
        let i = i as usize * 2;
        Some([*positions.get(i)?, *positions.get(i + 1)?])
    };
    Some([p(t[0])?, p(t[1])?, p(t[2])?])
}

fn area2(t: &[P; 3]) -> f32 {
    let [a, b, c] = t;
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Whether the insides of two triangles overlap: no edge of either one
/// has the other wholly on its far side (or on the line), the separating
/// axis test. Triangles that only share an edge or a corner do not.
fn overlap(a: &[P; 3], b: &[P; 3]) -> bool {
    // an edge of `t`, `t` itself on its left: does it shut `o` out?
    let shuts_out = |t: &[P; 3], o: &[P; 3]| {
        let s = area2(t).signum();
        let [t0, t1, t2] = *t;
        [(t0, t1), (t1, t2), (t2, t0)].into_iter().any(|(p, q)| {
            let scale = (q[0] - p[0]).hypot(q[1] - p[1]).max(1e-12);
            o.iter().all(|r| {
                let side = ((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])) * s;
                side / scale <= EPS
            })
        })
    };
    let tiny = |t: &[P; 3]| area2(t).abs() <= 1e-12;
    !tiny(a) && !tiny(b) && !shuts_out(a, b) && !shuts_out(b, a)
}

/// The triangles of a mesh that overlap another of its triangles, by
/// index into its triangle list, at the positions given (the mesh as it
/// is drawn now: overlaps can come and go as it deforms).
pub fn overlapping(positions: &[f32], indices: &[u16]) -> Vec<usize> {
    let tris: Vec<Option<([P; 3], [f32; 4])>> = indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let p = tri(positions, t)?;
            let x = p.map(|q| q[0]);
            let y = p.map(|q| q[1]);
            let lo = |v: [f32; 3]| v[0].min(v[1]).min(v[2]);
            let hi = |v: [f32; 3]| v[0].max(v[1]).max(v[2]);
            Some((p, [lo(x), lo(y), hi(x), hi(y)]))
        })
        .collect();
    let mut hit = vec![false; tris.len()];
    for (i, a) in tris.iter().enumerate() {
        let Some((ta, ba)) = a else { continue };
        for (j, b) in tris.iter().enumerate().skip(i + 1) {
            let Some((tb, bb)) = b else { continue };
            // boxes apart: no need for the full test
            if ba[2] < bb[0] || bb[2] < ba[0] || ba[3] < bb[1] || bb[3] < ba[1] {
                continue;
            }
            if overlap(ta, tb) {
                if let Some(h) = hit.get_mut(i) {
                    *h = true;
                }
                if let Some(h) = hit.get_mut(j) {
                    *h = true;
                }
            }
        }
    }
    hit.iter().enumerate().filter(|(_, h)| **h).map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_sharing_an_edge_do_not_overlap() {
        // a unit square as two triangles
        let pos = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        assert!(overlapping(&pos, &[0, 1, 2, 0, 2, 3]).is_empty());
        // corner to corner only
        let pos = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 2.0, 1.0, 1.0, 2.0];
        assert!(overlapping(&pos, &[0, 1, 2, 2, 3, 4]).is_empty());
    }

    #[test]
    fn a_folded_mesh_overlaps_itself() {
        // the second triangle folds back over the first
        let pos = [0.0, 0.0, 2.0, 0.0, 1.0, 2.0, 1.0, -0.5, 1.5, 1.0];
        let mut hits = overlapping(&pos, &[0, 1, 2, 3, 4, 0]);
        hits.sort_unstable();
        assert_eq!(hits, vec![0, 1]);
        // one triangle wholly inside another
        let pos = [0.0, 0.0, 4.0, 0.0, 0.0, 4.0, 0.5, 0.5, 1.0, 0.5, 0.5, 1.0];
        assert_eq!(overlapping(&pos, &[0, 1, 2, 3, 4, 5]).len(), 2);
    }

    #[test]
    fn only_advanced_blends_flicker() {
        use ayatsuri_format::model::{AlphaBlend, ColorBlend};
        let mut m = ArtMesh {
            id: "m".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Blend::NORMAL,
            vertex_count: 0,
            uvs: vec![],
            indices: vec![],
            masks: vec![],
            clip_group: None,
            clip_inherit: false,
            binding: Default::default(),
            opacities: vec![],
            draw_orders: vec![],
            positions: vec![],
            blend_shapes: vec![],
            multiply: Vec::new(),
            screen: Vec::new(),
        };
        assert!(!can_flicker(&m));
        m.flags = ayatsuri_format::model::flags::BLEND_ADDITIVE;
        assert!(!can_flicker(&m), "Add (before 5.3)");
        m.blend = Blend { color: ColorBlend::Screen, alpha: AlphaBlend::Over };
        assert!(can_flicker(&m));
        m.blend = Blend { color: ColorBlend::Normal, alpha: AlphaBlend::Atop };
        assert!(can_flicker(&m), "a 5.3 alpha blend");
    }
}
