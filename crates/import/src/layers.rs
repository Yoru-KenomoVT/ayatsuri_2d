// Layered paint documents (PSD, Krita) into a fresh model, the way Cubism's
// PSD import works: every group becomes a part, every visible pixel layer
// an art mesh, and the layer pixels get trimmed and packed into texture
// atlases. Meshes start as a two-triangle quad over the trimmed bounds;
// riggers re-mesh them with the auto mesher.

use ayatsuri_format::model::{ArtMesh, Binding, Canvas, Model, Part};

use crate::ImportError;

/// One raster layer, already flattened to straight RGBA.
pub struct Layer {
    pub name: String,
    /// Enclosing group names, outermost first.
    pub groups: Vec<String>,
    /// Placement on the canvas, in pixels.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub visible: bool,
    pub opacity: f32,
    /// Clipped to the nearest unclipped layer below, Photoshop style; it
    /// becomes a mask on the mesh.
    pub clip_to_below: bool,
}

/// A whole document, layers bottom to top.
pub struct Document {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
}

/// What an import hands back: the model plus its encoded texture pages.
pub struct Imported {
    pub model: Model,
    pub textures: Vec<Vec<u8>>,
    /// Display names, parallel to the model's parts and meshes.
    pub part_names: Vec<String>,
    /// The layer names, parallel to the model's art meshes. IDs stay plain
    /// ASCII (ArtMesh0, ArtMesh1...) the way Cubism names them.
    pub mesh_names: Vec<String>,
    pub skipped: usize,
    /// Layers bigger than a texture page, stored downscaled.
    pub scaled: usize,
}

/// A trimmed box inside a layer: x, y, width, height.
type Bounds = (u32, u32, u32, u32);
/// Where a box landed: atlas page, x, y.
type Spot = (usize, u32, u32);

/// The atlas page side; 4096 keeps big illustrations to few pages and is
/// what every GPU the editor runs on accepts.
const PAGE: u32 = 4096;
const GAP: u32 = 2;

/// Model-safe ids: ASCII letters, digits and underscores.
fn id_of(prefix: &str, n: usize) -> String {
    format!("{prefix}{n}")
}

/// Trim to the smallest box holding any alpha. None for an empty layer.
fn trim(layer: &Layer, cw: u32, ch: u32) -> Option<Bounds> {
    let (w, h) = (layer.width as usize, layer.height as usize);
    // the part of the layer that sits on the canvas; paint outside it is
    // invisible in the artist's own render, so it never reaches the atlas
    let cx0 = (-layer.x).clamp(0, w as i32) as usize;
    let cy0 = (-layer.y).clamp(0, h as i32) as usize;
    let cx1 = (cw as i32 - layer.x).clamp(0, w as i32) as usize;
    let cy1 = (ch as i32 - layer.y).clamp(0, h as i32) as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in cy0..cy1 {
        for x in cx0..cx1 {
            let a = layer.rgba.get((y * w + x) * 4 + 3).copied().unwrap_or(0);
            if a > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x0 <= x1 && y0 <= y1).then_some((
        x0 as u32,
        y0 as u32,
        (x1 - x0 + 1) as u32,
        (y1 - y0 + 1) as u32,
    ))
}

/// Shelf packing: tallest first, left to right, new shelf when a row
/// fills, new page when a page fills. Returns (page, x, y) per input.
fn pack(sizes: &[(u32, u32)]) -> Result<(Vec<Spot>, usize), ImportError> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(sizes.get(i).map(|s| s.1).unwrap_or(0)));
    let mut out: Vec<Spot> = vec![(0, 0, 0); sizes.len()];
    let (mut page, mut x, mut y, mut shelf) = (0usize, 0u32, 0u32, 0u32);
    for i in order {
        let (w, h) = sizes.get(i).copied().unwrap_or((0, 0));
        if w > PAGE || h > PAGE {
            return Err(ImportError::Export(format!(
                "a {w}x{h} layer is bigger than a {PAGE}px texture page"
            )));
        }
        if x + w > PAGE {
            x = 0;
            y += shelf + GAP;
            shelf = 0;
        }
        if y + h > PAGE {
            page += 1;
            x = 0;
            y = 0;
            shelf = 0;
        }
        if let Some(slot) = out.get_mut(i) {
            *slot = (page, x, y);
        }
        x += w + GAP;
        shelf = shelf.max(h);
    }
    Ok((out, page + 1))
}

