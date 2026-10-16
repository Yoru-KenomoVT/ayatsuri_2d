// What changed between two states of a model, named the way a rigger
// would say it: "warp deformer Head moved", "mesh eye_L added". The
// timelapse player lists these beside each step and highlights the objects
// on the frame; objects match by id, so a renamed object reads as removed
// and added.

use ayatsuri_format::model::{Binding, DeformerKind, Model};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Added,
    Removed,
    Edited,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum What {
    Canvas,
    Parameter,
    Part,
    Warp,
    Rotation,
    Mesh,
    Glue,
    Physics,
    ArtPath,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    pub kind: Kind,
    pub what: What,
    pub name: String,
    /// Where the object sits in the newer model, when it is still there.
    pub index: Option<usize>,
    /// A few words on what about it changed, for an edit.
    pub detail: &'static str,
}

/// How big a model is at a step.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Counts {
    pub parameters: usize,
    pub parts: usize,
    pub deformers: usize,
    pub meshes: usize,
    pub keyforms: usize,
    pub points: usize,
}

pub fn counts(m: &Model) -> Counts {
    Counts {
        parameters: m.parameters.len(),
        parts: m.parts.len(),
        deformers: m.deformers.len(),
        meshes: m.art_meshes.len(),
        keyforms: m.art_meshes.iter().map(|a| a.binding.keyform_count()).sum::<usize>()
            + m.deformers.iter().map(|d| d.binding.keyform_count()).sum::<usize>(),
        points: m.art_meshes.iter().map(|a| a.vertex_count as usize).sum(),
    }
}

fn same_binding(a: &Binding, b: &Binding) -> bool {
    a.axes.len() == b.axes.len()
        && a.axes.iter().zip(&b.axes).all(|(x, y)| x.parameter == y.parameter && x.keys == y.keys)
}

fn packed<T: serde::Serialize>(v: &T) -> Vec<u8> {
    rmp_serde::to_vec(v).unwrap_or_default()
}

/// Walk two id-keyed lists and report what was added, removed or edited.
/// `edited` says in a few words how an object that is in both differs, or
/// None when it is the same.
fn by_id<T>(
    before: &[T],
    after: &[T],
    id: impl Fn(&T) -> &str,
    what: What,
    edited: impl Fn(&T, &T) -> Option<&'static str>,
    out: &mut Vec<Change>,
) {
    for (i, a) in after.iter().enumerate() {
        match before.iter().find(|b| id(b) == id(a)) {
            None => out.push(Change {
                kind: Kind::Added,
                what,
                name: id(a).to_string(),
                index: Some(i),
                detail: "",
            }),
            Some(b) => {
                if let Some(detail) = edited(b, a) {
                    out.push(Change {
                        kind: Kind::Edited,
                        what,
                        name: id(a).to_string(),
                        index: Some(i),
                        detail,
                    });
                }
            }
        }
    }
    for b in before {
        if !after.iter().any(|a| id(a) == id(b)) {
            out.push(Change {
                kind: Kind::Removed,
                what,
                name: id(b).to_string(),
                index: None,
                detail: "",
            });
        }
    }
}

