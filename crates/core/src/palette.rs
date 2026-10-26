//! The Parts palette's order, as Cubism shows it: each part lists its
//! sub-parts and art meshes together, front-most first, so at the same draw
//! order the row higher up draws in front. Art meshes are stored back to
//! front (a later mesh draws over an earlier one when their draw orders
//! tie), so a part's list runs from its highest mesh index down; a
//! sub-part sits where its front-most mesh does, and an empty one at the
//! bottom. Moving rows renumbers the meshes so the palette shows them where
//! they were dropped.

use ayatsuri_format::model::Model;

/// One row of the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Row {
    Part(usize),
    Mesh(usize),
}

/// Where dropped rows go: just above or below a row, among that row's
/// siblings, or into a part at the bottom of its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop {
    Above(Row),
    Below(Row),
    Into(usize),
}

/// A part's parent, when it names a real part.
fn parent_of_part(model: &Model, p: usize) -> Option<usize> {
    model
        .parts
        .get(p)
        .and_then(|x| x.parent_part)
        .map(|q| q as usize)
        .filter(|&q| q < model.parts.len())
}

fn parent_of_mesh(model: &Model, m: usize) -> Option<usize> {
    model
        .art_meshes
        .get(m)
        .and_then(|x| x.parent_part)
        .map(|q| q as usize)
        .filter(|&q| q < model.parts.len())
}

/// The part a row sits in (None for the top level).
pub fn parent(model: &Model, row: Row) -> Option<usize> {
    match row {
        Row::Part(p) => parent_of_part(model, p),
        Row::Mesh(m) => parent_of_mesh(model, m),
    }
}

/// Whether part `p` is `of` or inside it, walking up at most once per part
/// so a loop in a broken file cannot spin.
pub fn within(model: &Model, p: usize, of: usize) -> bool {
    let mut at = Some(p);
    for _ in 0..=model.parts.len() {
        match at {
            Some(x) if x == of => return true,
            Some(x) => at = parent_of_part(model, x),
            None => return false,
        }
    }
    false
}

/// Per part, its front-most (highest) mesh index anywhere inside it.
fn front_mesh(model: &Model) -> Vec<Option<usize>> {
    let mut front = vec![None; model.parts.len()];
    for mi in 0..model.art_meshes.len() {
        let mut at = parent_of_mesh(model, mi);
        for _ in 0..model.parts.len() {
            let Some(p) = at else { break };
            if let Some(f) = front.get_mut(p) {
                *f = Some(f.map_or(mi, |x: usize| x.max(mi)));
            }
            at = parent_of_part(model, p);
        }
    }
    front
}

/// Every row, top to bottom, with its depth (0 for the top level). Parts
/// caught in a loop (never in a valid file) are left out.
pub fn rows(model: &Model) -> Vec<(Row, usize)> {
    let front = front_mesh(model);
    // children per parent, the top level last
    let n = model.parts.len();
    let mut kids: Vec<Vec<Row>> = vec![Vec::new(); n + 1];
    let slot = |p: Option<usize>| p.unwrap_or(n);
    for p in 0..n {
        if let Some(k) = kids.get_mut(slot(parent_of_part(model, p))) {
            k.push(Row::Part(p));
        }
    }
    for m in 0..model.art_meshes.len() {
        if let Some(k) = kids.get_mut(slot(parent_of_mesh(model, m))) {
            k.push(Row::Mesh(m));
        }
    }
    // front-most first; empty parts at the bottom in their stored order
    let key = |r: &Row| match *r {
        Row::Mesh(m) => (1, m as i64),
        Row::Part(p) => match front.get(p).copied().flatten() {
            Some(m) => (1, m as i64),
            None => (0, -(p as i64)),
        },
    };
    for k in &mut kids {
        k.sort_by_key(|r| std::cmp::Reverse(key(r)));
    }
    let mut out = Vec::new();
    let mut stack: Vec<(Row, usize)> =
        kids.get(n).map(|k| k.iter().rev().map(|&r| (r, 0)).collect()).unwrap_or_default();
    let mut seen = vec![false; n];
    while let Some((r, depth)) = stack.pop() {
        out.push((r, depth));
        if let Row::Part(p) = r {
            if seen.get(p).copied().unwrap_or(true) {
                continue;
            }
            if let Some(s) = seen.get_mut(p) {
                *s = true;
            }
            if let Some(k) = kids.get(p) {
                stack.extend(k.iter().rev().map(|&c| (c, depth + 1)));
            }
        }
    }
    out
}

