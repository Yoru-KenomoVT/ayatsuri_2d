// The texture atlas: each mesh draws a rectangle of one page, and those
// rectangles are the pieces the atlas editor moves. A layout says which
// page each piece sits on and where; applying one copies the pixels to
// their new places, page by page, and rewrites every mesh's uvs so nothing
// on the model changes. Auto layout shelf-packs the pieces onto as few
// pages of the chosen size as fit.

use std::collections::HashMap;

use ayatsuri_format::model::Model;

/// Room around a piece, so filtering never samples a neighbour, down to
/// the smallest mip level the canvas draws with (see the renderer's
/// sampler).
pub const GAP: u32 = 16;

/// One mesh's rectangle on its page, in page pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub mesh: usize,
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// The mesh's own art on the old page (x0, y0, x1, y1, page pixels):
    /// the rest of the piece is copied as transparent, so a mesh grown
    /// over a neighbour's art carries none of it. None copies it all.
    pub art: Option<[u32; 4]>,
}

/// What makes two pieces the same rectangle of pixels: copies of a mesh
/// share one place.
type PieceKey = (usize, u32, u32, u32, u32, Option<[u32; 4]>);

/// Where a piece goes: page and top-left, in page pixels of the new size.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub page: usize,
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub page_size: u32,
    pub pages: usize,
    /// Parallel to the pieces.
    pub places: Vec<Place>,
}

/// Every mesh's piece: the box around its uvs on its page, padded, and
/// clamped to the page, keeping only its own art, `art[mesh]` (uv boxes,
/// see `Session::art_boxes`). Meshes on pages the model has no image for,
/// and ArtPath meshes (their page is the swatch), are left out.
pub fn pieces(model: &Model, page_sizes: &[(u32, u32)], art: &[[f32; 4]]) -> Vec<Piece> {
    let swatch = model.art_path_page.map(|p| p as usize);
    let mut out = Vec::new();
    for (mi, m) in model.art_meshes.iter().enumerate() {
        let page = m.texture as usize;
        if Some(page) == swatch {
            continue;
        }
        let Some(&(pw, ph)) = page_sizes.get(page) else { continue };
        let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for uv in m.uvs.as_chunks::<2>().0 {
            u0 = u0.min(uv[0]);
            v0 = v0.min(uv[1]);
            u1 = u1.max(uv[0]);
            v1 = v1.max(uv[1]);
        }
        if !(u0 <= u1 && v0 <= v1) {
            continue;
        }
        let x0 = ((u0 * pw as f32).floor() as i64 - GAP as i64).max(0) as u32;
        let y0 = ((v0 * ph as f32).floor() as i64 - GAP as i64).max(0) as u32;
        let x1 = ((u1 * pw as f32).ceil() as u32 + GAP).min(pw);
        let y1 = ((v1 * ph as f32).ceil() as u32 + GAP).min(ph);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let own = art.get(mi).filter(|b| b[0] > -1.0 && b[2] < 2.0).map(|b| {
            [
                (b[0] * pw as f32).floor().clamp(0.0, pw as f32) as u32,
                (b[1] * ph as f32).floor().clamp(0.0, ph as f32) as u32,
                (b[2] * pw as f32).ceil().clamp(0.0, pw as f32) as u32,
                (b[3] * ph as f32).ceil().clamp(0.0, ph as f32) as u32,
            ]
        });
        out.push(Piece { mesh: mi, page, x: x0, y: y0, w: x1 - x0, h: y1 - y0, art: own });
    }
    out
}

/// Shelf-pack the pieces onto pages of `page_size`: tallest first, left to
/// right, a new shelf when the row fills, a new page when the page fills.
/// Pieces that share one rectangle (copies of a mesh) go to one place.
/// A piece bigger than a page is scaled down to fit when `fit` is set,
/// else it is reported as the error.
pub fn auto_layout(pieces: &[Piece], page_size: u32, fit: bool) -> Result<Layout, String> {
    let page_size = page_size.clamp(256, 16384);
    let mut key_of: HashMap<PieceKey, usize> = HashMap::new();
    let mut unique: Vec<(u32, u32)> = Vec::new();
    let mut slot: Vec<usize> = Vec::with_capacity(pieces.len());
    for p in pieces {
        let k = (p.page, p.x, p.y, p.w, p.h, p.art);
        let i = *key_of.entry(k).or_insert_with(|| {
            unique.push((p.w, p.h));
            unique.len() - 1
        });
        slot.push(i);
    }
    let mut order: Vec<usize> = (0..unique.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(unique.get(i).map(|s| s.1).unwrap_or(0)));
    let mut placed: Vec<Place> = vec![Place { page: 0, x: 0, y: 0 }; unique.len()];
    let (mut page, mut x, mut y, mut shelf) = (0usize, 0u32, 0u32, 0u32);
    for i in order {
        let (w, h) = unique.get(i).copied().unwrap_or((0, 0));
        if (w > page_size || h > page_size) && !fit {
            return Err(format!("a {w} x {h} piece does not fit a {page_size} px page"));
        }
        let (w, h) = (w.min(page_size), h.min(page_size));
        if x + w > page_size {
            x = 0;
            y += shelf + GAP;
            shelf = 0;
        }
        if y + h > page_size {
            page += 1;
            x = 0;
            y = 0;
            shelf = 0;
        }
        if let Some(p) = placed.get_mut(i) {
            *p = Place { page, x, y };
        }
        x += w + GAP;
        shelf = shelf.max(h);
    }
    let places = slot
        .iter()
        .map(|&i| placed.get(i).cloned().unwrap_or(Place { page: 0, x: 0, y: 0 }))
        .collect();
    Ok(Layout { page_size, pages: if unique.is_empty() { 0 } else { page + 1 }, places })
}