/// The changes from `before` to `after`, meshes and deformers first since
/// those are what shows on the frame.
pub fn diff(before: &Model, after: &Model) -> Vec<Change> {
    let mut out = Vec::new();
    by_id(
        &before.art_meshes,
        &after.art_meshes,
        |m| &m.id,
        What::Mesh,
        |b, a| {
            if b.vertex_count != a.vertex_count || b.indices != a.indices || b.uvs != a.uvs {
                Some("mesh rebuilt")
            } else if !same_binding(&b.binding, &a.binding) {
                Some("keys changed")
            } else if b.positions != a.positions {
                Some("points moved")
            } else if b.opacities != a.opacities {
                Some("opacity")
            } else if b.draw_orders != a.draw_orders {
                Some("draw order")
            } else if b.masks != a.masks || b.flags != a.flags {
                Some("masks or blending")
            } else if b.parent_deformer != a.parent_deformer || b.parent_part != a.parent_part {
                Some("moved in the tree")
            } else if b.visible != a.visible || b.enabled != a.enabled {
                Some("visibility")
            } else if b.texture != a.texture {
                Some("texture page")
            } else {
                None
            }
        },
        &mut out,
    );
    // deformers split by kind so the list and the overlay know what to draw
    let mut defs = Vec::new();
    by_id(
        &before.deformers,
        &after.deformers,
        |d| &d.id,
        What::Warp,
        |b, a| match (&b.kind, &a.kind) {
            (DeformerKind::Warp(x), DeformerKind::Warp(y)) => {
                if x.rows != y.rows || x.cols != y.cols {
                    Some("divisions changed")
                } else if !same_binding(&b.binding, &a.binding) {
                    Some("keys changed")
                } else if x.positions != y.positions {
                    Some("grid moved")
                } else if x.opacities != y.opacities {
                    Some("opacity")
                } else if x.quad != y.quad {
                    Some("interpolation")
                } else if b.parent_deformer != a.parent_deformer || b.parent_part != a.parent_part {
                    Some("moved in the tree")
                } else if b.visible != a.visible || b.enabled != a.enabled {
                    Some("visibility")
                } else {
                    None
                }
            }
            (DeformerKind::Rotation(x), DeformerKind::Rotation(y)) => {
                if !same_binding(&b.binding, &a.binding) {
                    Some("keys changed")
                } else if x.angles != y.angles || x.base_angle != y.base_angle {
                    Some("angle")
                } else if x.origins_x != y.origins_x || x.origins_y != y.origins_y {
                    Some("origin moved")
                } else if x.scales != y.scales {
                    Some("scale")
                } else if x.reflect_x != y.reflect_x || x.reflect_y != y.reflect_y {
                    Some("reflection")
                } else if x.opacities != y.opacities {
                    Some("opacity")
                } else if b.parent_deformer != a.parent_deformer || b.parent_part != a.parent_part {
                    Some("moved in the tree")
                } else if b.visible != a.visible || b.enabled != a.enabled {
                    Some("visibility")
                } else {
                    None
                }
            }
            _ => Some("kind changed"),
        },
        &mut defs,
    );
    for mut c in defs {
        let kind_of = |m: &Model| {
            m.deformers.iter().find(|d| d.id == c.name).map(|d| match d.kind {
                DeformerKind::Warp(_) => What::Warp,
                DeformerKind::Rotation(_) => What::Rotation,
            })
        };
        c.what = kind_of(after).or_else(|| kind_of(before)).unwrap_or(What::Warp);
        out.push(c);
    }
    by_id(
        &before.parameters,
        &after.parameters,
        |p| &p.id,
        What::Parameter,
        |b, a| {
            if b.min != a.min || b.max != a.max {
                Some("range")
            } else if b.default != a.default {
                Some("default")
            } else if b.repeat != a.repeat || b.decimals != a.decimals {
                Some("settings")
            } else {
                None
            }
        },
        &mut out,
    );
    by_id(
        &before.parts,
        &after.parts,
        |p| &p.id,
        What::Part,
        |b, a| {
            if b.parent_part != a.parent_part {
                Some("moved in the tree")
            } else if !same_binding(&b.binding, &a.binding) || b.draw_orders != a.draw_orders {
                Some("draw order")
            } else if b.visible != a.visible || b.enabled != a.enabled {
                Some("visibility")
            } else {
                None
            }
        },
        &mut out,
    );
    by_id(
        &before.glues,
        &after.glues,
        |g| &g.id,
        What::Glue,
        |b, a| (packed(b) != packed(a)).then_some("changed"),
        &mut out,
    );
    by_id(
        &before.art_paths,
        &after.art_paths,
        |p| &p.mesh,
        What::ArtPath,
        |b, a| (b != a).then_some("stroke changed"),
        &mut out,
    );
    match (&before.physics, &after.physics) {
        (None, Some(_)) => out.push(Change {
            kind: Kind::Added,
            what: What::Physics,
            name: String::new(),
            index: None,
            detail: "",
        }),
        (Some(_), None) => out.push(Change {
            kind: Kind::Removed,
            what: What::Physics,
            name: String::new(),
            index: None,
            detail: "",
        }),
        (Some(b), Some(a)) if packed(b) != packed(a) => {
            let detail = if b.groups.len() != a.groups.len() { "groups" } else { "settings" };
            out.push(Change {
                kind: Kind::Edited,
                what: What::Physics,
                name: String::new(),
                index: None,
                detail,
            });
        }
        _ => {}
    }
    let (bc, ac) = (before.canvas, after.canvas);
    if bc.width != ac.width || bc.height != ac.height {
        out.push(Change {
            kind: Kind::Edited,
            what: What::Canvas,
            name: String::new(),
            index: None,
            detail: "size",
        });
    } else if bc.origin_x != ac.origin_x
        || bc.origin_y != ac.origin_y
        || bc.pixels_per_unit != ac.pixels_per_unit
    {
        out.push(Change {
            kind: Kind::Edited,
            what: What::Canvas,
            name: String::new(),
            index: None,
            detail: "origin or scale",
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Deformer, WarpData};

    fn mesh(id: &str) -> ArtMesh {
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
            positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
        }
    }

    fn warp(id: &str) -> Deformer {
        Deformer {
            id: id.into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            kind: DeformerKind::Warp(WarpData {
                rows: 1,
                cols: 1,
                quad: true,
                opacities: vec![1.0],
                positions: vec![0.0; 8],
            }),
        }
    }

    #[test]
    fn names_the_edit_a_rigger_made() {
        let mut a = Model::new();
        a.art_meshes.push(mesh("eye"));
        a.deformers.push(warp("head"));
        let mut b = a.clone();
        assert!(diff(&a, &b).is_empty());
        // a point nudged
        b.art_meshes[0].positions[2] = 2.0;
        let d = diff(&a, &b);
        assert_eq!(d.len(), 1);
        assert_eq!((d[0].kind, d[0].what, d[0].detail), (Kind::Edited, What::Mesh, "points moved"));
        assert_eq!(d[0].index, Some(0));
        // a deformer grid dragged and a mesh added, old one gone
        let mut c = a.clone();
        if let DeformerKind::Warp(w) = &mut c.deformers[0].kind {
            w.positions[0] = 0.5;
        }
        c.art_meshes.clear();
        c.art_meshes.push(mesh("mouth"));
        let d = diff(&a, &c);
        let kinds: Vec<(Kind, What)> = d.iter().map(|x| (x.kind, x.what)).collect();
        assert!(kinds.contains(&(Kind::Added, What::Mesh)));
        assert!(kinds.contains(&(Kind::Removed, What::Mesh)));
        assert!(kinds.contains(&(Kind::Edited, What::Warp)));
        assert_eq!(d.iter().find(|x| x.what == What::Warp).unwrap().detail, "grid moved");
    }

    #[test]
    fn counts_add_up() {
        let mut m = Model::new();
        m.art_meshes.push(mesh("a"));
        m.art_meshes.push(mesh("b"));
        m.deformers.push(warp("w"));
        let c = counts(&m);
        assert_eq!((c.meshes, c.deformers, c.keyforms, c.points), (2, 1, 3, 6));
    }
}
