// Model back out to `.moc3`. The layout facts all come from the same place
// the reader's do: the research parser guide plus measurements of lawful
// files (openmoc3::sections has the slot table). The emit convention copies
// what measured files show: every slot defined by the target version gets
// an offset, ascending; a zero-length section shares the next section's
// offset; slots the version defines but the model never fills point at the
// file end; slots past the version are zero.

use std::collections::HashMap;

use ayatsuri_format::model::{Binding, Blend, DeformerKind, Model, flags};

use crate::ImportError;

/// The writable format revisions. v4.02 is the default and what current
/// runtimes expect; the older ones drop the sections they predate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportVersion {
    V3_00,
    V3_03,
    V4_00,
    V4_02,
    /// Cubism 5.0 to 5.2 (moc3 version 5).
    V5_00,
    /// Cubism 5.3, read by Cubism Core 6.0 (moc3 version 6).
    V5_03,
}

impl ExportVersion {
    /// What a new export starts on: the newest revision the apps people
    /// load models into can read. 5.3 grew the offset table to 0x16C0,
    /// past what OpenVT's reader will skip (0x1000), and VTube Studio's
    /// Core predates it.
    pub const DEFAULT: ExportVersion = ExportVersion::V5_00;

    /// `DEFAULT`'s place in `ALL`, for pickers.
    pub fn default_index() -> usize {
        Self::ALL.iter().position(|v| *v == Self::DEFAULT).unwrap_or(0)
    }

    pub const ALL: [ExportVersion; 6] = [
        ExportVersion::V3_00,
        ExportVersion::V3_03,
        ExportVersion::V4_00,
        ExportVersion::V4_02,
        ExportVersion::V5_00,
        ExportVersion::V5_03,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ExportVersion::V3_00 => "3.0",
            ExportVersion::V3_03 => "3.3",
            ExportVersion::V4_00 => "4.0",
            ExportVersion::V4_02 => "4.2",
            ExportVersion::V5_00 => "5.0",
            ExportVersion::V5_03 => "5.3",
        }
    }

    fn byte(self) -> u8 {
        match self {
            ExportVersion::V3_00 => 1,
            ExportVersion::V3_03 => 2,
            ExportVersion::V4_00 => 3,
            ExportVersion::V4_02 => 4,
            ExportVersion::V5_00 => 5,
            ExportVersion::V5_03 => 6,
        }
    }

    /// The highest slot this version defines. 101 (warp quad flags) arrived
    /// with 3.3; 102..=136 with 4.2; 137..=151 (per-keyform colour indices,
    /// blend shapes on parts, rotations and glue) with 5.0; 152..=166
    /// (offscreen drawing, extended blend modes) with 5.3.
    fn max_slot(self) -> usize {
        match self {
            ExportVersion::V3_00 => 100,
            ExportVersion::V3_03 | ExportVersion::V4_00 => 101,
            ExportVersion::V4_02 => 136,
            ExportVersion::V5_00 => 151,
            ExportVersion::V5_03 => 166,
        }
    }

    /// Count table size: 32 ints up to 4.2, 64 from 5.0.
    fn count_ints(self) -> usize {
        if self.max_slot() >= 137 { 64 } else { 32 }
    }

    /// The offset table's length in entries. Real 5.3 files end it at
    /// 0x16C0 (1440 entries); earlier ones at 0x7C0 (480).
    fn sot_entries(self) -> usize {
        if self.max_slot() >= 152 { 1440 } else { 480 }
    }
}

/// What the chosen revision cannot carry from this model. Empty means the
/// Shown for 5.3 exports. Tested 2026-10-12: VTube Studio 1.35.10 rejects
/// Live2D's own 5.3 models too ("The Core unsupport later than moc3
/// ver:[5]"), and OpenVT's reader stops at 5.3's longer offset table.
pub const VTS_NOTE: &str = "5.3 is not compatible with VTube Studio or OpenVT yet (they read up \
     to 5.0); export 5.0 for those";

/// export is lossless.
pub fn export_warnings(model: &Model, version: ExportVersion) -> Vec<String> {
    let mut out = Vec::new();
    if version.max_slot() < 101 {
        let quads = model
            .deformers
            .iter()
            .filter(|d| matches!(&d.kind, DeformerKind::Warp(w) if w.quad))
            .count();
        if quads > 0 {
            out.push(format!(
                "{quads} warp deformer(s) use quad interpolation, which 3.0 cannot \
                 store; they will interpolate as triangles"
            ));
        }
    }
    if version.max_slot() < 108 {
        let tinted = model
            .art_meshes
            .iter()
            .map(|m| (&m.multiply, &m.screen))
            .chain(model.deformers.iter().map(|d| (&d.multiply, &d.screen)))
            .filter(|(m, s)| m.iter().any(|&v| v != 1.0) || s.iter().any(|&v| v != 0.0))
            .count();
        if tinted > 0 {
            out.push(format!(
                "{tinted} object(s) use multiply or screen colours, which moc3 stores from 4.2 \
                 on; this version draws them without"
            ));
        }
    }
    let offscreen = model.parts.iter().filter(|p| p.offscreen.is_some()).count();
    if offscreen > 0 && version.max_slot() >= 152 && !model.draw_groups.is_empty() {
        let grouped: std::collections::HashSet<u32> = model
            .draw_groups
            .iter()
            .flat_map(|g| &g.items)
            .filter(|it| it.part && it.child_group.is_some())
            .map(|it| it.object)
            .collect();
        let loose = (0..model.parts.len() as u32)
            .filter(|p| {
                model.parts.get(*p as usize).is_some_and(|p| p.offscreen.is_some())
                    && !grouped.contains(p)
            })
            .count();
        if loose > 0 {
            out.push(format!(
                "{loose} offscreen part(s) are not draw order groups in this model's imported \
                 draw order, so they export drawn directly"
            ));
        }
    }
    if offscreen > 0 && version.max_slot() < 152 {
        out.push(format!(
            "{offscreen} part(s) use offscreen drawing, which only moc3 5.3 stores; this version \
             draws their contents directly, without the layer's opacity, clipping or blend mode"
        ));
    }
    // inherited clips go out as hidden helper masks (the editor's export
    // adds them); the ones that cannot say so
    let stuck = (0..model.art_meshes.len())
        .filter(|&mi| matches!(ayatsuri_core::clip::inherited_masks(model, mi), Some(Err(_))))
        .count();
    if stuck > 0 {
        out.push(format!(
            "{stuck} mesh(es) inherit a clip moc3 cannot carry (a level with more than one \
             mask, an inverted clip, or a chain that stops); they export clipped by their own \
             masks only"
        ));
    }
    let shaped = model.art_meshes.iter().filter(|m| !m.blend_shapes.is_empty()).count();
    if shaped > 0 && version.max_slot() < 136 {
        out.push(format!(
            "{shaped} mesh(es) have blend shapes, which need moc3 4.2 or later; this version \
             exports their normal keyforms only"
        ));
    }
    if version.max_slot() >= 152 {
        out.push(VTS_NOTE.into());
    } else {
        let changed = model
            .art_meshes
            .iter()
            .filter(|m| !Blend::of(m.blend, m.flags).legacy_bits().1)
            .count();
        if changed > 0 {
            out.push(format!(
                "{changed} mesh(es) use a blend mode only moc3 5.3 can store; this version \
                 draws them with the closest of Normal, Add and Multiply"
            ));
        }
    }
    if model.physics.is_some() {
        // physics never lives in the moc3 itself; noted so nobody hunts for it
        out.push("physics is written beside the moc3 as physics3.json".into());
    }
    out
}

const HEADER: usize = 0x40;

fn err(msg: impl Into<String>) -> ImportError {
    ImportError::Export(msg.into())
}

