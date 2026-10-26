// Inherited clipping for export. moc3 masks only add up (inside any one of
// them), so "inside the iris and inside the eye white" cannot be written
// as masks directly; but it is "outside neither", and a mesh can clip by
// its masks inverted. So each mask in an inheriting mesh's chain gets a
// hidden helper mesh showing exactly where that mask does not: the mask's
// own triangles moving with the same keyforms, its art's alpha flipped on
// a new atlas page, and a solid ring out to a box well around it. The mesh
// then clips, inverted, by the helpers. Plain masks only, so it works in
// every moc3 version and VTube Studio, at the cost of a mask mesh per
// helper (no offscreen layers).
//
// Done on copies at export time; the editor keeps the inherit setting.

use ayatsuri_core::automesh;
use ayatsuri_format::Model;
use ayatsuri_format::model::{ArtMesh, DrawItem, flags};

use crate::atlas::{self, Piece};

/// The model and pages to export, with helpers added.
pub struct Expanded {
    pub model: Model,
    pub pngs: Vec<Vec<u8>>,
    /// Helper meshes added.
    pub helpers: usize,
    /// Meshes whose inherited clip now goes out as helpers.
    pub meshes: usize,
}

/// How far the solid ring reaches past the mask, in mask sizes on each
/// side: the inheriting mesh must stay inside the box for the clip to hold.
const REACH: f32 = 2.0;

/// The solid patch the rings sample, in page pixels.
const PATCH: u32 = 8;

