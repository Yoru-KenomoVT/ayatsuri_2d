//! What a mesh really clips by: its masks and, with inherit on, the clips
//! of those masks in turn (a highlight inside
//! the iris inside the eye white). Every runtime clips a mask's own mask
//! away (it draws a mask with the clipped mesh's clip only), so a chain is
//! the editor's: it draws each level as a layer, and export rewrites it.

use ayatsuri_format::model::Model;

/// How deep inherited clips may chain.
pub const MAX_CHAIN: usize = 8;

/// A mesh's masks (a clip group keeps its meshes' masks in step, see
/// `edit::set_group_masks`), without unknown meshes.
fn masks_of(model: &Model, mesh: usize) -> Vec<u32> {
    let Some(m) = model.art_meshes.get(mesh) else { return Vec::new() };
    m.masks.iter().copied().filter(|&x| model.art_meshes.get(x as usize).is_some()).collect()
}

/// Why a chain stopped short of what inherit asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainStop {
    /// The masks at a level are clipped differently, so no one clip is
    /// theirs to pass on.
    MasksDisagree,
    /// A mask leads back to a mesh already in the chain.
    Loop,
    /// Deeper than `MAX_CHAIN`.
    TooDeep,
}

/// Every mask set the mesh clips by, its own first: it shows only where it
/// is inside some mask of every level. With inherit on, the next level is
/// what the masks of this one clip by, as long as they agree on it; it
/// goes on while those masks inherit too.
pub fn chain_of(model: &Model, mesh: usize) -> (Vec<Vec<u32>>, Option<ChainStop>) {
    let first = masks_of(model, mesh);
    if first.is_empty() {
        return (Vec::new(), None);
    }
    let mut seen = vec![mesh as u32];
    let mut inherit = model.art_meshes.get(mesh).is_some_and(|m| m.clip_inherit);
    let mut levels = vec![first];
    while inherit {
        let Some(level) = levels.last() else { break };
        if levels.len() >= MAX_CHAIN {
            return (levels, Some(ChainStop::TooDeep));
        }
        let Some(&lead) = level.first() else { break };
        let next = masks_of(model, lead as usize);
        if level.iter().any(|&x| masks_of(model, x as usize) != next) {
            return (levels, Some(ChainStop::MasksDisagree));
        }
        if next.is_empty() {
            break;
        }
        seen.extend_from_slice(level);
        if next.iter().any(|x| seen.contains(x)) {
            return (levels, Some(ChainStop::Loop));
        }
        inherit = model.art_meshes.get(lead as usize).is_some_and(|m| m.clip_inherit);
        levels.push(next);
    }
    (levels, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::ArtMesh;

    fn mesh(masks: Vec<u32>, inherit: bool) -> ArtMesh {
        ArtMesh {
            id: String::new(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 0,
            uvs: vec![],
            indices: vec![],
            masks,
            clip_group: None,
            clip_inherit: inherit,
            binding: Default::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![],
            blend_shapes: Vec::new(),
        }
    }

    #[test]
    fn inherit_follows_the_masks_clips() {
        // 0 eye white; 1 iris clipped by 0; 2 highlight clipped by 1
        let mut m = Model::new();
        m.art_meshes = vec![mesh(vec![], false), mesh(vec![0], false), mesh(vec![1], true)];
        assert_eq!(chain_of(&m, 2), (vec![vec![1], vec![0]], None));
        m.art_meshes[2].clip_inherit = false;
        assert_eq!(chain_of(&m, 2), (vec![vec![1]], None));
        // the iris inheriting from a lid (3) carries the chain one further
        m.art_meshes.push(mesh(vec![], false));
        m.art_meshes[2].clip_inherit = true;
        m.art_meshes[1].clip_inherit = true;
        m.art_meshes[0].masks = vec![3];
        assert_eq!(chain_of(&m, 2).0, vec![vec![1], vec![0], vec![3]]);
    }

    #[test]
    fn chains_stop_when_unsure() {
        let mut m = Model::new();
        m.art_meshes = vec![mesh(vec![], false), mesh(vec![0], false), mesh(vec![0, 1], true)];
        // masks 0 and 1 are clipped differently: no one clip to inherit
        assert_eq!(chain_of(&m, 2), (vec![vec![0, 1]], Some(ChainStop::MasksDisagree)));
        // a loop stops too
        let mut m = Model::new();
        m.art_meshes = vec![mesh(vec![1], true), mesh(vec![0], true), mesh(vec![0], true)];
        assert_eq!(chain_of(&m, 2).1, Some(ChainStop::Loop));
    }

    #[test]
    fn a_group_keeps_its_meshes_masks_in_step() {
        use crate::edit;
        let mut m = Model::new();
        m.art_meshes = vec![
            mesh(vec![], false),
            mesh(vec![], false),
            mesh(vec![0], false),
            mesh(vec![], false),
        ];
        let g = edit::create_clip_group(&mut m, 2, "Eyes".into()).unwrap_or(usize::MAX);
        assert!(edit::set_clip_group(&mut m, 3, Some(g)));
        assert_eq!(m.art_meshes[3].masks, vec![0]);
        // editing any member's masks edits the group: both follow
        assert!(edit::set_masks(&mut m, 3, &[0, 1]));
        assert_eq!(
            (m.art_meshes[2].masks.clone(), m.art_meshes[3].masks.clone()),
            (vec![0, 1], vec![0, 1])
        );
        // deleting a mask mesh renumbers the group and its meshes alike
        assert!(edit::delete_art_meshes(&mut m, &[0]));
        assert_eq!(m.clip_groups[0].masks, vec![0]);
        assert_eq!(m.art_meshes[2].masks, vec![0]);
        // deleting the group leaves the masks
        assert!(edit::delete_clip_group(&mut m, 0));
        assert_eq!((m.art_meshes[1].clip_group, m.art_meshes[1].masks.clone()), (None, vec![0]));
    }
}