/// Renumber the art meshes: `order[new] = old`. Everything that names a
/// mesh by index follows: masks, clip groups, offscreen masks, glue and
/// draw order groups (whose items keep the new order for ties). False when
/// `order` is not a permutation of the meshes.
pub fn reorder_meshes(model: &mut Model, order: &[usize]) -> bool {
    let n = model.art_meshes.len();
    let mut new_of = vec![usize::MAX; n];
    for (new, &old) in order.iter().enumerate() {
        match new_of.get_mut(old) {
            Some(slot) if *slot == usize::MAX => *slot = new,
            _ => return false,
        }
    }
    if order.len() != n {
        return false;
    }
    if order.iter().enumerate().all(|(i, &o)| i == o) {
        return false;
    }
    let map = |x: &mut u32| {
        if let Some(&v) = new_of.get(*x as usize) {
            *x = v as u32;
        }
    };
    let old = std::mem::take(&mut model.art_meshes);
    let mut slots: Vec<Option<_>> = old.into_iter().map(Some).collect();
    model.art_meshes =
        order.iter().filter_map(|&o| slots.get_mut(o).and_then(Option::take)).collect();
    for m in &mut model.art_meshes {
        m.masks.iter_mut().for_each(map);
    }
    for g in &mut model.clip_groups {
        g.masks.iter_mut().for_each(map);
    }
    for o in model.parts.iter_mut().filter_map(|p| p.offscreen.as_mut()) {
        o.masks.iter_mut().for_each(map);
    }
    for g in &mut model.glues {
        map(&mut g.art_mesh_a);
        map(&mut g.art_mesh_b);
    }
    for g in &mut model.draw_groups {
        for it in g.items.iter_mut().filter(|it| !it.part) {
            map(&mut it.object);
        }
    }
    // ties in a group go in stored order, so the items follow the meshes
    let first = crate::draworder::part_first_mesh(model);
    for g in &mut model.draw_groups {
        g.items.sort_by_key(|it| {
            let tie = if it.part {
                first.get(it.object as usize).copied().unwrap_or(u32::MAX)
            } else {
                it.object
            };
            (tie, it.part)
        });
    }
    true
}