/// Add the helpers for every inheriting mesh moc3 can carry. None when
/// there are none (export the model as it is).
pub fn expand(model: &Model, pngs: &[Vec<u8>]) -> Result<Option<Expanded>, String> {
    let targets: Vec<(usize, Vec<u32>)> = (0..model.art_meshes.len())
        .filter_map(|mi| match ayatsuri_core::clip::inherited_masks(model, mi) {
            Some(Ok(masks)) => Some((mi, masks)),
            _ => None,
        })
        .collect();
    if targets.is_empty() {
        return Ok(None);
    }
    let mut needed: Vec<u32> = targets.iter().flat_map(|(_, m)| m.iter().copied()).collect();
    needed.sort_unstable();
    needed.dedup();

    // the masks' art, and a solid patch, onto a new page
    let sizes: Vec<(u32, u32)> = pngs.iter().map(|p| png_size(p).unwrap_or((1, 1))).collect();
    let mut pieces: Vec<Piece> = atlas::pieces(model, &sizes, &[])
        .into_iter()
        .filter(|p| needed.contains(&(p.mesh as u32)))
        .collect();
    let patch_src = pngs.len();
    pieces.push(Piece {
        mesh: usize::MAX,
        page: patch_src,
        x: 0,
        y: 0,
        w: PATCH,
        h: PATCH,
        art: None,
    });
    let layout = [512u32, 1024, 2048, 4096]
        .into_iter()
        .find_map(|n| atlas::auto_layout(&pieces, n, false).ok().filter(|l| l.pages == 1))
        .map_or_else(|| atlas::auto_layout(&pieces, 4096, true), Ok)?;
    let decode = |i: usize| -> Option<image::RgbaImage> {
        if i == patch_src {
            return Some(image::RgbaImage::from_pixel(PATCH, PATCH, image::Rgba([255; 4])));
        }
        pngs.get(i).and_then(|p| image::load_from_memory(p).ok()).map(|img| img.into_rgba8())
    };
    let mut pages = atlas::build_pages(&pieces, &layout, decode);
    // the masks' alpha turned inside out: shown where the mask is not
    // (masks sharing art share a place: flip it once)
    let mut flipped = std::collections::HashSet::new();
    for (p, place) in pieces.iter().zip(&layout.places) {
        if p.mesh == usize::MAX || !flipped.insert((place.page, place.x, place.y)) {
            continue;
        }
        let Some(img) = pages.get_mut(place.page) else { continue };
        let (w, h) = (p.w.min(layout.page_size), p.h.min(layout.page_size));
        for y in place.y..(place.y + h).min(img.height()) {
            for x in place.x..(place.x + w).min(img.width()) {
                let px = img.get_pixel_mut(x, y);
                *px = image::Rgba([255, 255, 255, 255 - px[3]]);
            }
        }
    }
    let first_page = pngs.len();
    let mut out_pngs = pngs.to_vec();
    for img in pages {
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        out_pngs.push(png);
    }
    let n = layout.page_size as f32;
    let patch_place = pieces
        .iter()
        .zip(&layout.places)
        .find(|(p, _)| p.mesh == usize::MAX)
        .map(|(_, pl)| pl.clone());
    let Some(patch_place) = patch_place else { return Err("no room for the helper patch".into()) };
    let patch_uv = [
        (patch_place.x as f32 + PATCH as f32 * 0.5) / n,
        (patch_place.y as f32 + PATCH as f32 * 0.5) / n,
    ];

    let mut out = model.clone();
    let mut helper_of: Vec<(u32, u32)> = Vec::new();
    for &m in &needed {
        let Some(src) = model.art_meshes.get(m as usize) else { continue };
        let Some((p, place)) =
            pieces.iter().zip(&layout.places).find(|(p, _)| p.mesh == m as usize)
        else {
            continue;
        };
        let Some(&(pw, ph)) = sizes.get(p.page) else { continue };
        let scale = (
            layout.page_size.min(p.w) as f32 / p.w.max(1) as f32,
            layout.page_size.min(p.h) as f32 / p.h.max(1) as f32,
        );
        let uv = |u: f32, v: f32| {
            [
                (place.x as f32 + (u * pw as f32 - p.x as f32) * scale.0) / n,
                (place.y as f32 + (v * ph as f32 - p.y as f32) * scale.1) / n,
            ]
        };
        let page = (first_page + place.page) as u32;
        let id = free_id(&out, &format!("{}_NotMask", src.id));
        let Some(helper) = helper_mesh(src, id, page, &uv, patch_uv) else {
            log::warn!("export: no helper for mask {}; its clip goes out plain", src.id);
            continue;
        };
        helper_of.push((m, out.art_meshes.len() as u32));
        out.art_meshes.push(helper);
    }
    // a new mesh joins the root draw order group of an imported model
    let added = helper_of.len() as u32;
    if let Some(root) = out.draw_groups.first_mut() {
        for &(_, h) in &helper_of {
            root.items.push(DrawItem { part: false, object: h, child_group: None });
        }
        root.total += added;
    }
    let mut meshes = 0;
    for (mi, masks) in &targets {
        let helpers: Option<Vec<u32>> =
            masks.iter().map(|m| helper_of.iter().find(|(x, _)| x == m).map(|&(_, h)| h)).collect();
        let (Some(helpers), Some(mesh)) = (helpers, out.art_meshes.get_mut(*mi)) else {
            continue;
        };
        mesh.masks = helpers;
        mesh.flags |= flags::INVERTED_MASK;
        mesh.clip_inherit = false;
        mesh.clip_group = None;
        meshes += 1;
    }
    Ok(Some(Expanded { model: out, pngs: out_pngs, helpers: helper_of.len(), meshes }))
}

fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(png))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

fn free_id(model: &Model, want: &str) -> String {
    let taken = |id: &str| model.art_meshes.iter().any(|m| m.id == id);
    if !taken(want) {
        return want.to_string();
    }
    (2..).map(|i| format!("{want}{i}")).find(|id| !taken(id)).unwrap_or_default()
}

