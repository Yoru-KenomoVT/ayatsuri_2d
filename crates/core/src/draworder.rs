//! Draw order groups with offscreen parts, walked the way moc3 5.3 runtimes
//! do (semantics from PurismCore, MIT, render.c): a part item whose part
//! draws offscreen takes one render slot for its layer, then its child
//! group's run of slots; a group's total counts every mesh and every nested
//! offscreen inside it, but not its own part's layer, which the parent
//! group counts.

use ayatsuri_format::model::{DrawGroup, DrawItem, Model};

/// Totals for `groups`, worked out from what each holds rather than trusted
/// from a file, so turning offscreen drawing on or off cannot leave them
/// stale; `offscreen` says which parts' layers take a slot (none before
/// moc3 5.3). A group caught in a cycle (never in a valid file) counts 0.
pub fn totals(groups: &[DrawGroup], offscreen: impl Fn(u32) -> bool) -> Vec<u32> {
    let n = groups.len();
    let mut total: Vec<Option<u32>> = vec![None; n];
    // children before parents: each pass settles at least one group, so n
    // passes are enough
    for _ in 0..=n {
        let mut settled = true;
        for (gi, g) in groups.iter().enumerate() {
            if total.get(gi).copied().flatten().is_some() {
                continue;
            }
            let mut sum = 0u32;
            let mut ready = true;
            for it in &g.items {
                if !it.part {
                    sum = sum.saturating_add(1);
                    continue;
                }
                if offscreen(it.object) {
                    sum = sum.saturating_add(1);
                }
                if let Some(c) = it.child_group {
                    match total.get(c as usize) {
                        Some(Some(t)) => sum = sum.saturating_add(*t),
                        Some(None) => ready = false,
                        None => {}
                    }
                }
            }
            if ready {
                if let Some(t) = total.get_mut(gi) {
                    *t = Some(sum);
                }
            } else {
                settled = false;
            }
        }
        if settled {
            break;
        }
    }
    total.into_iter().map(|t| t.unwrap_or(0)).collect()
}

/// The model's draw order group totals, counted the runtime's way.
pub fn group_totals(model: &Model) -> Vec<u32> {
    totals(&model.draw_groups, |p| {
        model.parts.get(p as usize).is_some_and(|p| p.offscreen.is_some())
    })
}

/// The nearest offscreen part above `start` (a parent part index), walking
/// up at most once per part so a loop in a broken file cannot spin.
fn nearest_offscreen(model: &Model, start: Option<u32>) -> Option<u32> {
    let mut at = start;
    for _ in 0..model.parts.len() {
        let pi = at?;
        let p = model.parts.get(pi as usize)?;
        if p.offscreen.is_some() {
            return Some(pi);
        }
        at = p.parent_part;
    }
    None
}

/// Per part: the lowest index of a mesh anywhere inside it (u32::MAX when
/// it holds none). Items that tie on draw order keep this order, in the
/// editor and in the groups `synthesize` writes, so the two agree.
pub fn part_first_mesh(model: &Model) -> Vec<u32> {
    let mut first = vec![u32::MAX; model.parts.len()];
    for (mi, m) in model.art_meshes.iter().enumerate() {
        let mut at = m.parent_part;
        for _ in 0..model.parts.len() {
            let Some(pi) = at else { break };
            if let Some(f) = first.get_mut(pi as usize) {
                *f = (*f).min(mi as u32);
            }
            at = model.parts.get(pi as usize).and_then(|p| p.parent_part);
        }
    }
    first
}

