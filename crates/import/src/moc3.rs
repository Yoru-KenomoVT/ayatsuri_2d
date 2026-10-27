//! `.moc3` to [`Model`]. Slot numbers refer to
//! `ayatsuri2d-research/docs/15-moc3-v4-spec.md` section 4; runtime semantics
//! follow PurismCore (MIT), see docs/provenance.md.

use ayatsuri_format::model::{
    ArtMesh, Binding, BindingAxis, Canvas, Deformer, DeformerKind, DrawGroup, DrawItem, Glue,
    Model, Parameter, Part, RotationData, WarpData, WeightLimit,
};
use openmoc3::{F32s, I32s, Moc3};

use crate::ImportError;

/// `-1` means none; anything else must index into `limit`.
fn opt_idx(
    what: &'static str,
    at: usize,
    value: i32,
    limit: usize,
) -> Result<Option<u32>, ImportError> {
    match value {
        -1 => Ok(None),
        v if v >= 0 && (v as usize) < limit => Ok(Some(v as u32)),
        v => Err(ImportError::BadIndex { what, at, value: v as i64, limit }),
    }
}

/// A begin/count pair into a pool, validated once.
fn span(
    what: &'static str,
    at: usize,
    begin: i32,
    len: i32,
    pool: usize,
) -> Result<core::ops::Range<usize>, ImportError> {
    let err = || ImportError::BadSpan { what, at, begin: begin as i64, len: len as i64, pool };
    // a begin of -1 with nothing in it is the format's way of saying "none"
    if begin == -1 && len <= 0 {
        return Ok(0..0);
    }
    if begin < 0 || len < 0 {
        return Err(err());
    }
    let (b, l) = (begin as usize, len as usize);
    let end = b.checked_add(l).ok_or_else(err)?;
    if end > pool { Err(err()) } else { Ok(b..end) }
}

fn collect_f32(pool: &F32s<'_>, range: core::ops::Range<usize>) -> Vec<f32> {
    range.filter_map(|i| pool.get(i)).collect()
}