/// Move rows to a drop place, Cubism's Parts palette drag and drop: they
/// join the drop's part, keep their order among themselves, and their
/// meshes (a part's whole contents) are renumbered to sit there. A part
/// cannot go inside itself. True when anything changed.
pub fn move_rows(model: &mut Model, what: &[Row], to: Drop) -> bool {
    let dest = match to {
        Drop::Above(r) | Drop::Below(r) => parent(model, r),
        Drop::Into(p) => Some(p).filter(|&p| p < model.parts.len()),
    };
    if matches!(to, Drop::Into(p) if p >= model.parts.len()) {
        return false;
    }
    let moved_parts: Vec<usize> = what
        .iter()
        .filter_map(|r| match *r {
            Row::Part(p) if p < model.parts.len() => Some(p),
            _ => None,
        })
        .collect();
    // no part into itself or its own insides, and no drop beside a row
    // that is itself on the move
    if dest.is_some_and(|d| moved_parts.iter().any(|&p| within(model, d, p))) {
        return false;
    }
    if let Drop::Above(r) | Drop::Below(r) = to
        && what.contains(&r)
    {
        return false;
    }
    let inside_moved = |m: usize| {
        what.contains(&Row::Mesh(m))
            || parent_of_mesh(model, m)
                .is_some_and(|q| moved_parts.iter().any(|&p| within(model, q, p)))
    };
    // the meshes top to bottom as the palette shows them
    let flat: Vec<usize> = rows(model)
        .into_iter()
        .filter_map(|(r, _)| match r {
            Row::Mesh(m) => Some(m),
            Row::Part(_) => None,
        })
        .collect();
    let block: Vec<usize> = flat.iter().copied().filter(|&m| inside_moved(m)).collect();
    let rest: Vec<usize> = flat.iter().copied().filter(|&m| !inside_moved(m)).collect();
    let in_part =
        |m: usize, p: usize| parent_of_mesh(model, m).is_some_and(|q| within(model, q, p));
    // just past the last mesh (in `rest`) inside part p, or inside the top
    // level for None; an empty part's bottom is its parent's
    let bottom = |p: Option<usize>| -> usize {
        let mut at = p;
        for _ in 0..=model.parts.len() {
            match at {
                None => return rest.len(),
                Some(q) => {
                    if let Some(i) = rest.iter().rposition(|&m| in_part(m, q)) {
                        return i + 1;
                    }
                    at = parent_of_part(model, q);
                }
            }
        }
        rest.len()
    };
    let at = match to {
        Drop::Into(p) => bottom(Some(p)),
        Drop::Above(Row::Mesh(m)) => rest.iter().position(|&x| x == m).unwrap_or(rest.len()),
        Drop::Below(Row::Mesh(m)) => {
            rest.iter().position(|&x| x == m).map_or(rest.len(), |i| i + 1)
        }
        Drop::Above(Row::Part(p)) => match rest.iter().position(|&m| in_part(m, p)) {
            Some(i) => i,
            None => bottom(parent_of_part(model, p)),
        },
        Drop::Below(Row::Part(p)) => match rest.iter().rposition(|&m| in_part(m, p)) {
            Some(i) => i + 1,
            None => bottom(parent_of_part(model, p)),
        },
    };
    let mut new_flat = rest.clone();
    let at = at.min(new_flat.len());
    new_flat.splice(at..at, block.iter().copied());
    // meshes the walk missed (a broken parent loop) keep their place at the back
    let listed: std::collections::HashSet<usize> = new_flat.iter().copied().collect();
    let mut order: Vec<usize> =
        (0..model.art_meshes.len()).filter(|m| !listed.contains(m)).collect();
    order.extend(new_flat.iter().rev().copied());

    let mut changed = false;
    for r in what {
        match *r {
            Row::Mesh(m) => {
                if let Some(x) = model.art_meshes.get_mut(m) {
                    let to = dest.map(|d| d as u32);
                    changed |= x.parent_part != to;
                    x.parent_part = to;
                }
            }
            Row::Part(p) => {
                if let Some(x) = model.parts.get_mut(p) {
                    let to = dest.map(|d| d as u32);
                    changed |= x.parent_part != to;
                    x.parent_part = to;
                }
            }
        }
    }
    changed |= reorder_meshes(model, &order);
    changed
}

/// Cubism's New Part: a part with this id and draw order in `parent`,
/// holding `children` when given (Put selected objects into children).
/// The children keep their stored order, so nothing changes on the canvas
/// and the new part shows where its front-most child was. A part that is
/// `parent` itself or holds it cannot be a child. Returns the new index.
pub fn new_part(
    model: &mut Model,
    id: &str,
    parent: Option<usize>,
    draw_order: f32,
    children: &[Row],
) -> usize {
    let parent = parent.filter(|&p| p < model.parts.len());
    let index = model.parts.len();
    model.parts.push(ayatsuri_format::model::Part {
        id: id.to_string(),
        parent_part: parent.map(|p| p as u32),
        visible: true,
        enabled: true,
        binding: Default::default(),
        // no parameter keys yet: one keyform
        draw_orders: vec![draw_order],
        offscreen: None,
        blend_shapes: Vec::new(),
    });
    for r in children {
        match *r {
            Row::Mesh(m) => {
                if let Some(x) = model.art_meshes.get_mut(m) {
                    x.parent_part = Some(index as u32);
                }
            }
            Row::Part(p) if p != index && !parent.is_some_and(|q| within(model, q, p)) => {
                if let Some(x) = model.parts.get_mut(p) {
                    x.parent_part = Some(index as u32);
                }
            }
            Row::Part(_) => {}
        }
    }
    index
}