/// Draw order groups for a model that has none of its own: the root holds
/// every mesh outside an offscreen part and one part item per outermost
/// offscreen part, and each offscreen part's group holds its own the same
/// way. Ties list in mesh index order (a part by its lowest mesh), as the
/// editor sorts them. Empty when the model has no meshes.
pub fn synthesize(model: &Model) -> Vec<DrawGroup> {
    if model.art_meshes.is_empty() {
        return Vec::new();
    }
    // group 0 is the root; offscreen part p gets group slot[p]
    let mut slot = vec![None; model.parts.len()];
    let mut next = 1u32;
    for (pi, p) in model.parts.iter().enumerate() {
        if p.offscreen.is_some()
            && let Some(s) = slot.get_mut(pi)
        {
            *s = Some(next);
            next += 1;
        }
    }
    let first = part_first_mesh(model);
    let group_of = |owner: Option<u32>| match owner {
        None => Some(0usize),
        Some(p) => slot.get(p as usize).copied().flatten().map(|g| g as usize),
    };
    // (tie key, item) per group
    let mut items: Vec<Vec<(u32, DrawItem)>> = vec![Vec::new(); next as usize];
    let mut orders: Vec<Vec<f32>> = vec![Vec::new(); next as usize];
    for (mi, m) in model.art_meshes.iter().enumerate() {
        let Some(g) = group_of(nearest_offscreen(model, m.parent_part)) else { continue };
        if let (Some(list), Some(o)) = (items.get_mut(g), orders.get_mut(g)) {
            list.push((mi as u32, DrawItem { part: false, object: mi as u32, child_group: None }));
            o.extend_from_slice(&m.draw_orders);
        }
    }
    for (pi, p) in model.parts.iter().enumerate() {
        let Some(own) = slot.get(pi).copied().flatten() else { continue };
        let Some(g) = group_of(nearest_offscreen(model, p.parent_part)) else { continue };
        let tie = first.get(pi).copied().unwrap_or(u32::MAX);
        if let (Some(list), Some(o)) = (items.get_mut(g), orders.get_mut(g)) {
            list.push((tie, DrawItem { part: true, object: pi as u32, child_group: Some(own) }));
            o.extend_from_slice(&p.draw_orders);
        }
    }
    let mut groups: Vec<DrawGroup> = items
        .into_iter()
        .zip(orders)
        .map(|(mut list, o)| {
            list.sort_by_key(|(tie, it)| (*tie, it.part));
            let round = |v: &f32| v.round() as i32;
            DrawGroup {
                total: 0,
                min_order: o.iter().map(round).min().unwrap_or(500),
                max_order: o.iter().map(round).max().unwrap_or(500),
                items: list.into_iter().map(|(_, it)| it).collect(),
            }
        })
        .collect();
    let counted =
        totals(&groups, |p| model.parts.get(p as usize).is_some_and(|p| p.offscreen.is_some()));
    for (g, t) in groups.iter_mut().zip(counted) {
        g.total = t;
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Binding, Offscreen, Part};

    fn mesh(part: Option<u32>, order: f32) -> ArtMesh {
        ArtMesh {
            id: String::new(),
            parent_part: part,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 0,
            uvs: vec![],
            indices: vec![],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![order],
            positions: vec![],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
        }
    }

    fn part(parent: Option<u32>, offscreen: bool) -> Part {
        Part {
            id: String::new(),
            parent_part: parent,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            draw_orders: vec![200.0],
            offscreen: offscreen.then(|| Offscreen {
                masks: vec![],
                inverted: false,
                blend: Default::default(),
                opacities: vec![1.0],
            }),
            blend_shapes: Vec::new(),
        }
    }

    #[test]
    fn offscreen_parts_become_part_items_with_groups_of_their_own() {
        // part 0 offscreen holds part 1 (plain) and part 2 (offscreen)
        let mut m = Model::new();
        m.parts = vec![part(None, true), part(Some(0), false), part(Some(0), true)];
        m.art_meshes = vec![mesh(None, 100.0), mesh(Some(1), 300.0), mesh(Some(2), 400.0)];
        let g = synthesize(&m);
        assert_eq!(g.len(), 3);
        // root: mesh 0, then part 0 (its lowest mesh is 1)
        let root: Vec<(bool, u32)> = g[0].items.iter().map(|i| (i.part, i.object)).collect();
        assert_eq!(root, vec![(false, 0), (true, 0)]);
        // part 0's group: mesh 1 (plain part 1 flattens in) and part 2
        let inner: Vec<(bool, u32)> = g[1].items.iter().map(|i| (i.part, i.object)).collect();
        assert_eq!(inner, vec![(false, 1), (true, 2)]);
        // totals: part 2's group holds mesh 2; part 0's holds mesh 1, part
        // 2's layer and its group; the root holds mesh 0, part 0's layer
        // and its group
        assert_eq!(g.iter().map(|g| g.total).collect::<Vec<_>>(), vec![5, 3, 1]);
        assert_eq!((g[0].min_order, g[0].max_order), (100, 200));
    }

    #[test]
    fn stored_totals_are_recounted() {
        let mut m = Model::new();
        m.parts = vec![part(None, true)];
        m.art_meshes = vec![mesh(Some(0), 500.0), mesh(None, 500.0)];
        m.draw_groups = synthesize(&m);
        for g in &mut m.draw_groups {
            g.total = 99;
        }
        assert_eq!(group_totals(&m), vec![3, 1]);
        // offscreen off: the part's layer slot goes
        m.parts[0].offscreen = None;
        assert_eq!(group_totals(&m), vec![2, 1]);
    }
}