/// The helper for mask `src`: its triangles with the flipped art, then a
/// copy of its outline points and a box around it, joined by a solid ring.
/// Every form of the mask (keyforms, blend shapes) gets the same points
/// added, the box sized to that form, so the helper moves as the mask does.
fn helper_mesh(
    src: &ArtMesh,
    id: String,
    page: u32,
    uv: &dyn Fn(f32, f32) -> [f32; 2],
    patch_uv: [f32; 2],
) -> Option<ArtMesh> {
    let n = src.vertex_count as usize;
    let stride = n * 2;
    if n < 3 || stride == 0 {
        return None;
    }
    // the outline: edges of exactly one triangle
    let mut count: std::collections::HashMap<(u16, u16), u32> = std::collections::HashMap::new();
    for t in src.indices.as_chunks::<3>().0 {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    let mut edges: Vec<(u16, u16)> =
        count.into_iter().filter(|&(_, c)| c == 1).map(|(e, _)| e).collect();
    edges.sort_unstable();
    let mut outline: Vec<u16> = edges.iter().flat_map(|&(a, b)| [a, b]).collect();
    outline.sort_unstable();
    outline.dedup();
    let r = outline.len();
    let local = |v: u16| outline.binary_search(&v).ok();

    // one form of the mask -> the helper's: its points, the outline's
    // copies, the box's eight points
    let extend = |form: &[f32]| -> Vec<f32> {
        let mut out = form.to_vec();
        for &v in &outline {
            let i = v as usize * 2;
            out.extend([
                form.get(i).copied().unwrap_or(0.0),
                form.get(i + 1).copied().unwrap_or(0.0),
            ]);
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in form.as_chunks::<2>().0 {
            (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
        }
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let span = (x1 - x0).max(y1 - y0).max(1e-3);
        let (hx, hy) = ((x1 - x0).max(span * 0.25), (y1 - y0).max(span * 0.25));
        let (ex, ey) = (hx * (0.5 + REACH), hy * (0.5 + REACH));
        for (dx, dy) in [
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (1.0, 0.0),
            (1.0, 1.0),
            (0.0, 1.0),
            (-1.0, 1.0),
            (-1.0, 0.0),
        ] {
            out.extend([cx + dx * ex, cy + dy * ey]);
        }
        out
    };

    // the ring, triangulated once on the first keyform
    let first = src.positions.get(..stride)?;
    let ext = extend(first);
    let ring_pts: Vec<(f32, f32)> =
        ext.get(stride..)?.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect();
    let walls: Vec<(usize, usize)> =
        edges.iter().filter_map(|&(a, b)| Some((local(a)?, local(b)?))).collect();
    let tris = automesh::constrained(&ring_pts, &walls);
    let keep = automesh::outside(&tris, &walls);
    let total = n + r + 8;
    if total >= u16::MAX as usize {
        return None;
    }
    let mut indices = src.indices.clone();
    for (t, k) in tris.iter().zip(&keep) {
        if *k {
            indices.extend(t.iter().map(|&i| (n + i) as u16));
        }
    }

    let keyforms = src.positions.len() / stride;
    let mut positions = Vec::with_capacity(keyforms * total * 2);
    for k in 0..keyforms {
        positions.extend(extend(src.positions.get(k * stride..(k + 1) * stride)?));
    }
    let mut uvs: Vec<f32> =
        src.uvs.as_chunks::<2>().0.iter().flat_map(|p| uv(p[0], p[1])).collect();
    for _ in 0..r + 8 {
        uvs.extend(patch_uv);
    }
    let blend_shapes = src
        .blend_shapes
        .iter()
        .map(|s| {
            let mut shape = s.clone();
            shape.positions = s.positions.chunks(stride).flat_map(&extend).collect();
            shape
        })
        .collect();
    Some(ArtMesh {
        id,
        parent_part: src.parent_part,
        parent_deformer: src.parent_deformer,
        visible: true,
        enabled: true,
        texture: page,
        flags: 0,
        blend: Default::default(),
        vertex_count: total as u32,
        uvs,
        indices,
        masks: Vec::new(),
        clip_group: None,
        clip_inherit: false,
        binding: src.binding.clone(),
        opacities: vec![0.0; keyforms],
        draw_orders: src.draw_orders.clone(),
        positions,
        blend_shapes,
        multiply: Vec::new(),
        screen: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_core::Puppet;
    use ayatsuri_format::model::Canvas;
    use ayatsuri_render::TexData;

    /// A quad from x0 to x1, full height, on an 8 x 8 canvas, its art the
    /// whole (solid white) page.
    fn quad(id: &str, x0: f32, x1: f32, opacity: f32, masks: Vec<u32>) -> ArtMesh {
        ArtMesh {
            id: id.into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 4,
            uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            indices: vec![0, 1, 2, 0, 2, 3],
            masks,
            clip_group: None,
            clip_inherit: false,
            binding: Default::default(),
            opacities: vec![opacity],
            draw_orders: vec![500.0],
            positions: vec![x0, 0.0, x1, 0.0, x1, 8.0, x0, 8.0],
            blend_shapes: Vec::new(),
            multiply: Vec::new(),
            screen: Vec::new(),
        }
    }

    fn textures(pngs: &[Vec<u8>]) -> Vec<TexData> {
        pngs.iter()
            .map(|p| {
                let img = image::load_from_memory(p).unwrap().into_rgba8();
                TexData { width: img.width(), height: img.height(), rgba: img.into_raw() }
            })
            .collect()
    }

    /// Which of the 8 columns the highlight shows in, one row down the middle.
    fn shown(model: &Model, pngs: &[Vec<u8>]) -> Option<Vec<bool>> {
        let mut f = crate::gpu_frames::Frames::new(textures(pngs))?;
        let mut p = Puppet::new(model.clone());
        p.update();
        let (_, _, px) = f.draw(&p, 8, &[])?;
        Some((0..8).map(|x| px.get((4 * 8 + x) * 4 + 3).copied().unwrap_or(0) > 128).collect())
    }

    #[test]
    fn helpers_clip_exactly_as_the_editor_draws_inheritance() {
        let mut m = Model {
            canvas: Canvas { width: 8.0, height: 8.0, pixels_per_unit: 1.0, ..Canvas::default() },
            ..Model::default()
        };
        // eye white and iris drawn at 0% (masks only), the highlight shows
        m.art_meshes.push(quad("white", 0.0, 4.0, 0.0, vec![]));
        m.art_meshes.push(quad("iris", 2.0, 8.0, 0.0, vec![0]));
        let mut highlight = quad("highlight", 0.0, 8.0, 1.0, vec![1]);
        highlight.clip_inherit = true;
        m.art_meshes.push(highlight);
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(32, 32, image::Rgba([255; 4]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let pngs = vec![png];

        let e = expand(&m, &pngs).unwrap().unwrap();
        assert_eq!((e.helpers, e.meshes), (2, 1));
        let h = &e.model.art_meshes[2];
        assert!(!h.clip_inherit && h.flags & flags::INVERTED_MASK != 0);
        assert_eq!(h.masks, vec![4, 3], "inside neither helper: not-iris, not-white");
        assert_eq!(e.pngs.len(), 2, "one page of flipped art");
        // and it is plain moc3, back as written in every version
        for v in ayatsuri_import::ExportVersion::ALL {
            let moc = ayatsuri_import::to_moc3(&e.model, v).unwrap();
            let back = ayatsuri_import::from_moc3(&moc).unwrap();
            let h = &back.art_meshes[2];
            assert_eq!(h.masks, vec![4, 3], "{v:?}");
            assert!(h.flags & flags::INVERTED_MASK != 0, "{v:?}");
            assert!(ayatsuri_import::export_warnings(&m, v).iter().all(|w| !w.contains("inherit")));
        }

        let Some(editor) = shown(&m, &pngs) else {
            eprintln!("no GPU adapter here: skipping");
            return;
        };
        assert_eq!(editor, vec![false, false, true, true, false, false, false, false]);
        assert_eq!(shown(&e.model, &e.pngs), Some(editor), "the export draws the same");
    }

    #[test]
    fn nothing_to_do_without_inheritance() {
        let mut m = Model::default();
        m.art_meshes.push(quad("a", 0.0, 4.0, 1.0, vec![]));
        m.art_meshes.push(quad("b", 0.0, 4.0, 1.0, vec![0]));
        assert!(expand(&m, &[]).unwrap().is_none());
    }
}