/// An id no part has yet: `PartN` from the part count up.
pub fn free_part_id(model: &Model) -> String {
    let mut n = model.parts.len();
    loop {
        let id = format!("Part{n}");
        if !model.parts.iter().any(|p| p.id == id) {
            return id;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Binding, Part};

    fn mesh(id: &str, part: Option<u32>) -> ArtMesh {
        ArtMesh {
            id: id.into(),
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
            draw_orders: vec![500.0],
            positions: vec![],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
        }
    }

    fn part(id: &str, parent: Option<u32>) -> Part {
        Part {
            id: id.into(),
            parent_part: parent,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            draw_orders: vec![500.0],
            offscreen: None,
            blend_shapes: Vec::new(),
        }
    }

    /// Face (part 0) holds Eye (part 1); meshes stored back to front:
    /// skin (Face), white and iris (Eye), and a loose back hair.
    fn face() -> Model {
        let mut m = Model::new();
        m.parts = vec![part("Face", None), part("Eye", Some(0))];
        m.art_meshes = vec![
            mesh("back_hair", None),
            mesh("skin", Some(0)),
            mesh("white", Some(1)),
            mesh("iris", Some(1)),
        ];
        m
    }

    fn shown(m: &Model) -> Vec<String> {
        rows(m)
            .into_iter()
            .map(|(r, d)| {
                let name = match r {
                    Row::Part(p) => m.parts[p].id.clone(),
                    Row::Mesh(i) => m.art_meshes[i].id.clone(),
                };
                format!("{}{name}", "  ".repeat(d))
            })
            .collect()
    }

    #[test]
    fn the_palette_lists_front_most_first() {
        assert_eq!(
            shown(&face()),
            ["Face", "  Eye", "    iris", "    white", "  skin", "back_hair"]
        );
    }

    #[test]
    fn a_mesh_dropped_above_another_draws_in_front_of_it() {
        let mut m = face();
        // white above iris
        assert!(move_rows(&mut m, &[Row::Mesh(2)], Drop::Above(Row::Mesh(3))));
        assert_eq!(shown(&m), ["Face", "  Eye", "    white", "    iris", "  skin", "back_hair"]);
        let ids: Vec<&str> = m.art_meshes.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["back_hair", "skin", "iris", "white"]);
    }

    #[test]
    fn a_mesh_moves_into_another_part_and_masks_follow() {
        let mut m = face();
        m.art_meshes[3].masks = vec![2]; // iris clipped by white
        m.glues.push(ayatsuri_format::model::Glue {
            id: "g".into(),
            art_mesh_a: 0,
            art_mesh_b: 2,
            binding: Binding::default(),
            intensities: vec![1.0],
            weights: vec![],
            indices: vec![],
            blend_shapes: Vec::new(),
        });
        // back hair into Eye, at its bottom
        assert!(move_rows(&mut m, &[Row::Mesh(0)], Drop::Into(1)));
        assert_eq!(
            shown(&m),
            ["Face", "  Eye", "    iris", "    white", "    back_hair", "  skin"]
        );
        let at = |id: &str| m.art_meshes.iter().position(|x| x.id == id).unwrap() as u32;
        let iris = &m.art_meshes[at("iris") as usize];
        assert_eq!(iris.masks, vec![at("white")]);
        assert_eq!((m.glues[0].art_mesh_a, m.glues[0].art_mesh_b), (at("back_hair"), at("white")));
    }

    #[test]
    fn a_part_moves_with_everything_inside_it() {
        let mut m = face();
        // Eye out of Face, below back hair at the top level
        assert!(move_rows(&mut m, &[Row::Part(1)], Drop::Below(Row::Mesh(0))));
        assert_eq!(shown(&m), ["Face", "  skin", "back_hair", "Eye", "  iris", "  white"]);
        assert_eq!(m.parts[1].parent_part, None);
    }

    #[test]
    fn a_part_cannot_go_inside_itself() {
        let mut m = face();
        assert!(!move_rows(&mut m, &[Row::Part(0)], Drop::Into(1)));
        assert!(!move_rows(&mut m, &[Row::Part(0)], Drop::Above(Row::Mesh(2))));
        assert_eq!(shown(&m), shown(&face()));
    }

    #[test]
    fn a_new_part_takes_the_selection_without_moving_it() {
        let mut m = face();
        let p = new_part(&mut m, "Lids", Some(1), 500.0, &[Row::Mesh(2)]);
        assert_eq!(p, 2);
        assert_eq!(
            shown(&m),
            ["Face", "  Eye", "    iris", "    Lids", "      white", "  skin", "back_hair"]
        );
        // stored order untouched
        let ids: Vec<&str> = m.art_meshes.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["back_hair", "skin", "white", "iris"]);
        assert_eq!(free_part_id(&m), "Part3");
    }
}