fn i32s(v: &[i32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn i16s(v: &[i16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn bools(v: &[bool]) -> Vec<u8> {
    v.iter().flat_map(|&b| (b as i32).to_le_bytes()).collect()
}

/// 64-byte NUL-padded records. Non-ASCII bytes become underscores so the
/// record stays what the format promises.
fn ids(names: &[String]) -> Vec<u8> {
    let mut out = Vec::with_capacity(names.len() * 64);
    for n in names {
        let mut rec = [0u8; 64];
        for (slot, b) in rec.iter_mut().take(63).zip(n.bytes()) {
            *slot = if b.is_ascii() && b != 0 { b } else { b'_' };
        }
        out.extend_from_slice(&rec);
    }
    out
}

/// The shared binding machinery: parameter bindings grouped per parameter,
/// keyform bindings deduped across objects, all four pools at once.
/// One keyform's multiply and screen colour, rgb.
type KeyColor = ([f32; 3], [f32; 3]);

/// An object's `n` keyforms' colours from its pools (three floats per
/// keyform; empty or short is neutral).
fn key_colors<'a>(
    multiply: &'a [f32],
    screen: &'a [f32],
    n: usize,
) -> impl Iterator<Item = KeyColor> + 'a {
    let rgb = |pool: &[f32], k: usize, neutral: f32| -> [f32; 3] {
        let at = |c: usize| pool.get(k * 3 + c).copied().unwrap_or(neutral);
        [at(0), at(1), at(2)]
    };
    (0..n).map(move |k| (rgb(multiply, k, 1.0), rgb(screen, k, 0.0)))
}

/// The blend shape weight limit pools (slots 131-136).
#[derive(Default)]
struct Limits {
    index: Vec<i32>,
    param: Vec<i32>,
    begin: Vec<i32>,
    count: Vec<i32>,
    keys: Vec<f32>,
    weights: Vec<f32>,
}

#[derive(Default)]
struct Bindings {
    keys: Vec<f32>,
    /// Slots 56/57, per parameter.
    pb_begin: Vec<i32>,
    pb_count: Vec<i32>,
    /// Slots 75/76, per parameter binding.
    key_begin: Vec<i32>,
    key_count: Vec<i32>,
    /// Slot 72.
    pbi: Vec<i32>,
    /// Slots 73/74.
    kb_begin: Vec<i32>,
    kb_count: Vec<i32>,
    axis_ids: HashMap<(u32, Vec<u32>), i32>,
    kb_ids: HashMap<Vec<i32>, i32>,
}

impl Bindings {
    fn build(model: &Model) -> Result<Self, ImportError> {
        let n_params = model.parameters.len();
        let mut me = Self::default();

        // first pass: every distinct (parameter, keys) axis, grouped per
        // parameter because slots 56/57 demand contiguity
        let mut per_param: Vec<Vec<Vec<f32>>> = vec![Vec::new(); n_params];
        let mut locals: HashMap<(u32, Vec<u32>), usize> = HashMap::new();
        let mut register = |b: &Binding| -> Result<(), ImportError> {
            for a in &b.axes {
                let p = a.parameter as usize;
                if p >= n_params {
                    return Err(err(format!("binding references parameter {p} of {n_params}")));
                }
                let bits: Vec<u32> = a.keys.iter().map(|k| k.to_bits()).collect();
                if let std::collections::hash_map::Entry::Vacant(slot) =
                    locals.entry((a.parameter, bits))
                {
                    let Some(list) = per_param.get_mut(p) else { continue };
                    slot.insert(list.len());
                    list.push(a.keys.clone());
                }
            }
            Ok(())
        };
        for p in &model.parts {
            register(&p.binding)?;
        }
        for d in &model.deformers {
            register(&d.binding)?;
        }
        for m in &model.art_meshes {
            register(&m.binding)?;
        }
        for g in &model.glues {
            register(&g.binding)?;
        }

        // lay the parameter bindings out and remember each axis's index
        let mut running = 0i32;
        for (p, list) in per_param.iter().enumerate() {
            me.pb_begin.push(running);
            me.pb_count.push(list.len() as i32);
            for keys in list {
                let bits: Vec<u32> = keys.iter().map(|k| k.to_bits()).collect();
                me.axis_ids.insert((p as u32, bits), running);
                me.key_begin.push(me.keys.len() as i32);
                me.key_count.push(keys.len() as i32);
                me.keys.extend_from_slice(keys);
                running += 1;
            }
        }
        Ok(me)
    }

    /// The keyform binding index for one object, deduped.
    fn keyform_binding(&mut self, b: &Binding) -> Result<i32, ImportError> {
        let mut list = Vec::with_capacity(b.axes.len());
        for a in &b.axes {
            let bits: Vec<u32> = a.keys.iter().map(|k| k.to_bits()).collect();
            let pb = self
                .axis_ids
                .get(&(a.parameter, bits))
                .copied()
                .ok_or_else(|| err("axis vanished between passes"))?;
            list.push(pb);
        }
        if let Some(&kb) = self.kb_ids.get(&list) {
            return Ok(kb);
        }
        let kb = self.kb_begin.len() as i32;
        self.kb_begin.push(self.pbi.len() as i32);
        self.kb_count.push(list.len() as i32);
        self.pbi.extend_from_slice(&list);
        self.kb_ids.insert(list, kb);
        Ok(kb)
    }
}

/// Serialize the model as a `.moc3` of the chosen revision.
pub fn to_moc3(model: &Model, version: ExportVersion) -> Result<Vec<u8>, ImportError> {
    // extended interpolation goes out as plain keys along its curve, the
    // way Cubism hands it to the runtime
    let baked;
    let model = if ayatsuri_core::edit::has_curves(model) {
        let mut m = model.clone();
        ayatsuri_core::edit::bake_curves(&mut m);
        baked = m;
        &baked
    } else {
        model
    };
    let n_params = model.parameters.len();
    let n_parts = model.parts.len();
    let n_deformers = model.deformers.len();
    let n_meshes = model.art_meshes.len();
    let n_glue = model.glues.len();
    if n_params > i32::MAX as usize / 64 || n_meshes > i32::MAX as usize / 64 {
        return Err(err("model too large for the format"));
    }

    let mut bind = Bindings::build(model)?;
    let mut sect: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut coords: Vec<f32> = Vec::new();

    // ---- parameters (49-57) ----
    sect.insert(49, vec![0u8; n_params * 8]);
    sect.insert(50, ids(&model.parameters.iter().map(|p| p.id.clone()).collect::<Vec<_>>()));
    sect.insert(51, f32s(&model.parameters.iter().map(|p| p.max).collect::<Vec<_>>()));
    sect.insert(52, f32s(&model.parameters.iter().map(|p| p.min).collect::<Vec<_>>()));
    sect.insert(53, f32s(&model.parameters.iter().map(|p| p.default).collect::<Vec<_>>()));
    sect.insert(54, bools(&model.parameters.iter().map(|p| p.repeat).collect::<Vec<_>>()));
    sect.insert(55, i32s(&model.parameters.iter().map(|p| p.decimals).collect::<Vec<_>>()));
    sect.insert(56, i32s(&bind.pb_begin.clone()));
    sect.insert(57, i32s(&bind.pb_count.clone()));

    // ---- parts (2-9, 58) ----
    let part_keyforms;
    {
        let mut kb = Vec::new();
        let mut kf_begin = Vec::new();
        let mut kf_count = Vec::new();
        let mut pool58: Vec<f32> = Vec::new();
        for (i, p) in model.parts.iter().enumerate() {
            let expected = p.binding.keyform_count();
            if p.draw_orders.len() != expected {
                return Err(err(format!(
                    "part {i}: {} keyforms stored, binding calls for {expected}",
                    p.draw_orders.len()
                )));
            }
            kb.push(bind.keyform_binding(&p.binding)?);
            kf_begin.push(pool58.len() as i32);
            kf_count.push(p.draw_orders.len() as i32);
            pool58.extend_from_slice(&p.draw_orders);
        }
        sect.insert(2, vec![0u8; n_parts * 8]);
        sect.insert(3, ids(&model.parts.iter().map(|p| p.id.clone()).collect::<Vec<_>>()));
        sect.insert(4, i32s(&kb));
        sect.insert(5, i32s(&kf_begin));
        sect.insert(6, i32s(&kf_count));
        sect.insert(7, bools(&model.parts.iter().map(|p| p.visible).collect::<Vec<_>>()));
        sect.insert(8, bools(&model.parts.iter().map(|p| p.enabled).collect::<Vec<_>>()));
        sect.insert(
            9,
            i32s(
                &model
                    .parts
                    .iter()
                    .map(|p| p.parent_part.map(|v| v as i32).unwrap_or(-1))
                    .collect::<Vec<_>>(),
            ),
        );
        sect.insert(58, f32s(&pool58));
        part_keyforms = pool58.len();
    }

    // ---- deformers (10-28, 59-67, 101) ----
    let mut d_kb = Vec::new();
    let mut d_types = Vec::new();
    let mut d_specific = Vec::new();
    let mut w_kb = Vec::new();
    let mut w_kf_begin = Vec::new();
    let mut w_kf_count = Vec::new();
    let mut w_vertex_counts = Vec::new();
    let mut w_rows = Vec::new();
    let mut w_cols = Vec::new();
    let mut w_quad = Vec::new();
    let mut w_opac: Vec<f32> = Vec::new();
    // each keyform's multiply and screen colour, in the order of the
    // opacities beside them: the colour pool is warps, rotations, meshes
    let mut w_col: Vec<KeyColor> = Vec::new();
    let mut r_col: Vec<KeyColor> = Vec::new();
    let mut w_pos_begin = Vec::new();
    let mut r_kb = Vec::new();
    let mut r_kf_begin = Vec::new();
    let mut r_kf_count = Vec::new();
    let mut r_base = Vec::new();
    let mut r_opac: Vec<f32> = Vec::new();
    let mut r_angles: Vec<f32> = Vec::new();
    let mut r_ox: Vec<f32> = Vec::new();
    let mut r_oy: Vec<f32> = Vec::new();
    let mut r_scales: Vec<f32> = Vec::new();
    let mut r_rx: Vec<bool> = Vec::new();
    let mut r_ry: Vec<bool> = Vec::new();
    for (i, d) in model.deformers.iter().enumerate() {
        if let Some(p) = d.parent_deformer
            && p as usize >= i
        {
            return Err(err(format!("deformer {i} precedes its parent {p}")));
        }
        let kb = bind.keyform_binding(&d.binding)?;
        d_kb.push(kb);
        let expected = d.binding.keyform_count();
        match &d.kind {
            DeformerKind::Warp(w) => {
                let grid_floats = w.grid_points() * 2;
                if w.opacities.len() != expected
                    || w.positions.len() != expected.saturating_mul(grid_floats)
                {
                    return Err(err(format!("warp deformer {i}: keyform arrays disagree")));
                }
                d_types.push(0);
                d_specific.push(w_rows.len() as i32);
                w_kb.push(kb);
                w_kf_begin.push(w_opac.len() as i32);
                w_kf_count.push(expected as i32);
                w_vertex_counts.push(w.grid_points() as i32);
                w_rows.push(w.rows as i32);
                w_cols.push(w.cols as i32);
                w_quad.push(w.quad);
                w_opac.extend_from_slice(&w.opacities);
                w_col.extend(key_colors(&d.multiply, &d.screen, expected));
                // one begin per keyform even when the grid is empty
                for k in 0..expected {
                    w_pos_begin.push(coords.len() as i32);
                    if let Some(form) = w.positions.get(k * grid_floats..(k + 1) * grid_floats) {
                        coords.extend_from_slice(form);
                    }
                }
            }
            DeformerKind::Rotation(r) => {
                if r.opacities.len() != expected
                    || r.angles.len() != expected
                    || r.origins_x.len() != expected
                    || r.origins_y.len() != expected
                    || r.scales.len() != expected
                {
                    return Err(err(format!("rotation deformer {i}: keyform arrays disagree")));
                }
                d_types.push(1);
                d_specific.push(r_kf_begin.len() as i32);
                r_kb.push(kb);
                r_kf_begin.push(r_opac.len() as i32);
                r_kf_count.push(expected as i32);
                r_base.push(r.base_angle);
                r_opac.extend_from_slice(&r.opacities);
                r_col.extend(key_colors(&d.multiply, &d.screen, expected));
                r_angles.extend_from_slice(&r.angles);
                r_ox.extend_from_slice(&r.origins_x);
                r_oy.extend_from_slice(&r.origins_y);
                r_scales.extend_from_slice(&r.scales);
                r_rx.extend(r.reflect_x.iter().copied());
                r_ry.extend(r.reflect_y.iter().copied());
                r_rx.resize(r_opac.len(), false);
                r_ry.resize(r_opac.len(), false);
            }
        }
    }
    let n_warps = w_rows.len();
    let n_rots = r_kf_begin.len();
    sect.insert(10, vec![0u8; n_deformers * 8]);
    sect.insert(11, ids(&model.deformers.iter().map(|d| d.id.clone()).collect::<Vec<_>>()));
    sect.insert(12, i32s(&d_kb));
    sect.insert(13, bools(&model.deformers.iter().map(|d| d.visible).collect::<Vec<_>>()));
    sect.insert(14, bools(&model.deformers.iter().map(|d| d.enabled).collect::<Vec<_>>()));
    sect.insert(
        15,
        i32s(
            &model
                .deformers
                .iter()
                .map(|d| d.parent_part.map(|v| v as i32).unwrap_or(-1))
                .collect::<Vec<_>>(),
        ),
    );
    sect.insert(
        16,
        i32s(
            &model
                .deformers
                .iter()
                .map(|d| d.parent_deformer.map(|v| v as i32).unwrap_or(-1))
                .collect::<Vec<_>>(),
        ),
    );
    sect.insert(17, i32s(&d_types));
    sect.insert(18, i32s(&d_specific));
    sect.insert(19, i32s(&w_kb));
    sect.insert(20, i32s(&w_kf_begin));
    sect.insert(21, i32s(&w_kf_count));
    sect.insert(22, i32s(&w_vertex_counts));
    sect.insert(23, i32s(&w_rows));
    sect.insert(24, i32s(&w_cols));
    sect.insert(25, i32s(&r_kb));
    sect.insert(26, i32s(&r_kf_begin));
    sect.insert(27, i32s(&r_kf_count));
    sect.insert(28, f32s(&r_base));
    sect.insert(59, f32s(&w_opac));
    sect.insert(60, i32s(&w_pos_begin));
    sect.insert(61, f32s(&r_opac));
    sect.insert(62, f32s(&r_angles));
    sect.insert(63, f32s(&r_ox));
    sect.insert(64, f32s(&r_oy));
    sect.insert(65, f32s(&r_scales));
    sect.insert(66, bools(&r_rx));
    sect.insert(67, bools(&r_ry));
    if version.max_slot() >= 101 {
        sect.insert(101, bools(&w_quad));
    }

    // ---- blend shapes (4.2+): one key table per blend shape parameter, its
    // keys every key a mesh's shapes have plus the base key 0.0; a table
    // keeps the key the base sits at (Cubism files measured: 021.moc3) ----
    let shapes_on = version.max_slot() >= 136;
    let mut bs_tables: Vec<(u32, Vec<f32>)> = Vec::new();
    if shapes_on {
        for shape in model.art_meshes.iter().flat_map(|m| &m.blend_shapes) {
            let at = match bs_tables.iter().position(|(p, _)| *p == shape.parameter) {
                Some(at) => at,
                None => {
                    bs_tables.push((shape.parameter, vec![0.0]));
                    bs_tables.len() - 1
                }
            };
            if let Some((_, keys)) = bs_tables.get_mut(at) {
                keys.extend_from_slice(&shape.keys);
            }
        }
        for (_, keys) in &mut bs_tables {
            keys.sort_by(f32::total_cmp);
            keys.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        }
    }

    // ---- art meshes (29-48, 68-70, 78-80) ----
    let mut m_kb = Vec::new();
    let mut m_kf_begin = Vec::new();
    let mut m_kf_count = Vec::new();
    let mut m_uv_begin = Vec::new();
    let mut m_idx_begin = Vec::new();
    let mut m_idx_count = Vec::new();
    let mut m_mask_begin = Vec::new();
    let mut m_mask_count = Vec::new();
    let mut m_opac: Vec<f32> = Vec::new();
    let mut m_col: Vec<KeyColor> = Vec::new();
    let mut m_orders: Vec<f32> = Vec::new();
    let mut m_pos_begin = Vec::new();
    let mut uv_pool: Vec<f32> = Vec::new();
    let mut tri_pool: Vec<i16> = Vec::new();
    let mut mask_pool: Vec<i32> = Vec::new();
    for (i, m) in model.art_meshes.iter().enumerate() {
        let vc = m.vertex_count as usize;
        let expected = m.binding.keyform_count();
        if m.uvs.len() != vc * 2
            || m.opacities.len() != expected
            || m.draw_orders.len() != expected
            || m.positions.len() != expected.saturating_mul(vc * 2)
        {
            return Err(err(format!("art mesh {i}: arrays disagree with vertex count")));
        }
        m_kb.push(bind.keyform_binding(&m.binding)?);
        m_uv_begin.push(uv_pool.len() as i32);
        uv_pool.extend_from_slice(&m.uvs);
        m_idx_begin.push(tri_pool.len() as i32);
        m_idx_count.push(m.indices.len() as i32);
        for &ix in &m.indices {
            if ix as usize >= vc {
                return Err(err(format!("art mesh {i}: index {ix} outside {vc} vertices")));
            }
            tri_pool.push(ix as i16);
        }
        m_mask_begin.push(mask_pool.len() as i32);
        m_mask_count.push(m.masks.len() as i32);
        mask_pool.extend(m.masks.iter().map(|&s| s as i32));
        m_kf_begin.push(m_opac.len() as i32);
        m_kf_count.push(expected as i32);
        m_opac.extend_from_slice(&m.opacities);
        m_col.extend(key_colors(&m.multiply, &m.screen, expected));
        m_orders.extend_from_slice(&m.draw_orders);
        for k in 0..expected {
            m_pos_begin.push(coords.len() as i32);
            if let Some(form) = m.positions.get(k * vc * 2..(k + 1) * vc * 2) {
                coords.extend_from_slice(form);
            }
        }
    }
    // the shapes' keyforms follow every normal mesh keyform: per shape one
    // binding with a keyform for each key of its table, base included, each
    // holding what the shape adds there (runtimes add it; opacity and draw
    // order add nothing, and the colours they link to are zero)
    let mut bs_bindings: Vec<(i32, i32, i32)> = Vec::new();
    let mut bs_meshes: Vec<(i32, i32, i32)> = Vec::new();
    // weight limits (131-136): per binding a run of constraint indices, one
    // constraint per limit (its parameter and graph), none shared; weights
    // are written as 0 to 1, as the model holds them (no Cubism file with
    // limits has been measured yet)
    let mut bs_limit_runs: Vec<(i32, i32)> = Vec::new();
    let mut lim = Limits::default();
    for (mi, m) in model.art_meshes.iter().enumerate().filter(|_| shapes_on) {
        if m.blend_shapes.is_empty() {
            continue;
        }
        let Some(base) = ayatsuri_core::edit::base_form(model, mi) else {
            return Err(err(format!(
                "art mesh {mi}: its blend shapes need keys at every parameter's default"
            )));
        };
        let len = base.len();
        let first = bs_bindings.len() as i32;
        for shape in &m.blend_shapes {
            if shape.positions.len() != shape.keys.len().saturating_mul(len) {
                return Err(err(format!(
                    "art mesh {mi}: blend shape points disagree with its vertices"
                )));
            }
            let Some(t) = bs_tables.iter().position(|(p, _)| *p == shape.parameter) else {
                continue;
            };
            let Some((_, keys)) = bs_tables.get(t) else { continue };
            let off = m_opac.len() as i32;
            for &key in keys {
                let mut delta = vec![0.0f32; len];
                for (k, w) in ayatsuri_core::blendshape::weights(key, &shape.keys) {
                    let start = k.saturating_mul(len);
                    if w > 0.0
                        && let Some(form) = shape.positions.get(start..start + len)
                    {
                        ayatsuri_core::blendshape::add(&mut delta, form, &base, w);
                    }
                }
                m_opac.push(0.0);
                m_col.push(([0.0; 3], [0.0; 3]));
                m_orders.push(0.0);
                m_pos_begin.push(coords.len() as i32);
                coords.extend_from_slice(&delta);
            }
            bs_bindings.push((t as i32, off, keys.len() as i32));
            bs_limit_runs.push((lim.index.len() as i32, shape.limits.len() as i32));
            for l in &shape.limits {
                if l.parameter as usize >= n_params || l.keys.len() != l.weights.len() {
                    return Err(err(format!(
                        "art mesh {mi}: a blend shape weight limit is broken"
                    )));
                }
                lim.index.push(lim.param.len() as i32);
                lim.param.push(l.parameter as i32);
                lim.begin.push(lim.keys.len() as i32);
                lim.count.push(l.keys.len() as i32);
                lim.keys.extend_from_slice(&l.keys);
                lim.weights.extend_from_slice(&l.weights);
            }
        }
        bs_meshes.push((mi as i32, first, m.blend_shapes.len() as i32));
    }
    for slot in 29..=32usize {
        sect.insert(slot, vec![0u8; n_meshes * 8]);
    }
    sect.insert(33, ids(&model.art_meshes.iter().map(|m| m.id.clone()).collect::<Vec<_>>()));
    sect.insert(34, i32s(&m_kb));
    sect.insert(35, i32s(&m_kf_begin));
    sect.insert(36, i32s(&m_kf_count));
    sect.insert(37, bools(&model.art_meshes.iter().map(|m| m.visible).collect::<Vec<_>>()));
    sect.insert(38, bools(&model.art_meshes.iter().map(|m| m.enabled).collect::<Vec<_>>()));
    sect.insert(
        39,
        i32s(
            &model
                .art_meshes
                .iter()
                .map(|m| m.parent_part.map(|v| v as i32).unwrap_or(-1))
                .collect::<Vec<_>>(),
        ),
    );
    sect.insert(
        40,
        i32s(
            &model
                .art_meshes
                .iter()
                .map(|m| m.parent_deformer.map(|v| v as i32).unwrap_or(-1))
                .collect::<Vec<_>>(),
        ),
    );
    sect.insert(41, i32s(&model.art_meshes.iter().map(|m| m.texture as i32).collect::<Vec<_>>()));
    // blend bits: the closest of normal, add and multiply, which is all a
    // moc3 before 5.3 has (5.3 also reads them, beside its own codes)
    let blend_bits = flags::BLEND_ADDITIVE | flags::BLEND_MULTIPLICATIVE;
    sect.insert(
        42,
        model
            .art_meshes
            .iter()
            .map(|m| (m.flags & !blend_bits) | Blend::of(m.blend, m.flags).legacy_bits().0)
            .collect(),
    );
    sect.insert(
        43,
        i32s(&model.art_meshes.iter().map(|m| m.vertex_count as i32).collect::<Vec<_>>()),
    );
    sect.insert(44, i32s(&m_uv_begin));
    sect.insert(45, i32s(&m_idx_begin));
    sect.insert(46, i32s(&m_idx_count));
    sect.insert(47, i32s(&m_mask_begin));
    sect.insert(48, i32s(&m_mask_count));
    sect.insert(68, f32s(&m_opac));
    sect.insert(69, f32s(&m_orders));
    sect.insert(70, i32s(&m_pos_begin));
    sect.insert(78, f32s(&uv_pool));
    sect.insert(79, i16s(&tri_pool));
    // the draw order groups to write: the model's own, or for a model built
    // from scratch the ones its offscreen parts call for (runtimes want at
    // least the root group, and an offscreen part must be a part item with
    // a group of its own)
    let synthesized;
    let groups: &[ayatsuri_format::model::DrawGroup] = if model.draw_groups.is_empty() {
        synthesized = ayatsuri_core::draworder::synthesize(model);
        &synthesized
    } else {
        &model.draw_groups
    };
    let grouped: std::collections::HashSet<u32> = groups
        .iter()
        .flat_map(|g| &g.items)
        .filter(|it| it.part && it.child_group.is_some())
        .map(|it| it.object)
        .collect();

    // ---- offscreen parts (5.3): one offscreen per part that has it on,
    // its masks joining the drawable mask pool after the meshes' and one
    // offscreen keyform per part keyform ----
    let mut part_off = vec![-1i32; n_parts];
    let mut part_kf_off: Vec<i32> = Vec::new();
    let (mut off_owner, mut off_flags, mut off_blend) = (Vec::new(), Vec::new(), Vec::new());
    let (mut off_mask_begin, mut off_mask_count) = (Vec::new(), Vec::new());
    let mut off_opac: Vec<f32> = Vec::new();
    if version.max_slot() >= 152 {
        for (i, p) in model.parts.iter().enumerate() {
            // a part the draw order groups don't hold as a group draws directly
            let Some(o) = p.offscreen.as_ref().filter(|_| grouped.contains(&(i as u32))) else {
                part_kf_off.extend(std::iter::repeat_n(-1, p.draw_orders.len()));
                continue;
            };
            if o.opacities.len() != p.draw_orders.len() {
                return Err(err(format!(
                    "part {i}: {} offscreen opacities for {} keyforms",
                    o.opacities.len(),
                    p.draw_orders.len()
                )));
            }
            if let Some(slot) = part_off.get_mut(i) {
                *slot = off_owner.len() as i32;
            }
            off_owner.push(i as i32);
            // the flags byte is read as a drawable's: only the mask bit
            off_flags.push(if o.inverted { flags::INVERTED_MASK } else { 0 });
            off_blend.push(o.blend.to_moc3());
            off_mask_begin.push(mask_pool.len() as i32);
            off_mask_count.push(o.masks.len() as i32);
            mask_pool.extend(o.masks.iter().map(|&s| s as i32));
            for &a in &o.opacities {
                part_kf_off.push(off_opac.len() as i32);
                off_opac.push(a);
            }
        }
    }
    sect.insert(80, i32s(&mask_pool));

    // ---- glue (89-100) ----
    let mut g_kb = Vec::new();
    let mut g_kf_begin = Vec::new();
    let mut g_kf_count = Vec::new();
    let mut g_info_begin = Vec::new();
    let mut g_info_count = Vec::new();
    let mut g_weights: Vec<f32> = Vec::new();
    let mut g_pos: Vec<i16> = Vec::new();
    let mut g_intens: Vec<f32> = Vec::new();
    for (i, g) in model.glues.iter().enumerate() {
        let expected = g.binding.keyform_count();
        if g.intensities.len() != expected || g.weights.len() != g.indices.len() {
            return Err(err(format!("glue {i}: arrays disagree")));
        }
        g_kb.push(bind.keyform_binding(&g.binding)?);
        g_kf_begin.push(g_intens.len() as i32);
        g_kf_count.push(expected as i32);
        g_intens.extend_from_slice(&g.intensities);
        g_info_begin.push(g_weights.len() as i32);
        g_info_count.push(g.weights.len() as i32);
        g_weights.extend_from_slice(&g.weights);
        g_pos.extend(g.indices.iter().map(|&v| v as i16));
    }
    sect.insert(89, vec![0u8; n_glue * 8]);
    sect.insert(90, ids(&model.glues.iter().map(|g| g.id.clone()).collect::<Vec<_>>()));
    sect.insert(91, i32s(&g_kb));
    sect.insert(92, i32s(&g_kf_begin));
    sect.insert(93, i32s(&g_kf_count));
    sect.insert(94, i32s(&model.glues.iter().map(|g| g.art_mesh_a as i32).collect::<Vec<_>>()));
    sect.insert(95, i32s(&model.glues.iter().map(|g| g.art_mesh_b as i32).collect::<Vec<_>>()));
    sect.insert(96, i32s(&g_info_begin));
    sect.insert(97, i32s(&g_info_count));
    sect.insert(98, f32s(&g_weights));
    sect.insert(99, i16s(&g_pos));
    sect.insert(100, f32s(&g_intens));

    // ---- draw order groups (81-88) ----
    let mut dg_begin = Vec::new();
    let mut dg_count = Vec::new();
    let mut dg_total = Vec::new();
    let mut dg_max = Vec::new();
    let mut dg_min = Vec::new();
    let mut do_types = Vec::new();
    let mut do_idx = Vec::new();
    let mut do_self = Vec::new();
    // totals counted the runtime's way: an offscreen written here takes a
    // layer slot, one dropped (or a version before 5.3) does not
    let totals = ayatsuri_core::draworder::totals(groups, |p| {
        part_off.get(p as usize).is_some_and(|&o| o >= 0)
    });
    for (g, &total) in groups.iter().zip(&totals) {
        dg_begin.push(do_types.len() as i32);
        dg_count.push(g.items.len() as i32);
        dg_total.push(total as i32);
        dg_max.push(g.max_order);
        dg_min.push(g.min_order);
        for it in &g.items {
            do_types.push(it.part as i32);
            do_idx.push(it.object as i32);
            do_self.push(it.child_group.map(|v| v as i32).unwrap_or(-1));
        }
    }
    sect.insert(81, i32s(&dg_begin));
    sect.insert(82, i32s(&dg_count));
    sect.insert(83, i32s(&dg_total));
    sect.insert(84, i32s(&dg_max));
    sect.insert(85, i32s(&dg_min));
    sect.insert(86, i32s(&do_types));
    sect.insert(87, i32s(&do_idx));
    sect.insert(88, i32s(&do_self));

    // ---- binding pools (71-77) ----
    sect.insert(71, f32s(&coords));
    sect.insert(72, i32s(&bind.pbi.clone()));
    sect.insert(73, i32s(&bind.kb_begin.clone()));
    sect.insert(74, i32s(&bind.kb_count.clone()));
    sect.insert(75, i32s(&bind.key_begin.clone()));
    sect.insert(76, i32s(&bind.key_count.clone()));

    // ---- v4.2 extensions (102-116) ----
    if version.max_slot() >= 102 {
        // each parameter's own sorted key list rides at the end of the keys
        // pool, which is what the extension tables point at in real files
        let mut ext_begin = Vec::with_capacity(n_params);
        let mut ext_count = Vec::with_capacity(n_params);
        for (p, meta) in model.parameters.iter().enumerate() {
            let mut all: Vec<f32> = Vec::new();
            let mut see = |b: &Binding| {
                for a in &b.axes {
                    if a.parameter as usize == p {
                        all.extend_from_slice(&a.keys);
                    }
                }
            };
            for x in &model.parts {
                see(&x.binding);
            }
            for x in &model.deformers {
                see(&x.binding);
            }
            for x in &model.art_meshes {
                see(&x.binding);
            }
            for x in &model.glues {
                see(&x.binding);
            }
            if let Some((_, keys)) = bs_tables.iter().find(|(q, _)| *q as usize == p) {
                all.extend_from_slice(keys);
            }
            if all.is_empty() {
                all.push(meta.min);
                all.push(meta.max);
            }
            all.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            all.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
            ext_begin.push(bind.keys.len() as i32);
            ext_count.push(all.len() as i32);
            bind.keys.extend_from_slice(&all);
        }
        sect.insert(102, vec![0u8; n_params * 8]);
        sect.insert(103, i32s(&ext_begin));
        sect.insert(104, i32s(&ext_count));
        // v4.2 cores expect a full neutral color pool: one multiply and one
        // screen color per keyform, warps then rotations then meshes, with
        // the per-object begins pointing at their keyform runs
        let w_total = w_opac.len() as i32;
        let r_total = r_opac.len() as i32;
        // offscreen keyforms take neutral colours after the meshes'
        let color_total = w_opac.len() + r_opac.len() + m_opac.len() + off_opac.len();
        sect.insert(105, i32s(&w_kf_begin.clone()));
        sect.insert(106, i32s(&r_kf_begin.iter().map(|b| b + w_total).collect::<Vec<_>>()));
        sect.insert(
            107,
            i32s(&m_kf_begin.iter().map(|b| b + w_total + r_total).collect::<Vec<_>>()),
        );
        // each keyform's own multiply (108-110 RGB) and screen (111-113);
        // neutral is a white multiply and a black screen (a white screen
        // lifts every pixel to white: a silhouette). A blend shape's
        // keyforms hold zeros, as their colours are added. Offscreen
        // keyforms take neutral colours after the meshes'.
        let neutral: KeyColor = ([1.0; 3], [0.0; 3]);
        let pool: Vec<KeyColor> = w_col
            .iter()
            .chain(&r_col)
            .chain(&m_col)
            .copied()
            .chain(core::iter::repeat_n(neutral, off_opac.len()))
            .collect();
        if pool.len() != color_total {
            return Err(err("keyform colours disagree with the keyforms"));
        }
        for (c, slot) in (108..=110usize).enumerate() {
            sect.insert(
                slot,
                f32s(&pool.iter().map(|k| k.0.get(c).copied().unwrap_or(1.0)).collect::<Vec<_>>()),
            );
        }
        for (c, slot) in (111..=113usize).enumerate() {
            sect.insert(
                slot,
                f32s(&pool.iter().map(|k| k.1.get(c).copied().unwrap_or(0.0)).collect::<Vec<_>>()),
            );
        }
        // blend shape parameters: type 1 and their key table
        let table_of = |p: usize| bs_tables.iter().position(|(q, _)| *q as usize == p);
        sect.insert(
            114,
            i32s(&(0..n_params).map(|p| table_of(p).map_or(0, |_| 1)).collect::<Vec<_>>()),
        );
        sect.insert(
            115,
            i32s(&(0..n_params).map(|p| table_of(p).map_or(-1, |t| t as i32)).collect::<Vec<_>>()),
        );
        sect.insert(
            116,
            i32s(&(0..n_params).map(|p| table_of(p).map_or(0, |_| 1)).collect::<Vec<_>>()),
        );
        if !bs_tables.is_empty() {
            let (mut kb, mut kn, mut base) = (Vec::new(), Vec::new(), Vec::new());
            for (_, keys) in &bs_tables {
                kb.push(bind.keys.len() as i32);
                kn.push(keys.len() as i32);
                base.push(keys.iter().position(|k| k.abs() < 1e-6).unwrap_or(0) as i32);
                bind.keys.extend_from_slice(keys);
            }
            sect.insert(117, i32s(&kb));
            sect.insert(118, i32s(&kn));
            sect.insert(119, i32s(&base));
            sect.insert(120, i32s(&bs_bindings.iter().map(|b| b.0).collect::<Vec<_>>()));
            sect.insert(121, i32s(&bs_bindings.iter().map(|b| b.1).collect::<Vec<_>>()));
            sect.insert(122, i32s(&bs_bindings.iter().map(|b| b.2).collect::<Vec<_>>()));
            sect.insert(123, i32s(&bs_limit_runs.iter().map(|r| r.0).collect::<Vec<_>>()));
            sect.insert(124, i32s(&bs_limit_runs.iter().map(|r| r.1).collect::<Vec<_>>()));
            sect.insert(131, i32s(&lim.index));
            sect.insert(132, i32s(&lim.param));
            sect.insert(133, i32s(&lim.begin));
            sect.insert(134, i32s(&lim.count));
            sect.insert(135, f32s(&lim.keys));
            sect.insert(136, f32s(&lim.weights));
            sect.insert(128, i32s(&bs_meshes.iter().map(|b| b.0).collect::<Vec<_>>()));
            sect.insert(129, i32s(&bs_meshes.iter().map(|b| b.1).collect::<Vec<_>>()));
            sect.insert(130, i32s(&bs_meshes.iter().map(|b| b.2).collect::<Vec<_>>()));
        }
    }
    sect.insert(77, f32s(&bind.keys.clone()));

    // ---- v5.0 extensions (137-151) ----
    // per-keyform colour links into the multiply / screen pools, which are
    // laid out warps then rotations then meshes, one entry per keyform;
    // blend shapes on parts, rotations and glue stay empty
    if version.max_slot() >= 137 {
        let w_total = w_opac.len() as i32;
        let r_total = r_opac.len() as i32;
        let w_idx: Vec<i32> = (0..w_total).collect();
        let r_idx: Vec<i32> = (0..r_total).map(|k| w_total + k).collect();
        let m_idx: Vec<i32> = (0..m_opac.len() as i32).map(|k| w_total + r_total + k).collect();
        sect.insert(137, i32s(&w_idx));
        sect.insert(138, i32s(&w_idx));
        sect.insert(139, i32s(&r_idx));
        sect.insert(140, i32s(&r_idx));
        sect.insert(141, i32s(&m_idx));
        sect.insert(142, i32s(&m_idx));
    }

    // ---- v5.3 extensions (152-166) ----
    // no offscreens: every part says so (-1), every part keyform points at
    // no offscreen keyform; each mesh's colour and alpha blend in one word
    // (colour in the low byte)
    if version.max_slot() >= 152 {
        let modes: Vec<i32> =
            model.art_meshes.iter().map(|m| Blend::of(m.blend, m.flags).to_moc3()).collect();
        sect.insert(153, i32s(&modes));
        sect.insert(152, i32s(&part_off));
        sect.insert(154, vec![0u8; off_owner.len() * 8]);
        sect.insert(155, i32s(&off_owner));
        sect.insert(156, off_flags.clone());
        sect.insert(157, i32s(&off_blend));
        sect.insert(158, i32s(&off_mask_begin));
        sect.insert(159, i32s(&off_mask_count));
        sect.insert(160, i32s(&part_kf_off));
        sect.insert(161, f32s(&off_opac));
        let first_color = (w_opac.len() + r_opac.len() + m_opac.len()) as i32;
        let off_colors: Vec<i32> = (0..off_opac.len() as i32).map(|k| first_color + k).collect();
        sect.insert(162, i32s(&off_colors));
        sect.insert(163, i32s(&off_colors));
    }

    // ---- counts table (slot 0), field order is the on-disk order ----
    let counts: [i32; 32] = [
        n_parts as i32,
        n_deformers as i32,
        n_warps as i32,
        n_rots as i32,
        n_meshes as i32,
        n_params as i32,
        part_keyforms as i32,
        w_opac.len() as i32,
        r_opac.len() as i32,
        m_opac.len() as i32,
        coords.len() as i32,
        bind.pbi.len() as i32,
        bind.kb_begin.len() as i32,
        bind.key_begin.len() as i32,
        bind.keys.len() as i32,
        uv_pool.len() as i32,
        tri_pool.len() as i32,
        mask_pool.len() as i32,
        groups.len() as i32,
        do_types.len() as i32,
        n_glue as i32,
        g_weights.len() as i32,
        g_intens.len() as i32,
        if version.max_slot() >= 102 {
            (w_opac.len() + r_opac.len() + m_opac.len() + off_opac.len()) as i32
        } else {
            0
        },
        if version.max_slot() >= 102 {
            (w_opac.len() + r_opac.len() + m_opac.len() + off_opac.len()) as i32
        } else {
            0
        },
        bs_tables.len() as i32,
        bs_bindings.len() as i32,
        0,
        bs_meshes.len() as i32,
        lim.index.len() as i32,
        lim.param.len() as i32,
        lim.keys.len() as i32,
    ];
    // 5.0 doubled the table; past 4.2's fields only the offscreen counts
    // (35, 36) are written, as no blend shapes are
    let mut count_bytes = vec![0u8; version.count_ints() * 4];
    let extra = [(35usize, off_owner.len() as i32), (36, off_opac.len() as i32)];
    let extra = extra.into_iter().filter(|_| version.max_slot() >= 152);
    for (i, c) in counts.iter().copied().enumerate().chain(extra) {
        let at = i * 4;
        if let Some(slice) = count_bytes.get_mut(at..at + 4) {
            slice.copy_from_slice(&c.to_le_bytes());
        }
    }
    sect.insert(0, count_bytes);

    // ---- canvas info (slot 1), a 64-byte record ----
    let mut canvas = vec![0u8; 64];
    for (i, v) in [
        model.canvas.pixels_per_unit,
        model.canvas.origin_x,
        model.canvas.origin_y,
        model.canvas.width,
        model.canvas.height,
    ]
    .iter()
    .enumerate()
    {
        if let Some(slice) = canvas.get_mut(i * 4..i * 4 + 4) {
            slice.copy_from_slice(&v.to_le_bytes());
        }
    }
    sect.insert(1, canvas);

    // ---- assembly: header, offset table, sections ascending by slot ----
    let sot_entries = version.sot_entries();
    let mut file = vec![0u8; HEADER + sot_entries * 4];
    if let Some(h) = file.get_mut(0..6) {
        h.copy_from_slice(b"MOC3\0\0");
        if let Some(v) = h.get_mut(4) {
            *v = version.byte();
        }
    }
    let mut offsets = vec![0u32; sot_entries];
    // empty slots after the last populated one point at the file end, as
    // in real files; an empty slot with data still to come shares the next
    // section's offset instead, or offsets would stop rising (5.0 and 5.3
    // put data after 4.2's empty tail, and cores reject that)
    let last_filled = sect.keys().copied().filter(|&k| k <= version.max_slot()).max().unwrap_or(0);
    for (slot, offset) in offsets.iter_mut().enumerate().take(version.max_slot() + 1) {
        let payload = sect.remove(&slot);
        if payload.is_none() && slot >= 117 && slot > last_filled {
            // defined by the version but never populated by this writer;
            // patched to the file length once it is known
            *offset = u32::MAX;
            continue;
        }
        // defined slots the model has no data for still get an offset,
        // shared with whatever comes next, which measured files do too
        while !file.len().is_multiple_of(32) {
            file.push(0);
        }
        *offset = file.len() as u32;
        if let Some(bytes) = payload {
            file.extend_from_slice(&bytes);
        }
    }
    // reference cores require 8-byte alignment on every offset, the
    // end-of-file markers included, and the official Core's consistency
    // check rejects a file whose size is not a multiple of its 64-byte moc
    // alignment ("size" is invalid); every real file measured ends on 64
    while !file.len().is_multiple_of(64) {
        file.push(0);
    }
    let flen = file.len() as u32;
    for o in offsets.iter_mut() {
        if *o == u32::MAX {
            *o = flen;
        }
    }
    for (slot, o) in offsets.iter().enumerate() {
        let at = HEADER + slot * 4;
        if let Some(slice) = file.get_mut(at..at + 4) {
            slice.copy_from_slice(&o.to_le_bytes());
        }
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_version_ends_on_64_bytes() {
        let model = Model::new();
        for v in ExportVersion::ALL {
            let file = to_moc3(&model, v).unwrap();
            // the official Core's consistency check calls anything else an
            // invalid size; measured Cubism files all end on 64
            assert_eq!(file.len() % 64, 0, "{} is {} bytes", v.label(), file.len());
            let first = u32::from_le_bytes(file[HEADER..HEADER + 4].try_into().unwrap()) as usize;
            let table = (first - HEADER) / 4;
            let max = (0..table)
                .map(|i| {
                    let at = HEADER + i * 4;
                    u32::from_le_bytes(file[at..at + 4].try_into().unwrap()) as usize
                })
                .max()
                .unwrap();
            assert!(max <= file.len(), "{}: offset past the end", v.label());
        }
    }

    /// Slot `slot`'s section as f32s, `n` of them.
    fn floats(file: &[u8], slot: usize, n: usize) -> Vec<f32> {
        let at = HEADER + slot * 4;
        let off = u32::from_le_bytes(file[at..at + 4].try_into().unwrap()) as usize;
        (0..n)
            .map(|i| f32::from_le_bytes(file[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
            .collect()
    }

    #[test]
    fn colour_pools_are_neutral() {
        // one mesh with one keyform is enough to get a pool entry
        let mut model = Model::new();
        model.art_meshes.push(ayatsuri_format::model::ArtMesh {
            id: "M".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 3,
            uvs: vec![0.0; 6],
            indices: vec![0, 1, 2],
            masks: vec![],
            binding: Default::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
        });
        for v in ExportVersion::ALL.into_iter().filter(|v| v.max_slot() >= 113) {
            let file = to_moc3(&model, v).unwrap();
            for slot in 108..=110 {
                assert!(floats(&file, slot, 1).iter().all(|&c| c == 1.0), "{} multiply", v.label());
            }
            // a white screen colour turns every pixel white
            for slot in 111..=113 {
                assert!(floats(&file, slot, 1).iter().all(|&c| c == 0.0), "{} screen", v.label());
            }
        }
    }

    #[test]
    fn keyform_colours_round_trip_and_pass_down() {
        use ayatsuri_format::model::{
            ArtMesh, BindingAxis, Deformer, DeformerKind, Parameter, WarpData,
        };
        let mut model = Model::new();
        model.parameters.push(Parameter {
            id: "ParamX".into(),
            min: 0.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
            blend_shape: false,
        });
        let on_x = || Binding {
            axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }],
            curves: Vec::new(),
        };
        // a warp that darkens to half at x = 1, and a mesh in it that
        // screens in blue at x = 1
        let grid = vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        model.deformers.push(Deformer {
            id: "W".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            binding: on_x(),
            kind: DeformerKind::Warp(WarpData {
                rows: 1,
                cols: 1,
                quad: false,
                opacities: vec![1.0, 1.0],
                positions: [grid.clone(), grid].concat(),
            }),
            multiply: vec![1.0, 1.0, 1.0, 0.5, 0.5, 0.5],
            screen: Vec::new(),
        });
        let tri = vec![0.1, 0.1, 0.9, 0.1, 0.1, 0.9];
        model.art_meshes.push(ArtMesh {
            id: "M".into(),
            parent_part: None,
            parent_deformer: Some(0),
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 3,
            uvs: vec![0.0; 6],
            indices: vec![0, 1, 2],
            masks: vec![],
            binding: on_x(),
            opacities: vec![1.0, 1.0],
            draw_orders: vec![500.0, 500.0],
            positions: [tri.clone(), tri].concat(),
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        });
        let color = |m: &Model, x: f32| {
            let mut p = ayatsuri_core::Puppet::new(m.clone());
            p.set_value(0, x);
            p.update();
            p.drawables().next().map(|d| d.color).unwrap_or_default()
        };
        let mid = color(&model, 0.5);
        assert_eq!(mid.multiply, [0.75; 3], "the warp's multiply passes down");
        assert_eq!(mid.screen, [0.0, 0.0, 0.5]);
        for v in [ExportVersion::V4_02, ExportVersion::V5_00, ExportVersion::V5_03] {
            let back = crate::moc3::from_moc3(&to_moc3(&model, v).unwrap()).unwrap();
            assert_eq!(back.deformers[0].multiply, model.deformers[0].multiply, "{v:?}");
            assert!(back.deformers[0].screen.is_empty(), "{v:?}: neutral stays empty");
            assert_eq!(back.art_meshes[0].screen, model.art_meshes[0].screen, "{v:?}");
            assert_eq!(color(&back, 0.5), mid, "{v:?}");
        }
        let w = export_warnings(&model, ExportVersion::V4_00).join(" ");
        assert!(w.contains("2 object(s) use multiply or screen colours"), "{w}");
        assert!(export_warnings(&model, ExportVersion::V4_02).join(" ").find("colours").is_none());
    }

    #[test]
    fn blend_modes_survive_5_3_and_fall_back_before_it() {
        use ayatsuri_format::model::{AlphaBlend, ColorBlend};
        let mesh = |id: &str, color| ayatsuri_format::model::ArtMesh {
            id: id.into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Blend { color, alpha: AlphaBlend::Atop },
            vertex_count: 3,
            uvs: vec![0.0; 6],
            indices: vec![0, 1, 2],
            masks: vec![],
            binding: Default::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
        };
        let mut model = Model::new();
        model.art_meshes.push(mesh("Overlay", ColorBlend::Overlay));
        model.art_meshes.push(mesh("Screen", ColorBlend::Screen));

        let back = crate::moc3::from_moc3(&to_moc3(&model, ExportVersion::V5_03).unwrap()).unwrap();
        let got: Vec<Blend> = back.art_meshes.iter().map(|m| m.blend).collect();
        assert_eq!(got, model.art_meshes.iter().map(|m| m.blend).collect::<Vec<_>>());

        // 5.0 has only normal, add and multiply: overlay draws normal,
        // screen adds, and the export says so
        let back = crate::moc3::from_moc3(&to_moc3(&model, ExportVersion::V5_00).unwrap()).unwrap();
        let m = &back.art_meshes;
        assert_eq!(Blend::of(m[0].blend, m[0].flags), Blend::NORMAL);
        assert_eq!(Blend::of(m[1].blend, m[1].flags).color, ColorBlend::AddCompatible);
        let warnings = export_warnings(&model, ExportVersion::V5_00).join(" ");
        assert!(warnings.contains("2 mesh(es) use a blend mode"), "{warnings}");
    }

    #[test]
    fn blend_shapes_round_trip_and_draw_the_same() {
        use ayatsuri_format::model::{ArtMesh, BindingAxis, BlendShape, Parameter};
        let param = |id: &str, min: f32, blend_shape: bool| Parameter {
            id: id.into(),
            min,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
            blend_shape,
        };
        let mut model = Model::new();
        model.parameters = vec![param("ParamX", 0.0, false), param("ParamSmile", -1.0, true)];
        // a triangle moving right along ParamX; Smile lifts vertex 0 at +1
        // and pushes vertex 1 left at -1, the base key in the middle
        model.art_meshes.push(ArtMesh {
            id: "Mouth".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 3,
            uvs: vec![0.0; 6],
            indices: vec![0, 1, 2],
            masks: vec![],
            clip_group: None,
            clip_inherit: false,
            binding: Binding {
                axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }],
                curves: Vec::new(),
            },
            opacities: vec![1.0, 1.0],
            draw_orders: vec![500.0, 500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 2.0, 0.0, 3.0, 0.0, 2.0, 1.0],
            blend_shapes: vec![BlendShape {
                parameter: 1,
                keys: vec![-1.0, 1.0],
                positions: vec![0.0, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0, 2.0, 1.0, 0.0, 0.0, 1.0],
                // ParamX past the middle fades the smile out
                limits: vec![ayatsuri_format::model::WeightLimit {
                    parameter: 0,
                    keys: vec![0.0, 0.5, 1.0],
                    weights: vec![1.0, 1.0, 0.0],
                }],
            }],
            multiply: Vec::new(),
            screen: Vec::new(),
        });
        let drawn = |m: &Model, x: f32, smile: f32| {
            let mut p = ayatsuri_core::Puppet::new(m.clone());
            p.set_value(0, x);
            p.set_value(1, smile);
            p.update();
            p.drawables().next().map(|d| d.positions.to_vec()).unwrap_or_default()
        };
        for v in [ExportVersion::V4_02, ExportVersion::V5_00, ExportVersion::V5_03] {
            let back = crate::moc3::from_moc3(&to_moc3(&model, v).unwrap()).unwrap();
            assert!(back.parameters[1].blend_shape && !back.parameters[0].blend_shape, "{v:?}");
            let shape = &back.art_meshes[0].blend_shapes[0];
            assert_eq!(shape.keys, vec![-1.0, 1.0], "{v:?}");
            assert_eq!(shape.limits, model.art_meshes[0].blend_shapes[0].limits, "{v:?}");
            for (x, smile) in [
                (0.0, 0.0),
                (0.0, 1.0),
                (0.5, 0.5),
                (1.0, -1.0),
                (0.3, -0.25),
                (1.0, 1.0),
                (0.75, 1.0),
            ] {
                let (want, got) = (drawn(&model, x, smile), drawn(&back, x, smile));
                assert_eq!(want.len(), got.len());
                for (a, b) in want.iter().zip(&got) {
                    assert!(
                        (a - b).abs() < 1e-5,
                        "{v:?} at x {x} smile {smile}: {want:?} vs {got:?}"
                    );
                }
            }
            assert!(
                !export_warnings(&model, v).iter().any(|w| w.contains("blend shapes")),
                "{v:?}"
            );
        }
        // 4.0 has no blend shapes: the shape goes, and the export says so
        let back = crate::moc3::from_moc3(&to_moc3(&model, ExportVersion::V4_00).unwrap()).unwrap();
        assert!(back.art_meshes[0].blend_shapes.is_empty());
        let warnings = export_warnings(&model, ExportVersion::V4_00).join(" ");
        assert!(warnings.contains("need moc3 4.2"), "{warnings}");
    }

    #[test]
    fn offscreen_parts_round_trip_through_5_3() {
        use ayatsuri_format::model::{AlphaBlend, ArtMesh, Blend, ColorBlend, Offscreen, Part};
        let mut model = Model::new();
        model.parts.push(Part {
            id: "Hair".into(),
            parent_part: None,
            visible: true,
            enabled: true,
            binding: Default::default(),
            draw_orders: vec![500.0],
            offscreen: Some(Offscreen {
                masks: vec![1],
                inverted: true,
                blend: Blend { color: ColorBlend::Multiply, alpha: AlphaBlend::Over },
                opacities: vec![0.4],
            }),
        });
        let mesh = |id: &str, part: Option<u32>| ArtMesh {
            id: id.into(),
            parent_part: part,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 3,
            uvs: vec![0.0; 6],
            indices: vec![0, 1, 2],
            masks: vec![],
            binding: Default::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
        };
        model.art_meshes.push(mesh("Strand", Some(0)));
        model.art_meshes.push(mesh("Head", None));

        let back = crate::moc3::from_moc3(&to_moc3(&model, ExportVersion::V5_03).unwrap()).unwrap();
        assert_eq!(back.parts[0].offscreen, model.parts[0].offscreen);
        // the part is a part item with a group of its own, and the root's
        // total counts both meshes and the layer's slot, as runtimes walk it
        let root = &back.draw_groups[0];
        assert!(root.items.iter().any(|it| it.part && it.object == 0 && it.child_group.is_some()));
        assert_eq!(root.total, 3);
        // read back, every mesh still finds its place in the render order
        let mut puppet = ayatsuri_core::Puppet::new(back.clone());
        puppet.update();
        assert_eq!(puppet.drawables().count(), 2);
        assert_eq!(puppet.layers_of(0), &[0]);
        // the meshes' own masks are untouched by the layer's
        assert!(back.art_meshes.iter().all(|m| m.masks.is_empty()));

        // before 5.3 the layer goes, and the export says so
        let back = crate::moc3::from_moc3(&to_moc3(&model, ExportVersion::V5_00).unwrap()).unwrap();
        assert!(back.parts[0].offscreen.is_none());
        // no layer slots before 5.3: the root counts the two meshes only
        assert_eq!(back.draw_groups[0].total, 2);
        let warnings = export_warnings(&model, ExportVersion::V5_00).join(" ");
        assert!(warnings.contains("1 part(s) use offscreen drawing"), "{warnings}");
    }

    #[test]
    fn a_skinned_strip_survives_moc3() {
        use ayatsuri_format::model::{ArtMesh, DrawGroup, DrawItem};
        let mut model = Model::new();
        let cols = 6u16;
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        for c in 0..=cols {
            for r in 0..2u16 {
                positions.extend_from_slice(&[f32::from(c) * 0.1, f32::from(r) * 0.05]);
                uvs.extend_from_slice(&[f32::from(c) / f32::from(cols), f32::from(r)]);
            }
        }
        let indices: Vec<u16> = (0..cols)
            .flat_map(|c| [c * 2, c * 2 + 1, c * 2 + 2, c * 2 + 1, c * 2 + 3, c * 2 + 2])
            .collect();
        model.art_meshes.push(ArtMesh {
            id: "Hair".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: u32::from(cols + 1) * 2,
            uvs,
            indices,
            masks: vec![],
            binding: Default::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions,
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
        });
        model.draw_groups.push(DrawGroup {
            total: 1,
            min_order: 500,
            max_order: 500,
            items: vec![DrawItem { part: false, object: 0, child_group: None }],
        });
        let skin = ayatsuri_core::skin::skin(
            &mut model,
            0,
            &[[0.0, 0.025], [0.2, 0.025], [0.4, 0.025], [0.6, 0.025]],
            45.0,
            "ParamHairSkin",
        )
        .unwrap();
        let back = crate::moc3::from_moc3(&to_moc3(&model, ExportVersion::V5_00).unwrap()).unwrap();
        assert_eq!(back.art_meshes.len(), 4);
        assert_eq!(back.deformers.len(), 3);
        assert_eq!(back.glues.len(), 2);
        assert_eq!(back.parameters.len(), skin.params.len());
        assert!(!back.art_meshes[0].visible);
        for w in skin.deformers.windows(2) {
            assert_eq!(back.deformers[w[1]].parent_deformer, Some(w[0] as u32));
        }
    }

    #[test]
    fn default_is_readable_by_common_apps() {
        // 5.3's offset table runs to 0x16C0, past what OpenVT will skip
        assert!(ExportVersion::DEFAULT.sot_entries() * 4 + HEADER <= 0x2a0 + 0x1000);
        assert_eq!(ExportVersion::ALL[ExportVersion::default_index()], ExportVersion::V5_00);
    }
}