/// Rewrite the model's uvs and page numbers for a layout. Pixels are not
/// touched here; `build_pages` does that.
pub fn apply_to_model(
    model: &mut Model,
    pieces: &[Piece],
    layout: &Layout,
    page_sizes: &[(u32, u32)],
    first_new_page: u32,
) {
    let n = layout.page_size as f32;
    for (p, place) in pieces.iter().zip(&layout.places) {
        let Some(&(pw, ph)) = page_sizes.get(p.page) else { continue };
        let Some(am) = model.art_meshes.get_mut(p.mesh) else { continue };
        // a piece scaled to fit a page keeps its proportions
        let sx = (layout.page_size.min(p.w)) as f32 / p.w.max(1) as f32;
        let sy = (layout.page_size.min(p.h)) as f32 / p.h.max(1) as f32;
        for uv in am.uvs.as_chunks_mut::<2>().0 {
            let px = uv[0] * pw as f32 - p.x as f32;
            let py = uv[1] * ph as f32 - p.y as f32;
            uv[0] = (place.x as f32 + px * sx) / n;
            uv[1] = (place.y as f32 + py * sy) / n;
        }
        am.texture = first_new_page + place.page as u32;
    }
}

/// Compose the new pages, one at a time, copying each piece's pixels from
/// its old page. `decode` fetches an old page's pixels (the caller keeps
/// one decoded at a time); the result is one RGBA image per new page.
pub fn build_pages(
    pieces: &[Piece],
    layout: &Layout,
    mut decode: impl FnMut(usize) -> Option<image::RgbaImage>,
) -> Vec<image::RgbaImage> {
    let n = layout.page_size;
    let mut out = Vec::with_capacity(layout.pages);
    for page in 0..layout.pages {
        let mut img = image::RgbaImage::new(n, n);
        // pieces of this page grouped by source page, so each source is decoded once
        let mut by_src: Vec<(usize, &Piece, &Place)> = pieces
            .iter()
            .zip(&layout.places)
            .filter(|(_, pl)| pl.page == page)
            .map(|(p, pl)| (p.page, p, pl))
            .collect();
        by_src.sort_by_key(|(src, _, _)| *src);
        let mut cache: Option<(usize, image::RgbaImage)> = None;
        let mut done: std::collections::HashSet<PieceKey> = std::collections::HashSet::new();
        for (src, p, pl) in by_src {
            if !done.insert((p.page, p.x, p.y, p.w, p.h, p.art)) {
                continue;
            }
            if cache.as_ref().is_none_or(|(s, _)| *s != src) {
                cache = decode(src).map(|img| (src, img));
            }
            let Some((_, source)) = &cache else { continue };
            let (w, h) = (p.w.min(n), p.h.min(n));
            let mut view = image::imageops::crop_imm(source, p.x, p.y, p.w, p.h).to_image();
            // only the mesh's own art comes along
            if let Some([x0, y0, x1, y1]) = p.art {
                for (x, y, px) in view.enumerate_pixels_mut() {
                    let (sx, sy) = (p.x + x, p.y + y);
                    if sx < x0 || sx >= x1 || sy < y0 || sy >= y1 {
                        *px = image::Rgba([0, 0, 0, 0]);
                    }
                }
            }
            if w == p.w && h == p.h {
                image::imageops::replace(&mut img, &view, pl.x as i64, pl.y as i64);
            } else {
                let small =
                    image::imageops::resize(&view, w, h, image::imageops::FilterType::Triangle);
                image::imageops::replace(&mut img, &small, pl.x as i64, pl.y as i64);
            }
        }
        out.push(img);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Binding};

    fn mesh(page: u32, uvs: &[f32]) -> ArtMesh {
        let n = (uvs.len() / 2) as u32;
        ArtMesh {
            id: "m".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: page,
            flags: 0,
            blend: Default::default(),
            vertex_count: n,
            uvs: uvs.to_vec(),
            indices: vec![0, 1, 2],
            masks: Vec::new(),
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0; uvs.len()],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
        }
    }

    #[test]
    fn pieces_are_padded_uv_boxes_on_their_page() {
        let mut m = Model::new();
        m.art_meshes.push(mesh(0, &[0.25, 0.25, 0.5, 0.25, 0.5, 0.5]));
        let p = pieces(&m, &[(1024, 1024)], &[]);
        assert_eq!(
            p,
            vec![Piece {
                mesh: 0,
                page: 0,
                x: 256 - GAP,
                y: 256 - GAP,
                w: 256 + 2 * GAP,
                h: 256 + 2 * GAP,
                art: None,
            }]
        );
    }

    #[test]
    fn auto_layout_packs_and_shares_identical_rectangles() {
        let ps = vec![
            Piece { mesh: 0, page: 0, x: 0, y: 0, w: 300, h: 300, art: None },
            Piece { mesh: 1, page: 0, x: 0, y: 0, w: 300, h: 300, art: None },
            Piece { mesh: 2, page: 1, x: 10, y: 10, w: 300, h: 100, art: None },
        ];
        let l = auto_layout(&ps, 512, false).unwrap();
        assert_eq!(l.pages, 1);
        assert_eq!(l.places[0], l.places[1]);
        assert_ne!(l.places[0], l.places[2]);
        let too_big = vec![Piece { mesh: 0, page: 0, x: 0, y: 0, w: 600, h: 10, art: None }];
        assert!(auto_layout(&too_big, 512, false).is_err());
        assert_eq!(auto_layout(&too_big, 512, true).unwrap().pages, 1);
    }

    #[test]
    fn uvs_follow_their_piece_exactly() {
        let mut m = Model::new();
        m.art_meshes.push(mesh(0, &[0.25, 0.25, 0.5, 0.25, 0.5, 0.5]));
        let sizes = [(1024, 1024)];
        let ps = pieces(&m, &sizes, &[]);
        let layout =
            Layout { page_size: 512, pages: 1, places: vec![Place { page: 0, x: 100, y: 200 }] };
        apply_to_model(&mut m, &ps, &layout, &sizes, 3);
        let am = &m.art_meshes[0];
        assert_eq!(am.texture, 3);
        // the first uv sat GAP px inside the piece, so it lands GAP px inside the place
        let expect_u = (100.0 + GAP as f32) / 512.0;
        let expect_v = (200.0 + GAP as f32) / 512.0;
        assert!((am.uvs[0] - expect_u).abs() < 1e-5 && (am.uvs[1] - expect_v).abs() < 1e-5);
    }

    #[test]
    fn a_grown_mesh_carries_only_its_own_art() {
        // a red layer at 0..4 with a blue neighbour at 4..8; the mesh grew
        // over both, its art is the red one
        let src = image::RgbaImage::from_fn(8, 8, |x, _| {
            if x < 4 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) }
        });
        let ps = vec![Piece { mesh: 0, page: 0, x: 0, y: 0, w: 8, h: 8, art: Some([0, 0, 4, 8]) }];
        let layout =
            Layout { page_size: 16, pages: 1, places: vec![Place { page: 0, x: 0, y: 0 }] };
        let out = build_pages(&ps, &layout, |_| Some(src.clone()));
        assert_eq!(out[0].get_pixel(2, 2), &image::Rgba([255, 0, 0, 255]));
        assert_eq!(out[0].get_pixel(6, 2), &image::Rgba([0, 0, 0, 0]));
        // and pieces work the box out from uvs
        let mut m = Model::new();
        m.art_meshes.push(mesh(0, &[0.0, 0.0, 1.0, 0.0, 1.0, 1.0]));
        let p = pieces(&m, &[(8, 8)], &[[0.0, 0.0, 0.5, 1.0]]);
        assert_eq!(p[0].art, Some([0, 0, 4, 8]));
    }

    #[test]
    fn pages_are_rebuilt_from_the_pieces() {
        let ps = vec![Piece { mesh: 0, page: 0, x: 2, y: 2, w: 4, h: 4, art: None }];
        let layout =
            Layout { page_size: 16, pages: 1, places: vec![Place { page: 0, x: 8, y: 8 }] };
        let src = image::RgbaImage::from_fn(8, 8, |x, y| {
            if (2..6).contains(&x) && (2..6).contains(&y) {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        let out = build_pages(&ps, &layout, |_| Some(src.clone()));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get_pixel(8, 8), &image::Rgba([255, 0, 0, 255]));
        assert_eq!(out[0].get_pixel(12, 12), &image::Rgba([0, 0, 0, 0]));
        assert_eq!(out[0].get_pixel(11, 11), &image::Rgba([255, 0, 0, 255]));
    }
}