pub fn build(doc: &Document) -> Result<Imported, ImportError> {
    if doc.width == 0 || doc.height == 0 {
        return Err(ImportError::Export("document has no size".into()));
    }
    // one part per distinct group path, created on first sight; layers
    // outside any group stay part-less and show at the palette's top level
    let mut parts: Vec<Part> = Vec::new();
    let mut part_names: Vec<String> = Vec::new();
    let mut paths: Vec<Vec<String>> = Vec::new();
    let mut part_for = |path: &[String], parts: &mut Vec<Part>| -> Option<u32> {
        let mut parent: Option<u32> = None;
        for depth in 1..=path.len() {
            let sub = path.get(..depth).unwrap_or(&[]).to_vec();
            let found = paths.iter().position(|p| *p == sub);
            let idx = match found {
                Some(i) => i,
                None => {
                    paths.push(sub.clone());
                    parts.push(Part {
                        id: id_of("Part", parts.len()),
                        parent_part: parent,
                        visible: true,
                        enabled: true,
                        binding: Binding::default(),
                        draw_orders: vec![500.0],
                    });
                    part_names.push(sub.last().cloned().unwrap_or_default());
                    parts.len() - 1
                }
            };
            parent = Some(idx as u32);
        }
        parent
    };

    // keep only layers with pixels; trim each to its alpha
    let mut kept: Vec<(&Layer, Bounds)> = Vec::new();
    let mut skipped = 0;
    for l in &doc.layers {
        if !l.visible || l.opacity <= 0.0 {
            skipped += 1;
            continue;
        }
        match trim(l, doc.width, doc.height) {
            Some(b) => kept.push((l, b)),
            None => skipped += 1,
        }
    }
    // each trimmed layer becomes its own small image, opacity baked in;
    // anything bigger than a page is scaled down to fit, which only costs
    // texel density since the mesh keeps its canvas-space size
    let mut tiles: Vec<image::RgbaImage> = Vec::with_capacity(kept.len());
    let mut scaled = 0usize;
    for (layer, (tx, ty, tw, th)) in &kept {
        let mut img = image::RgbaImage::new(*tw, *th);
        for row in 0..*th {
            for col in 0..*tw {
                let src = (((ty + row) * layer.width + (tx + col)) * 4) as usize;
                let Some(px) = layer.rgba.get(src..src + 4) else { continue };
                let a = (px.get(3).copied().unwrap_or(0) as f32 * layer.opacity).round() as u8;
                img.put_pixel(
                    col,
                    row,
                    image::Rgba([
                        px.first().copied().unwrap_or(0),
                        px.get(1).copied().unwrap_or(0),
                        px.get(2).copied().unwrap_or(0),
                        a,
                    ]),
                );
            }
        }
        let limit = PAGE - GAP;
        if *tw > limit || *th > limit {
            let k = (limit as f32 / *tw as f32).min(limit as f32 / *th as f32);
            let (nw, nh) = (((*tw as f32 * k) as u32).max(1), ((*th as f32 * k) as u32).max(1));
            img = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
            scaled += 1;
        }
        tiles.push(img);
    }
    let sizes: Vec<(u32, u32)> = tiles.iter().map(|t| (t.width(), t.height())).collect();
    let (spots, pages) = pack(&sizes)?;

    let mut atlases: Vec<image::RgbaImage> =
        (0..pages).map(|_| image::RgbaImage::new(PAGE, PAGE)).collect();

    // model units: one unit per canvas height, origin at the canvas centre
    let ppu = doc.height as f32;
    let canvas = Canvas {
        pixels_per_unit: ppu,
        origin_x: doc.width as f32 * 0.5,
        origin_y: doc.height as f32 * 0.5,
        width: doc.width as f32,
        height: doc.height as f32,
    };

    let mut meshes: Vec<ArtMesh> = Vec::with_capacity(kept.len());
    let mut clip_base: Option<u32> = None;
    for (n, (((layer, (tx, ty, tw, th)), (page, ax, ay)), tile)) in
        kept.iter().zip(spots.iter()).zip(tiles.iter()).enumerate()
    {
        let masks = if layer.clip_to_below {
            clip_base.map(|b| vec![b]).unwrap_or_default()
        } else {
            clip_base = Some(n as u32);
            Vec::new()
        };
        if let Some(atlas) = atlases.get_mut(*page) {
            image::imageops::replace(atlas, tile, *ax as i64, *ay as i64);
        }
        // the quad, in canvas pixels then model units
        let left = (layer.x + *tx as i32) as f32;
        let top = (layer.y + *ty as i32) as f32;
        let (right, bottom) = (left + *tw as f32, top + *th as f32);
        let to_unit =
            |px: f32, py: f32| ((px - canvas.origin_x) / ppu, (py - canvas.origin_y) / ppu);
        let corners = [
            to_unit(left, top),
            to_unit(right, top),
            to_unit(right, bottom),
            to_unit(left, bottom),
        ];
        let p = PAGE as f32;
        let (u0, v0) = (*ax as f32 / p, *ay as f32 / p);
        let (u1, v1) = ((*ax + tile.width()) as f32 / p, (*ay + tile.height()) as f32 / p);
        meshes.push(ArtMesh {
            id: id_of("ArtMesh", n),
            parent_part: part_for(&layer.groups, &mut parts),
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: *page as u32,
            flags: 0,
            vertex_count: 4,
            uvs: vec![u0, v0, u1, v0, u1, v1, u0, v1],
            indices: vec![0, 1, 2, 0, 2, 3],
            masks,
            binding: Binding::default(),
            opacities: vec![1.0],
            // bottom layers first: later meshes draw over earlier ones
            draw_orders: vec![500.0],
            positions: corners.iter().flat_map(|&(x, y)| [x, y]).collect(),
        });
    }

    let mut textures = Vec::with_capacity(atlases.len());
    for atlas in &atlases {
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(atlas.clone())
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| ImportError::Export(format!("atlas encode: {e}")))?;
        textures.push(png);
    }

    let mesh_names: Vec<String> = kept.iter().map(|(l, _)| l.name.clone()).collect();
    let mut model = Model::new();
    model.canvas = canvas;
    model.parts = parts;
    model.art_meshes = meshes;
    Ok(Imported { model, textures, part_names, mesh_names, skipped, scaled })
}