/// moc3 4.2+'s multiply and screen colours per keyform: pools of rgb
/// (108-113), reached per keyform through 5.0's own indices (137-142) or,
/// before that, each object's first entry (105-107) and its keyform number.
struct KeyColors<'a> {
    multiply: [Option<F32s<'a>>; 3],
    screen: [Option<F32s<'a>>; 3],
    /// Warps, rotations, meshes.
    begin: [Option<I32s<'a>>; 3],
    keyform_multiply: [Option<I32s<'a>>; 3],
    keyform_screen: [Option<I32s<'a>>; 3],
}

#[derive(Clone, Copy)]
enum ColorOwner {
    Warp = 0,
    Rotation = 1,
    Mesh = 2,
}

impl<'a> KeyColors<'a> {
    fn new(moc: &Moc3<'a>) -> Self {
        let f = |s: usize| moc.f32s(s).ok();
        let i = |s: usize| moc.i32s(s).ok();
        KeyColors {
            multiply: [f(108), f(109), f(110)],
            screen: [f(111), f(112), f(113)],
            begin: [i(105), i(106), i(107)],
            keyform_multiply: [i(137), i(139), i(141)],
            keyform_screen: [i(138), i(140), i(142)],
        }
    }

    /// One object's keyforms' colours (`keyforms` are its global keyform
    /// indices, `object` its index among its kind): three floats per
    /// keyform, or empty when every one is neutral or the file has none.
    fn of(
        &self,
        owner: ColorOwner,
        object: usize,
        keyforms: core::ops::Range<usize>,
    ) -> (Vec<f32>, Vec<f32>) {
        let o = owner as usize;
        let at = |per_keyform: &[Option<I32s<'a>>; 3], g: usize, k: usize| -> Option<usize> {
            let idx = match per_keyform.get(o).and_then(Option::as_ref) {
                Some(t) => t.get(g)?,
                None => self.begin.get(o)?.as_ref()?.get(object)?.checked_add(k as i32)?,
            };
            usize::try_from(idx).ok()
        };
        let rgb = |pools: &[Option<F32s<'a>>; 3], i: Option<usize>, neutral: f32| -> [f32; 3] {
            let c = |p: &Option<F32s<'a>>| {
                i.and_then(|i| p.as_ref().and_then(|p| p.get(i))).unwrap_or(neutral)
            };
            [c(&pools[0]), c(&pools[1]), c(&pools[2])]
        };
        let (mut mul, mut scr) = (Vec::new(), Vec::new());
        for (k, g) in keyforms.enumerate() {
            mul.extend(rgb(&self.multiply, at(&self.keyform_multiply, g, k), 1.0));
            scr.extend(rgb(&self.screen, at(&self.keyform_screen, g, k), 0.0));
        }
        if mul.iter().all(|&v| v == 1.0) {
            mul.clear();
        }
        if scr.iter().all(|&v| v == 0.0) {
            scr.clear();
        }
        (mul, scr)
    }
}

/// The per-parameter axis tables, resolved once.
struct Axes {
    /// Per parameterBinding: owning parameter and its key values.
    axes: Vec<Option<BindingAxis>>,
    /// keyformBindings begin/count into the parameterBindingIndices pool.
    kb_begin: Vec<i32>,
    kb_count: Vec<i32>,
    /// parameterBindingIndices: entries index parameterBindings.
    pbi: Vec<i32>,
}

impl Axes {
    fn build(moc: &Moc3<'_>, param_count: usize) -> Result<Self, ImportError> {
        let c = moc.counts();
        let pb_count = c.parameter_bindings.max(0) as usize;

        let p_begin = moc.i32s(56)?;
        let p_count = moc.i32s(57)?;
        let key_begin = moc.i32s(75)?;
        let key_count = moc.i32s(76)?;
        let keys = moc.f32s(77)?;

        let mut axes: Vec<Option<BindingAxis>> = vec![None; pb_count];
        for p in 0..param_count {
            let range = span(
                "parameter bindings of parameter",
                p,
                p_begin.get(p).unwrap_or(-1),
                p_count.get(p).unwrap_or(0),
                pb_count,
            )?;
            for pb in range {
                let krange = span(
                    "keys of parameter binding",
                    pb,
                    key_begin.get(pb).unwrap_or(-1),
                    key_count.get(pb).unwrap_or(0),
                    keys.len(),
                )?;
                *axes.get_mut(pb).ok_or(ImportError::UnownedBinding(pb))? =
                    Some(BindingAxis { parameter: p as u32, keys: collect_f32(&keys, krange) });
            }
        }

        Ok(Self {
            axes,
            kb_begin: moc.i32s(73)?.iter().collect(),
            kb_count: moc.i32s(74)?.iter().collect(),
            pbi: moc.i32s(72)?.iter().collect(),
        })
    }

    /// The binding of an object, from its keyform binding sources index.
    fn binding(
        &self,
        what: &'static str,
        at: usize,
        kb_index: i32,
    ) -> Result<Binding, ImportError> {
        let Some(kbi) = opt_idx(what, at, kb_index, self.kb_begin.len())? else {
            return Ok(Binding::default());
        };
        let kbi = kbi as usize;
        let range = span(
            "parameter binding indices of keyform binding",
            kbi,
            self.kb_begin.get(kbi).copied().unwrap_or(-1),
            self.kb_count.get(kbi).copied().unwrap_or(0),
            self.pbi.len(),
        )?;
        let mut binding = Binding::default();
        for i in range {
            let pb = self.pbi.get(i).copied().unwrap_or(-1);
            let pb = opt_idx("parameter binding index", i, pb, self.axes.len())?
                .ok_or(ImportError::UnownedBinding(i))? as usize;
            let axis =
                self.axes.get(pb).and_then(|a| a.clone()).ok_or(ImportError::UnownedBinding(pb))?;
            binding.axes.push(axis);
        }
        Ok(binding)
    }
}

/// An object's keyform span, which must agree with its binding's product.
fn keyform_span(
    what: &'static str,
    at: usize,
    binding: &Binding,
    begin: i32,
    count: i32,
    pool: usize,
) -> Result<core::ops::Range<usize>, ImportError> {
    let range = span(what, at, begin, count, pool)?;
    let expected = binding.keyform_count();
    if range.len() != expected {
        return Err(ImportError::KeyformMismatch { what, at, expected, got: range.len() });
    }
    Ok(range)
}

/// Keyform-major positions: per keyform, `floats` consecutive f32 from the
/// coordinates pool (slot 71), starting at that keyform's begin index.
fn gather_positions(
    what: &'static str,
    at: usize,
    keyforms: core::ops::Range<usize>,
    begins: &I32s<'_>,
    pool: &F32s<'_>,
    floats: usize,
) -> Result<Vec<f32>, ImportError> {
    let mut out = Vec::with_capacity(keyforms.len().saturating_mul(floats));
    for k in keyforms {
        let begin = begins.get(k).unwrap_or(-1);
        let range = span(what, at, begin, floats as i32, pool.len())?;
        out.extend(range.filter_map(|i| pool.get(i)));
    }
    Ok(out)
}

/// Import a parsed-and-validated `.moc3` as a puppet model.
pub fn from_moc3(bytes: &[u8]) -> Result<Model, ImportError> {
    let moc = Moc3::parse(bytes)?;
    let c = *moc.counts();
    let n_params = c.parameters.max(0) as usize;
    let n_parts = c.parts.max(0) as usize;
    let n_deformers = c.deformers.max(0) as usize;
    let n_meshes = c.art_meshes.max(0) as usize;

    let ci = moc.canvas_info()?;
    let canvas = Canvas {
        pixels_per_unit: ci.pixels_per_unit,
        origin_x: ci.origin_x,
        origin_y: ci.origin_y,
        width: ci.width,
        height: ci.height,
    };

    // ---- parameters (slots 50-57) ----
    let mins = moc.f32s(52)?;
    let maxs = moc.f32s(51)?;
    let defaults = moc.f32s(53)?;
    let repeats = moc.i32s(54)?;
    let decimals = moc.i32s(55)?;
    let parameters: Vec<Parameter> = moc
        .ids(openmoc3::slot::PARAMETER_IDS)?
        .enumerate()
        .map(|(i, id)| Parameter {
            id: id.to_string(),
            min: mins.get(i).unwrap_or(0.0),
            max: maxs.get(i).unwrap_or(0.0),
            default: defaults.get(i).unwrap_or(0.0),
            repeat: repeats.get(i).unwrap_or(0) != 0,
            decimals: decimals.get(i).unwrap_or(0),
            blend_shape: false,
        })
        .collect();

    let axes = Axes::build(&moc, n_params)?;
    let coords = moc.f32s(71)?;

    // ---- parts (slots 2-9, 58) ----
    let part_ids: Vec<&str> = moc.ids(openmoc3::slot::PART_IDS)?.collect();
    let part_kb = moc.i32s(4)?;
    let part_kf_begin = moc.i32s(5)?;
    let part_kf_count = moc.i32s(6)?;
    let part_visible = moc.i32s(7)?;
    let part_enabled = moc.i32s(8)?;
    let part_parent = moc.i32s(9)?;
    let part_draw_orders = moc.f32s(58)?;
    let mut parts = Vec::with_capacity(n_parts);
    for (i, id) in part_ids.iter().enumerate() {
        let binding = axes.binding("part binding", i, part_kb.get(i).unwrap_or(-1))?;
        let kf = keyform_span(
            "part keyforms",
            i,
            &binding,
            part_kf_begin.get(i).unwrap_or(-1),
            part_kf_count.get(i).unwrap_or(0),
            part_draw_orders.len(),
        )?;
        parts.push(Part {
            id: (*id).to_string(),
            parent_part: opt_idx("part parent", i, part_parent.get(i).unwrap_or(-1), n_parts)?,
            visible: part_visible.get(i).unwrap_or(1) != 0,
            enabled: part_enabled.get(i).unwrap_or(1) != 0,
            binding,
            draw_orders: collect_f32(&part_draw_orders, kf),
            offscreen: None,
            blend_shapes: Vec::new(),
        });
    }
    read_offscreens(&moc, &mut parts, &part_kf_begin);

    // ---- deformers (slots 10-28, 59-67, 101) ----
    let d_ids: Vec<&str> = moc.ids(openmoc3::slot::DEFORMER_IDS)?.collect();
    let d_kb = moc.i32s(12)?;
    let d_visible = moc.i32s(13)?;
    let d_enabled = moc.i32s(14)?;
    let d_parent_part = moc.i32s(15)?;
    let d_parent = moc.i32s(16)?;
    let d_types = moc.i32s(17)?;
    let d_specific = moc.i32s(18)?;

    let w_kf_begin = moc.i32s(20)?;
    let w_kf_count = moc.i32s(21)?;
    let w_vertex_counts = moc.i32s(22)?;
    let w_rows = moc.i32s(23)?;
    let w_cols = moc.i32s(24)?;
    let w_quad = moc.i32s(101).ok();
    if w_quad.is_none() {
        log::debug!("moc3: no quad transform section; every warp reads as not quad");
    }
    let w_opacities = moc.f32s(59)?;
    let key_colors = KeyColors::new(&moc);
    let w_pos_begin = moc.i32s(60)?;

    let r_kf_begin = moc.i32s(26)?;
    let r_kf_count = moc.i32s(27)?;
    let r_base_angles = moc.f32s(28)?;
    let r_opacities = moc.f32s(61)?;
    let r_angles = moc.f32s(62)?;
    let r_ox = moc.f32s(63)?;
    let r_oy = moc.f32s(64)?;
    let r_scales = moc.f32s(65)?;
    let r_rx = moc.i32s(66)?;
    let r_ry = moc.i32s(67)?;

    let n_warps = c.warp_deformers.max(0) as usize;
    let n_rots = c.rotation_deformers.max(0) as usize;

    let mut deformers = Vec::with_capacity(n_deformers);
    for (i, id) in d_ids.iter().enumerate() {
        let parent_deformer =
            opt_idx("deformer parent", i, d_parent.get(i).unwrap_or(-1), n_deformers)?;
        if let Some(p) = parent_deformer {
            // The runtime walks deformers in index order, parents first.
            if p as usize >= i {
                return Err(ImportError::NotTopological { child: i, parent: p as usize });
            }
        }
        let binding = axes.binding("deformer binding", i, d_kb.get(i).unwrap_or(-1))?;
        let ty = d_types.get(i).unwrap_or(-1);
        let colors;
        let kind = match ty {
            0 => {
                let si =
                    opt_idx("warp specific index", i, d_specific.get(i).unwrap_or(-1), n_warps)?
                        .ok_or(ImportError::BadIndex {
                            what: "warp specific index",
                            at: i,
                            value: -1,
                            limit: n_warps,
                        })? as usize;
                let rows = w_rows.get(si).unwrap_or(0).max(0) as u32;
                let cols = w_cols.get(si).unwrap_or(0).max(0) as u32;
                let grid_floats = w_vertex_counts.get(si).unwrap_or(0).max(0) as usize * 2;
                let kf = keyform_span(
                    "warp keyforms",
                    si,
                    &binding,
                    w_kf_begin.get(si).unwrap_or(-1),
                    w_kf_count.get(si).unwrap_or(0),
                    w_opacities.len(),
                )?;
                colors = key_colors.of(ColorOwner::Warp, si, kf.clone());
                let data = WarpData {
                    rows,
                    cols,
                    quad: w_quad.as_ref().and_then(|q| q.get(si)).unwrap_or(0) != 0,
                    opacities: collect_f32(&w_opacities, kf.clone()),
                    positions: gather_positions(
                        "warp grid",
                        si,
                        kf,
                        &w_pos_begin,
                        &coords,
                        grid_floats,
                    )?,
                };
                DeformerKind::Warp(data)
            }
            1 => {
                let si =
                    opt_idx("rotation specific index", i, d_specific.get(i).unwrap_or(-1), n_rots)?
                        .ok_or(ImportError::BadIndex {
                            what: "rotation specific index",
                            at: i,
                            value: -1,
                            limit: n_rots,
                        })? as usize;
                let kf = keyform_span(
                    "rotation keyforms",
                    si,
                    &binding,
                    r_kf_begin.get(si).unwrap_or(-1),
                    r_kf_count.get(si).unwrap_or(0),
                    r_opacities.len(),
                )?;
                colors = key_colors.of(ColorOwner::Rotation, si, kf.clone());
                let data = RotationData {
                    base_angle: r_base_angles.get(si).unwrap_or(0.0),
                    opacities: collect_f32(&r_opacities, kf.clone()),
                    angles: collect_f32(&r_angles, kf.clone()),
                    origins_x: collect_f32(&r_ox, kf.clone()),
                    origins_y: collect_f32(&r_oy, kf.clone()),
                    scales: collect_f32(&r_scales, kf.clone()),
                    reflect_x: kf.clone().map(|k| r_rx.get(k).unwrap_or(0) != 0).collect(),
                    reflect_y: kf.map(|k| r_ry.get(k).unwrap_or(0) != 0).collect(),
                };
                DeformerKind::Rotation(data)
            }
            v => return Err(ImportError::UnknownDeformerType { at: i, value: v }),
        };
        deformers.push(Deformer {
            id: (*id).to_string(),
            parent_part: opt_idx(
                "deformer parent part",
                i,
                d_parent_part.get(i).unwrap_or(-1),
                n_parts,
            )?,
            parent_deformer,
            visible: d_visible.get(i).unwrap_or(1) != 0,
            enabled: d_enabled.get(i).unwrap_or(1) != 0,
            binding,
            kind,
            multiply: colors.0,
            screen: colors.1,
            blend_shapes: Vec::new(),
        });
    }

    // ---- art meshes (slots 29-48, 68-70, 78-80) ----
    let m_ids: Vec<&str> = moc.ids(openmoc3::slot::ART_MESH_IDS)?.collect();
    let m_kb = moc.i32s(34)?;
    let m_kf_begin = moc.i32s(35)?;
    let m_kf_count = moc.i32s(36)?;
    let m_visible = moc.i32s(37)?;
    let m_enabled = moc.i32s(38)?;
    let m_parent_part = moc.i32s(39)?;
    let m_parent_deformer = moc.i32s(40)?;
    let m_texture = moc.i32s(41)?;
    let m_flags = moc.u8s(42)?;
    // 5.3 gives every mesh a colour and alpha blend of its own
    let m_blend = moc.i32s(153).ok();
    if m_blend.is_none() {
        log::debug!("moc3: no per-mesh blend section (before 5.3)");
    }
    let m_vertex_counts = moc.i32s(43)?;
    let m_uv_begin = moc.i32s(44)?;
    let m_idx_begin = moc.i32s(45)?;
    let m_idx_count = moc.i32s(46)?;
    let m_mask_begin = moc.i32s(47)?;
    let m_mask_count = moc.i32s(48)?;
    let m_opacities = moc.f32s(68)?;
    let m_draw_orders = moc.f32s(69)?;
    let m_pos_begin = moc.i32s(70)?;
    let uvs_pool = moc.f32s(78)?;
    let tri_pool = moc.i16s(79)?;
    let mask_pool = moc.i32s(80)?;

    let mut art_meshes = Vec::with_capacity(n_meshes);
    for (i, id) in m_ids.iter().enumerate() {
        let binding = axes.binding("art mesh binding", i, m_kb.get(i).unwrap_or(-1))?;
        let vertex_count = m_vertex_counts.get(i).unwrap_or(0).max(0) as u32;
        let vc = vertex_count as usize;

        let uv_range = span(
            "art mesh uvs",
            i,
            m_uv_begin.get(i).unwrap_or(-1),
            (vc * 2) as i32,
            uvs_pool.len(),
        )?;
        let uvs = collect_f32(&uvs_pool, uv_range);

        let tri_range = span(
            "art mesh indices",
            i,
            m_idx_begin.get(i).unwrap_or(-1),
            m_idx_count.get(i).unwrap_or(0),
            tri_pool.len(),
        )?;
        let mut indices = Vec::with_capacity(tri_range.len());
        for t in tri_range {
            let v = tri_pool.get(t).unwrap_or(-1);
            if v < 0 || v as u32 >= vertex_count {
                return Err(ImportError::BadIndex {
                    what: "art mesh vertex index",
                    at: i,
                    value: v as i64,
                    limit: vc,
                });
            }
            indices.push(v as u16);
        }

        let mask_range = span(
            "art mesh masks",
            i,
            m_mask_begin.get(i).unwrap_or(-1),
            m_mask_count.get(i).unwrap_or(0),
            mask_pool.len(),
        )?;
        let mut masks = Vec::with_capacity(mask_range.len());
        for mi in mask_range {
            let v = mask_pool.get(mi).unwrap_or(-1);
            if let Some(s) = opt_idx("mask source", i, v, n_meshes)? {
                masks.push(s);
            }
        }

        let kf = keyform_span(
            "art mesh keyforms",
            i,
            &binding,
            m_kf_begin.get(i).unwrap_or(-1),
            m_kf_count.get(i).unwrap_or(0),
            m_opacities.len(),
        )?;
        let (multiply, screen) = key_colors.of(ColorOwner::Mesh, i, kf.clone());
        art_meshes.push(ArtMesh {
            id: (*id).to_string(),
            parent_part: opt_idx(
                "art mesh parent part",
                i,
                m_parent_part.get(i).unwrap_or(-1),
                n_parts,
            )?,
            parent_deformer: opt_idx(
                "art mesh parent deformer",
                i,
                m_parent_deformer.get(i).unwrap_or(-1),
                n_deformers,
            )?,
            visible: m_visible.get(i).unwrap_or(1) != 0,
            enabled: m_enabled.get(i).unwrap_or(1) != 0,
            texture: m_texture.get(i).unwrap_or(0).max(0) as u32,
            flags: m_flags.get(i).copied().unwrap_or(0),
            blend: m_blend
                .as_ref()
                .and_then(|b| b.get(i))
                .map(ayatsuri_format::model::Blend::from_moc3)
                .unwrap_or_default(),
            vertex_count,
            uvs,
            indices,
            masks,
            binding,
            opacities: collect_f32(&m_opacities, kf.clone()),
            draw_orders: collect_f32(&m_draw_orders, kf.clone()),
            positions: gather_positions(
                "art mesh positions",
                i,
                kf,
                &m_pos_begin,
                &coords,
                vc * 2,
            )?,
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply,
            screen,
        });
    }

    // ---- glue (slots 89-100) ----
    let g_ids: Vec<&str> = moc.ids(openmoc3::slot::GLUE_IDS)?.collect();
    let g_kb = moc.i32s(91)?;
    let g_kf_begin = moc.i32s(92)?;
    let g_kf_count = moc.i32s(93)?;
    let g_mesh_a = moc.i32s(94)?;
    let g_mesh_b = moc.i32s(95)?;
    let g_info_begin = moc.i32s(96)?;
    let g_info_count = moc.i32s(97)?;
    let g_weights = moc.f32s(98)?;
    let g_pos_idx = moc.i16s(99)?;
    let g_intensities = moc.f32s(100)?;

    let mut glues = Vec::with_capacity(g_ids.len());
    for (i, id) in g_ids.iter().enumerate() {
        let binding = axes.binding("glue binding", i, g_kb.get(i).unwrap_or(-1))?;
        let kf = keyform_span(
            "glue keyforms",
            i,
            &binding,
            g_kf_begin.get(i).unwrap_or(-1),
            g_kf_count.get(i).unwrap_or(0),
            g_intensities.len(),
        )?;
        let a = opt_idx("glue mesh a", i, g_mesh_a.get(i).unwrap_or(-1), n_meshes)?.ok_or(
            ImportError::BadIndex { what: "glue mesh a", at: i, value: -1, limit: n_meshes },
        )?;
        let b = opt_idx("glue mesh b", i, g_mesh_b.get(i).unwrap_or(-1), n_meshes)?.ok_or(
            ImportError::BadIndex { what: "glue mesh b", at: i, value: -1, limit: n_meshes },
        )?;
        let info = span(
            "glue info",
            i,
            g_info_begin.get(i).unwrap_or(-1),
            g_info_count.get(i).unwrap_or(0),
            g_weights.len().min(g_pos_idx.len()),
        )?;
        let vc_a = art_meshes.get(a as usize).map_or(0, |m| m.vertex_count);
        let vc_b = art_meshes.get(b as usize).map_or(0, |m| m.vertex_count);
        let mut weights = Vec::with_capacity(info.len());
        let mut indices = Vec::with_capacity(info.len());
        for (n, gi) in info.enumerate() {
            let v = g_pos_idx.get(gi).unwrap_or(-1);
            let limit = if n % 2 == 0 { vc_a } else { vc_b };
            if v < 0 || v as u32 >= limit {
                return Err(ImportError::BadIndex {
                    what: "glue vertex index",
                    at: i,
                    value: v as i64,
                    limit: limit as usize,
                });
            }
            weights.push(g_weights.get(gi).unwrap_or(0.0));
            indices.push(v as u16);
        }
        glues.push(Glue {
            id: (*id).to_string(),
            art_mesh_a: a,
            art_mesh_b: b,
            binding,
            intensities: collect_f32(&g_intensities, kf),
            weights,
            indices,
            blend_shapes: Vec::new(),
        });
    }

    // draw order groups (slots 81-88)
    let n_groups = c.draw_order_groups.max(0) as usize;
    let g_begin = moc.i32s(81)?;
    let g_count = moc.i32s(82)?;
    let g_total = moc.i32s(83)?;
    let g_max = moc.i32s(84)?;
    let g_min = moc.i32s(85)?;
    let o_types = moc.i32s(86)?;
    let o_idx = moc.i32s(87)?;
    let o_self = moc.i32s(88)?;
    let n_objects = c.draw_order_group_objects.max(0) as usize;
    let mut draw_groups = Vec::with_capacity(n_groups);
    for gi in 0..n_groups {
        let range = span(
            "draw group objects",
            gi,
            g_begin.get(gi).unwrap_or(-1),
            g_count.get(gi).unwrap_or(0),
            n_objects,
        )?;
        let mut items = Vec::with_capacity(range.len());
        for oi in range {
            let part = o_types.get(oi).unwrap_or(0) == 1;
            let limit = if part { n_parts } else { n_meshes };
            let object =
                opt_idx("draw item object", oi, o_idx.get(oi).unwrap_or(-1), limit)?.ok_or(
                    ImportError::BadIndex { what: "draw item object", at: oi, value: -1, limit },
                )?;
            let child_group = if part {
                opt_idx("draw item child group", oi, o_self.get(oi).unwrap_or(-1), n_groups)?
            } else {
                None
            };
            items.push(DrawItem { part, object, child_group });
        }
        draw_groups.push(DrawGroup {
            total: g_total.get(gi).unwrap_or(0).max(0) as u32,
            min_order: g_min.get(gi).unwrap_or(0),
            max_order: g_max.get(gi).unwrap_or(0),
            items,
        });
    }

    let mut model = Model {
        canvas,
        parameters,
        parts,
        deformers,
        art_meshes,
        glues,
        draw_groups,
        physics: None,
        art_paths: Vec::new(),
        art_path_colors: Vec::new(),
        art_path_page: None,
        clip_groups: Vec::new(),
    };
    read_blend_shapes(&moc, &mut model, &coords, &m_pos_begin);
    Ok(model)
}

/// moc3 4.2+'s blend shapes (slots 114-151): on art meshes and warps
/// (4.2), parts, rotations and glue (5.0). Each binding's keyforms, one per
/// key of its parameter's key table with the base key's among them, hold
/// what the shape adds; the model keeps the shape's form at each non-base
/// key (the object's base plus that) on every channel its kind has, and its
/// weight limits (slots 123-124, 131-136). Offscreen blend shapes (5.3) are
/// not kept yet, and say so.
fn read_blend_shapes(moc: &Moc3<'_>, model: &mut Model, coords: &F32s<'_>, pos_begin: &I32s<'_>) {
    use ayatsuri_core::blendshape::Channel;
    use ayatsuri_core::edit::Target;
    let c = *moc.counts();
    if c.blend_shape_keyform_bindings <= 0 {
        return;
    }
    if c.blend_shapes_offscreens > 0 {
        log::warn!(
            "moc3: {} blend shape(s) on offscreen layers are not kept yet",
            c.blend_shapes_offscreens
        );
    }
    let (Ok(ptype), Ok(table_begin), Ok(table_count)) =
        (moc.i32s(114), moc.i32s(115), moc.i32s(116))
    else {
        return;
    };
    for (p, meta) in model.parameters.iter_mut().enumerate() {
        meta.blend_shape = ptype.get(p) == Some(1) && table_count.get(p).unwrap_or(0) > 0;
    }
    let (Ok(keys_begin), Ok(keys_count), Ok(base_key), Ok(pool)) =
        (moc.i32s(117), moc.i32s(118), moc.i32s(119), moc.f32s(77))
    else {
        return;
    };
    // which parameter each key table belongs to
    let mut table_param = vec![None; c.blend_shape_parameter_bindings.max(0) as usize];
    for p in 0..model.parameters.len() {
        let (b, n) = (table_begin.get(p).unwrap_or(-1), table_count.get(p).unwrap_or(0));
        for t in b.max(0)..b.max(0) + n.max(0) {
            if let Some(slot) = table_param.get_mut(t as usize) {
                *slot = Some(p as u32);
            }
        }
    }
    let (Ok(b_table), Ok(b_off), Ok(b_len)) = (moc.i32s(120), moc.i32s(121), moc.i32s(122)) else {
        return;
    };
    let f = |slot: usize| moc.f32s(slot).ok();
    let i = |slot: usize| moc.i32s(slot).ok();
    // deformer index of each warp and rotation, by the file's own tables
    let (types, specific) = (i(17), i(18));
    let n_deformers = model.deformers.len();
    let deformer_of = |kind: i32, index: usize| -> Option<usize> {
        let (t, sp) = (types.as_ref()?, specific.as_ref()?);
        (0..n_deformers).find(|&d| t.get(d) == Some(kind) && sp.get(d) == Some(index as i32))
    };
    // a keyform's colour: 5.0's own indices, or 4.2's first entry of the
    // object plus how far the keyform is past the object's first keyform
    let (mul, scr) = ([f(108), f(109), f(110)], [f(111), f(112), f(113)]);
    let color_at =
        |own: [usize; 2], begin: usize, object: usize, kf_begin: i32, g: usize, screen: bool| {
            let own_slot = if screen { own[1] } else { own[0] };
            let idx = match i(own_slot) {
                Some(t) => t.get(g),
                None => i(begin).and_then(|b| b.get(object)).map(|b| b + (g as i32 - kf_begin)),
            };
            let pools = if screen { &scr } else { &mul };
            let neutral = if screen { 0.0 } else { 1.0 };
            let at = |p: &Option<F32s<'_>>| {
                idx.and_then(|x| usize::try_from(x).ok())
                    .and_then(|x| p.as_ref().and_then(|p| p.get(x)))
                    .unwrap_or(neutral)
            };
            [at(&pools[0]), at(&pools[1]), at(&pools[2])]
        };
    // per kind: entries (target, binding begin, binding count) and how to
    // read one keyform's delta on a channel
    let kinds: [(usize, i32); 5] = [
        (128, c.blend_shapes_art_meshes),
        (125, c.blend_shapes_warp_deformers),
        (143, c.blend_shapes_parts),
        (146, c.blend_shapes_rotation_deformers),
        (149, c.blend_shapes_glue),
    ];
    let (w_pos, w_opac, w_kf) = (i(60), f(59), i(20));
    let (r_opac, r_ang, r_ox, r_oy, r_sc, r_kf) = (f(61), f(62), f(63), f(64), f(65), i(26));
    let (m_opac, m_kf, part_orders, glue_int) = (f(68), i(35), f(58), f(100));
    for (slot, count) in kinds {
        let (Some(target), Some(bind_begin), Some(bind_count)) =
            (i(slot), i(slot + 1), i(slot + 2))
        else {
            continue;
        };
        for e in 0..count.max(0) as usize {
            let Some(index) = target.get(e).and_then(|t| usize::try_from(t).ok()) else { continue };
            // the object, and its channels: name, floats a key, reader
            let obj: Option<Target> = match slot {
                128 => Some(Target::ArtMesh(index)),
                125 => deformer_of(0, index).map(Target::Deformer),
                143 => Some(Target::Part(index)),
                146 => deformer_of(1, index).map(Target::Deformer),
                _ => None,
            };
            let first = bind_begin.get(e).unwrap_or(0).max(0);
            let mut shapes = Vec::new();
            for b in first..first + bind_count.get(e).unwrap_or(0).max(0) {
                let b = b as usize;
                let Some(t) = b_table.get(b).and_then(|t| usize::try_from(t).ok()) else {
                    continue;
                };
                let Some(parameter) = table_param.get(t).copied().flatten() else { continue };
                let (kb, kn) =
                    (keys_begin.get(t).unwrap_or(0).max(0), keys_count.get(t).unwrap_or(0).max(0));
                let base_at = base_key.get(t).unwrap_or(-1);
                let (off, n) = (b_off.get(b).unwrap_or(0), b_len.get(b).unwrap_or(0));
                let mut shape = ayatsuri_format::model::BlendShape {
                    parameter,
                    limits: read_limits(moc, b, model.parameters.len()),
                    ..Default::default()
                };
                for k in 0..kn.min(n) {
                    if k == base_at {
                        continue;
                    }
                    let Some(key) = pool.get((kb + k) as usize) else { continue };
                    shape.keys.push(key);
                    let g = (off + k).max(0) as usize;
                    let get =
                        |p: &Option<F32s<'_>>| p.as_ref().and_then(|p| p.get(g)).unwrap_or(0.0);
                    let block = |begin: Option<i32>, len: usize| -> Vec<f32> {
                        let start = begin.unwrap_or(-1).max(0) as usize;
                        (0..len).map(|x| coords.get(start + x).unwrap_or(0.0)).collect()
                    };
                    match obj {
                        Some(Target::ArtMesh(mi)) => {
                            let len =
                                model.art_meshes.get(mi).map_or(0, |m| m.vertex_count as usize * 2);
                            let kf0 = m_kf.as_ref().and_then(|x| x.get(mi)).unwrap_or(0);
                            shape.positions.extend(block(pos_begin.get(g), len));
                            shape.opacities.push(get(&m_opac));
                            shape.multiply.extend(color_at([141, 142], 107, mi, kf0, g, false));
                            shape.screen.extend(color_at([141, 142], 107, mi, kf0, g, true));
                        }
                        Some(Target::Deformer(di)) if slot == 125 => {
                            let len = match model.deformers.get(di).map(|d| &d.kind) {
                                Some(DeformerKind::Warp(w)) => w.grid_points() * 2,
                                _ => 0,
                            };
                            let kf0 = w_kf.as_ref().and_then(|x| x.get(index)).unwrap_or(0);
                            shape
                                .positions
                                .extend(block(w_pos.as_ref().and_then(|x| x.get(g)), len));
                            shape.opacities.push(get(&w_opac));
                            shape.multiply.extend(color_at([137, 138], 105, index, kf0, g, false));
                            shape.screen.extend(color_at([137, 138], 105, index, kf0, g, true));
                        }
                        Some(Target::Deformer(_)) => {
                            let kf0 = r_kf.as_ref().and_then(|x| x.get(index)).unwrap_or(0);
                            shape.opacities.push(get(&r_opac));
                            shape.angles.push(get(&r_ang));
                            shape.origins_x.push(get(&r_ox));
                            shape.origins_y.push(get(&r_oy));
                            shape.scales.push(get(&r_sc));
                            shape.multiply.extend(color_at([139, 140], 106, index, kf0, g, false));
                            shape.screen.extend(color_at([139, 140], 106, index, kf0, g, true));
                        }
                        Some(Target::Part(_)) => shape.draw_orders.push(get(&part_orders)),
                        None => shape.intensities.push(get(&glue_int)),
                    }
                }
                if shape.keys.is_empty() {
                    continue;
                }
                // what the file holds is what each key adds: the model keeps
                // the form there, the object's base plus that
                for channel in [
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
                ] {
                    if channel.of(&shape).is_empty() {
                        continue;
                    }
                    let base = match obj {
                        Some(t) => ayatsuri_core::shapes::base(model, t, channel),
                        None => model.glues.get(index).and_then(|gl| {
                            ayatsuri_core::shapes::default_blend(
                                model,
                                &gl.binding,
                                &gl.intensities,
                                1,
                            )
                        }),
                    };
                    let Some(base) = base.filter(|b| !b.is_empty()) else {
                        channel.of_mut(&mut shape).clear();
                        continue;
                    };
                    let pool = channel.of_mut(&mut shape);
                    for (v, b) in pool.iter_mut().zip(base.iter().cycle()) {
                        *v += b;
                    }
                    // a colour or opacity the shape never changes stays out
                    if matches!(channel, Channel::Multiply | Channel::Screen | Channel::Opacity)
                        && pool
                            .chunks(base.len())
                            .all(|form| form.iter().zip(&base).all(|(a, b)| (a - b).abs() < 1e-6))
                    {
                        pool.clear();
                    }
                }
                shapes.push(shape);
            }
            let list = match obj {
                Some(t) => ayatsuri_core::shapes::shapes_mut(model, t),
                None => model.glues.get_mut(index).map(|g| &mut g.blend_shapes),
            };
            match list {
                Some(l) => *l = shapes,
                None => {
                    log::warn!("moc3: blend shapes for an object that is not in the file; dropped")
                }
            }
        }
    }
}

/// Binding `b`'s weight limits: its run of constraint indices (123, 124)
/// into the constraints (132-134) and their graphs (135, 136). A limit
/// that points outside the file, or whose graph is empty, is skipped.
fn read_limits(moc: &Moc3<'_>, b: usize, n_params: usize) -> Vec<WeightLimit> {
    let (Ok(run_begin), Ok(run_count), Ok(index)) = (moc.i32s(123), moc.i32s(124), moc.i32s(131))
    else {
        return Vec::new();
    };
    let (Ok(param), Ok(v_begin), Ok(v_count), Ok(keys), Ok(weights)) =
        (moc.i32s(132), moc.i32s(133), moc.i32s(134), moc.f32s(135), moc.f32s(136))
    else {
        return Vec::new();
    };
    let first = run_begin.get(b).unwrap_or(0).max(0);
    let mut out = Vec::new();
    for r in first..first + run_count.get(b).unwrap_or(0).max(0) {
        let Some(c) = index.get(r as usize).and_then(|c| usize::try_from(c).ok()) else { continue };
        let Some(p) = param.get(c).and_then(|p| usize::try_from(p).ok()).filter(|p| *p < n_params)
        else {
            continue;
        };
        let (vb, vn) = (v_begin.get(c).unwrap_or(0).max(0), v_count.get(c).unwrap_or(0).max(0));
        let ks: Vec<f32> = (vb..vb + vn).filter_map(|i| keys.get(i as usize)).collect();
        let ws: Vec<f32> = (vb..vb + vn).filter_map(|i| weights.get(i as usize)).collect();
        if ks.is_empty() || ks.len() != ws.len() {
            log::warn!("moc3: a blend shape weight limit has no usable graph; dropped");
            continue;
        }
        out.push(WeightLimit { parameter: p as u32, keys: ks, weights: ws });
    }
    out
}

/// moc3 5.3's offscreen parts (slots 152-161): each part's offscreen, its
/// masks in the drawable mask pool and one opacity per part keyform. Files
/// before 5.3 have none; a malformed entry leaves that part drawn directly.
fn read_offscreens(moc: &Moc3<'_>, parts: &mut [Part], part_kf_begin: &I32s<'_>) {
    let (Ok(index), Ok(owner), Ok(flag), Ok(blend), Ok(mask_begin), Ok(mask_count)) =
        (moc.i32s(152), moc.i32s(155), moc.u8s(156), moc.i32s(157), moc.i32s(158), moc.i32s(159))
    else {
        return;
    };
    let (Ok(kf_off), Ok(opacity), Ok(pool)) = (moc.i32s(160), moc.f32s(161), moc.i32s(80)) else {
        return;
    };
    for (i, p) in parts.iter_mut().enumerate() {
        let Ok(o) = usize::try_from(index.get(i).unwrap_or(-1)) else { continue };
        if owner.get(o) != Some(i as i32) {
            log::warn!("moc3: offscreen {o} does not belong to part {i}; drawn directly");
            continue;
        }
        let begin = mask_begin.get(o).unwrap_or(0).max(0) as usize;
        let count = mask_count.get(o).unwrap_or(0).max(0) as usize;
        let masks = (begin..begin.saturating_add(count))
            .filter_map(|k| u32::try_from(pool.get(k)?).ok())
            .collect();
        let kf = part_kf_begin.get(i).unwrap_or(0).max(0) as usize;
        let opacities = (0..p.draw_orders.len())
            .map(|k| {
                let at = kf_off.get(kf + k).unwrap_or(-1);
                usize::try_from(at).ok().and_then(|a| opacity.get(a)).unwrap_or(1.0)
            })
            .collect();
        p.offscreen = Some(ayatsuri_format::model::Offscreen {
            masks,
            inverted: flag
                .get(o)
                .is_some_and(|f| f & ayatsuri_format::model::flags::INVERTED_MASK != 0),
            blend: ayatsuri_format::model::Blend::from_moc3(blend.get(o).unwrap_or(0)),
            opacities,
        });
    }
}
