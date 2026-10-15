// Clip Studio Paint documents. A .clip is a chunked container: a header,
// external data chunks (the layers' pixel tiles among them) and an SQLite
// database that holds the canvas, the layer tree and which chunk each
// layer's bitmap lives in. Worked out from files; the layout is:
//
//     CSFCHUNK  u64 file size  u64 first chunk offset
//     CHNKHead  u64 size  ...
//     CHNKExta  u64 size  u64 id length  id  u64 body length  body
//     CHNKSQLi  u64 size  <SQLite database>
//     CHNKFoot  u64 0
//
// A bitmap ("offscreen") is a grid of 256 x 256 tiles. Each tile is one
// zlib stream holding the alpha plane first, then the colour pixels as
// interleaved B, G, R and a spare byte. Tiles the artist never touched are
// not stored at all. Masks are one-plane tiles. Integers are big-endian
// unless noted.

use crate::ImportError;
use crate::layers::{Document, Layer};
use crate::sqlite::{Db, Table, Value};

fn err(msg: impl Into<String>) -> ImportError {
    ImportError::Export(format!("clip: {}", msg.into()))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

fn be64(b: &[u8], at: usize) -> Option<u64> {
    let hi = be32(b, at)? as u64;
    let lo = be32(b, at + 4)? as u64;
    Some(hi << 32 | lo)
}

/// A length-prefixed UTF-16BE label; returns its text and the offset after.
fn label(b: &[u8], at: usize) -> Option<(String, usize)> {
    let n = be32(b, at)? as usize;
    if n > 64 {
        return None;
    }
    let raw = b.get(at + 4..at + 4 + n * 2)?;
    let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
    Some((String::from_utf16_lossy(&units), at + 4 + n * 2))
}

/// The external chunks by id, and the database image.
struct Container<'a> {
    exta: Vec<(&'a [u8], &'a [u8])>,
    sqlite: &'a [u8],
}

fn container(bytes: &[u8]) -> Result<Container<'_>, ImportError> {
    if bytes.get(..8) != Some(b"CSFCHUNK") {
        return Err(err("not a Clip Studio file"));
    }
    let mut pos = be64(bytes, 16).ok_or_else(|| err("short header"))? as usize;
    let mut exta = Vec::new();
    let mut sqlite = None;
    let mut hops = 0;
    while let Some(body) = pos.checked_add(16).filter(|b| *b <= bytes.len()) {
        hops += 1;
        if hops > 1 << 20 {
            return Err(err("too many chunks"));
        }
        let tag = bytes.get(pos..pos + 8).ok_or_else(|| err("chunk"))?;
        let size = usize::try_from(be64(bytes, pos + 8).ok_or_else(|| err("chunk"))?)
            .map_err(|_| err("chunk size"))?;
        let end = body.checked_add(size).filter(|e| *e <= bytes.len());
        match tag {
            b"CHNKExta" => {
                let end = end.ok_or_else(|| err("external chunk past the end"))?;
                let idlen = be64(bytes, body).ok_or_else(|| err("external chunk"))? as usize;
                if idlen > 256 {
                    return Err(err("external id length"));
                }
                let id = bytes.get(body + 8..body + 8 + idlen).ok_or_else(|| err("external id"))?;
                let blen = be64(bytes, body + 8 + idlen).ok_or_else(|| err("external chunk"))?;
                let start = (body + 16 + idlen).min(end);
                let take = usize::try_from(blen).unwrap_or(usize::MAX).min(end - start);
                let data = bytes.get(start..start + take).ok_or_else(|| err("external body"))?;
                exta.push((id, data));
                pos = end;
            }
            b"CHNKSQLi" => {
                let end = end.ok_or_else(|| err("database past the end"))?;
                sqlite = bytes.get(body..end);
                pos = end;
            }
            b"CHNKFoot" => break,
            _ => {
                let end = end.ok_or_else(|| err("chunk past the end"))?;
                pos = end;
            }
        }
    }
    let sqlite = sqlite.ok_or_else(|| err("no database chunk"))?;
    Ok(Container { exta, sqlite })
}

/// How a bitmap's tiles are packed, from `Offscreen.Attribute`.
struct Attr {
    width: u32,
    height: u32,
    cols: u32,
    rows: u32,
    /// Planar alpha channels before the colour buffer, and bits per one.
    alpha_planes: u32,
    alpha_bits: u32,
    /// Interleaved colour channels, and bits per pixel across them all.
    colour_channels: u32,
    colour_bits: u32,
    tile_w: u32,
    tile_h: u32,
    /// Tiles that are not stored are this: 0 clear, 1 the initial colour.
    default_fill: u32,
    init: Vec<u32>,
}

fn attribute(b: &[u8]) -> Option<Attr> {
    let head = be32(b, 0)? as usize;
    let psize = be32(b, 4)? as usize;
    let isize = be32(b, 8)? as usize;
    let (tag, p) = label(b, head)?;
    if tag != "Parameter" {
        return None;
    }
    let mut v = Vec::with_capacity(20);
    let mut at = p;
    while at + 4 <= head + psize && v.len() < 20 {
        v.push(be32(b, at)?);
        at += 4;
    }
    let (itag, ip) = label(b, head + psize)?;
    let mut init = Vec::new();
    if itag == "InitColor" {
        let mut at = ip;
        while at + 4 <= head + psize + isize && init.len() < 16 {
            init.push(be32(b, at)?);
            at += 4;
        }
    }
    let g = |i: usize| v.get(i).copied().unwrap_or(0);
    Some(Attr {
        width: g(0),
        height: g(1),
        cols: g(2),
        rows: g(3),
        alpha_planes: g(5),
        colour_channels: g(6),
        colour_bits: g(10) >> 5,
        alpha_bits: g(12) >> 5,
        tile_w: g(14),
        tile_h: g(15),
        default_fill: init.get(1).copied().unwrap_or(0),
        init,
    })
}

/// One decoded bitmap: straight RGBA, or a single-plane mask.
struct Bitmap {
    width: u32,
    height: u32,
    /// 4 bytes per pixel for colour, 1 for a mask.
    stride: usize,
    data: Vec<u8>,
}

const MAX_PIXELS: u64 = 64 << 20;

/// Decode an offscreen's tiles into one bitmap.
fn bitmap(attr: &Attr, tiles: Option<&[u8]>) -> Result<Bitmap, ImportError> {
    let (w, h) = (attr.width, attr.height);
    if w == 0 || h == 0 || attr.tile_w == 0 || attr.tile_h == 0 || attr.cols == 0 {
        return Err(err("empty bitmap"));
    }
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(err("bitmap too large"));
    }
    let abytes = (attr.alpha_bits / 8).max(if attr.alpha_planes > 0 { 1 } else { 0 });
    let cbytes_px = attr.colour_bits / 8;
    let cbytes = cbytes_px.checked_div(attr.colour_channels).map_or(0, |b| b.max(1));
    let is_mask = attr.colour_channels == 0;
    let stride = if is_mask { 1 } else { 4 };
    let tile_px = attr.tile_w as usize * attr.tile_h as usize;
    let tile_len = tile_px * (attr.alpha_planes as usize * abytes as usize + cbytes_px as usize);
    let mut data = vec![0u8; w as usize * h as usize * stride];
    // the fill for tiles the file does not store
    let fill: [u8; 4] = if attr.default_fill == 0 {
        [0, 0, 0, 0]
    } else if is_mask {
        [255, 0, 0, 0]
    } else {
        let c = |i: usize| (attr.init.get(i).copied().unwrap_or(0) >> 24) as u8;
        [c(7), c(6), c(5), (attr.init.get(2).copied().unwrap_or(u32::MAX) >> 24) as u8]
    };
    if fill != [0, 0, 0, 0] {
        for px in data.chunks_exact_mut(stride) {
            px.copy_from_slice(fill.get(..stride).unwrap_or(&[]));
        }
    }
    let Some(tiles) = tiles else { return Ok(Bitmap { width: w, height: h, stride, data }) };
    let mut pos = 0usize;
    let mut count = 0usize;
    while pos + 8 <= tiles.len() {
        count += 1;
        if count > 1 << 20 {
            return Err(err("too many tiles"));
        }
        let size = be32(tiles, pos).ok_or_else(|| err("tile"))? as usize;
        let Some((tag, p)) = label(tiles, pos + 4) else { break };
        if tag != "BlockDataBeginChunk" {
            break;
        }
        let index = be32(tiles, p).ok_or_else(|| err("tile"))?;
        let declared = be32(tiles, p + 4).ok_or_else(|| err("tile"))? as usize;
        let present = be32(tiles, p + 16).ok_or_else(|| err("tile"))?;
        if present == 1 {
            let inner = u32::from_le_bytes([
                *tiles.get(p + 24).ok_or_else(|| err("tile"))?,
                *tiles.get(p + 25).ok_or_else(|| err("tile"))?,
                *tiles.get(p + 26).ok_or_else(|| err("tile"))?,
                *tiles.get(p + 27).ok_or_else(|| err("tile"))?,
            ]) as usize;
            let z = tiles.get(p + 28..p + 28 + inner).ok_or_else(|| err("tile data"))?;
            let want = if declared == tile_len { tile_len } else { declared.min(16 << 20) };
            let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(z, want)
                .map_err(|_| err("tile will not inflate"))?;
            if raw.len() != tile_len {
                return Err(err(format!("tile {index}: {} bytes, {tile_len} expected", raw.len())));
            }
            if index / attr.cols >= attr.rows {
                return Err(err(format!(
                    "tile {index} is outside the {} x {} grid",
                    attr.cols, attr.rows
                )));
            }
            let tx = (index % attr.cols) as usize * attr.tile_w as usize;
            let ty = (index / attr.cols) as usize * attr.tile_h as usize;
            let alpha_plane = raw.get(..tile_px * abytes as usize).unwrap_or(&[]);
            let colour =
                raw.get(tile_px * attr.alpha_planes as usize * abytes as usize..).unwrap_or(&[]);
            for row in 0..attr.tile_h as usize {
                let y = ty + row;
                if y >= h as usize {
                    break;
                }
                for col in 0..attr.tile_w as usize {
                    let x = tx + col;
                    if x >= w as usize {
                        break;
                    }
                    let k = row * attr.tile_w as usize + col;
                    // the top byte of each channel, whatever its depth
                    let a = alpha_plane
                        .get(k * abytes as usize + abytes as usize - 1)
                        .copied()
                        .unwrap_or(255);
                    let out = (y * w as usize + x) * stride;
                    if is_mask {
                        if let Some(d) = data.get_mut(out) {
                            *d = a;
                        }
                        continue;
                    }
                    let ch = |c: usize| {
                        colour
                            .get(k * cbytes_px as usize + c * cbytes as usize + cbytes as usize - 1)
                            .copied()
                            .unwrap_or(0)
                    };
                    let (r, g, b) = if attr.colour_channels >= 3 {
                        (ch(2), ch(1), ch(0))
                    } else {
                        let v = ch(0);
                        (v, v, v)
                    };
                    let a = if attr.alpha_planes == 0 { 255 } else { a };
                    if let Some(px) = data.get_mut(out..out + 4) {
                        px.copy_from_slice(&[r, g, b, a]);
                    }
                }
            }
        }
        if size < 8 {
            return Err(err("tile size"));
        }
        pos = pos.checked_add(size).ok_or_else(|| err("tile size"))?;
    }
    Ok(Bitmap { width: w, height: h, stride, data })
}

/// The layer rows this importer reads, by MainId.
struct Row {
    id: i64,
    name: String,
    kind: i64,
    folder: i64,
    first_child: i64,
    next: i64,
    visible: bool,
    opacity: f32,
    clip: bool,
    masking: i64,
    x: i64,
    y: i64,
    mask_dx: i64,
    mask_dy: i64,
    render_mip: i64,
    mask_mip: i64,
}

fn rows(layers: &Table) -> Vec<Row> {
    let int = |r: &[Value], n: &str| layers.get(r, n).and_then(Value::int).unwrap_or(0);
    layers
        .rows
        .iter()
        .map(|r| Row {
            id: int(r, "MainId"),
            name: layers.get(r, "LayerName").and_then(Value::text).unwrap_or("").to_string(),
            kind: int(r, "LayerType"),
            folder: int(r, "LayerFolder"),
            first_child: int(r, "LayerFirstChildIndex"),
            next: int(r, "LayerNextIndex"),
            visible: int(r, "LayerVisibility") & 1 != 0,
            opacity: (int(r, "LayerOpacity") as f32 / 256.0).clamp(0.0, 1.0),
            clip: int(r, "LayerClip") != 0,
            masking: int(r, "LayerMasking"),
            x: int(r, "LayerOffsetX") + int(r, "LayerRenderOffscrOffsetX"),
            y: int(r, "LayerOffsetY") + int(r, "LayerRenderOffscrOffsetY"),
            mask_dx: int(r, "LayerMaskOffsetX") + int(r, "LayerMaskOffscrOffsetX"),
            mask_dy: int(r, "LayerMaskOffsetY") + int(r, "LayerMaskOffscrOffsetY"),
            render_mip: int(r, "LayerRenderMipmap"),
            mask_mip: int(r, "LayerLayerMaskMipmap"),
        })
        .collect()
}

/// Read a .clip into a layered document, bottom layer first, with how many
/// layers were skipped (paper, adjustment layers, layers without pixels).
pub fn read_clip(bytes: &[u8]) -> Result<(Document, usize), ImportError> {
    let c = container(bytes)?;
    let db = Db::open(c.sqlite)?;
    if !db.has("Offscreen") {
        return Err(err("no bitmaps in this file"));
    }
    let canvas = db.table("Canvas")?;
    let crow = canvas.rows.first().ok_or_else(|| err("no canvas"))?;
    let num = |n: &str| canvas.get(crow, n).and_then(Value::real).unwrap_or(0.0);
    let (cw, ch) = (num("CanvasWidth").round(), num("CanvasHeight").round());
    if !(1.0..=65536.0).contains(&cw) || !(1.0..=65536.0).contains(&ch) {
        return Err(err(format!("canvas {cw} x {ch}")));
    }
    let root = canvas.get(crow, "CanvasRootFolder").and_then(Value::int).unwrap_or(0);
    let layers = db.table("Layer")?;
    let all = rows(&layers);
    let by_id = |id: i64| all.iter().find(|r| r.id == id);

    // mipmap -> base offscreen -> (attribute, tiles)
    let mipmap = db.table("Mipmap")?;
    let info = db.table("MipmapInfo")?;
    let offscreen = db.table("Offscreen")?;
    let find = |t: &Table, id: i64| -> Option<Vec<Value>> {
        let k = t.col("MainId")?;
        t.rows.iter().find(|r| r.get(k).and_then(Value::int) == Some(id)).cloned()
    };
    let decode = |mip: i64| -> Option<Bitmap> {
        if mip == 0 {
            return None;
        }
        let m = find(&mipmap, mip)?;
        let base = mipmap.get(&m, "BaseMipmapInfo").and_then(Value::int)?;
        let i = find(&info, base)?;
        let off = info.get(&i, "Offscreen").and_then(Value::int)?;
        let o = find(&offscreen, off)?;
        let attr = attribute(offscreen.get(&o, "Attribute").and_then(Value::bytes)?)?;
        let id = offscreen.get(&o, "BlockData").and_then(Value::bytes);
        let tiles = id.and_then(|id| c.exta.iter().find(|(k, _)| *k == id)).map(|(_, d)| *d);
        match bitmap(&attr, tiles) {
            Ok(b) => Some(b),
            Err(e) => {
                log::warn!("clip: {e}");
                None
            }
        }
    };

    // walk the tree: a folder's first child is its bottom-most layer, and
    // siblings chain upward, so a depth-first walk yields bottom to top
    let mut out: Vec<Layer> = Vec::new();
    let mut skipped = 0usize;
    let mut path: Vec<String> = Vec::new();
    let mut stack: Vec<(i64, usize)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(r) = by_id(root) {
        stack.push((r.first_child, 0));
    }
    while let Some((id, depth)) = stack.pop() {
        path.truncate(depth);
        if id == 0 {
            continue;
        }
        if !seen.insert(id) || seen.len() > 100_000 || depth > 64 {
            return Err(err("layer tree loops"));
        }
        let Some(r) = by_id(id) else { continue };
        // the next sibling continues at this depth once this one is done
        stack.push((r.next, depth));
        if r.folder & 1 != 0 {
            path.push(r.name.clone());
            stack.push((r.first_child, depth + 1));
            continue;
        }
        // paper, adjustment and camera layers draw nothing of the artist's
        if r.kind & (1024 | 4096 | 512) != 0 {
            skipped += 1;
            continue;
        }
        let Some(mut bm) = decode(r.render_mip) else {
            skipped += 1;
            continue;
        };
        if bm.stride != 4 {
            skipped += 1;
            continue;
        }
        if r.masking & 1 != 0
            && let Some(mask) = decode(r.mask_mip)
            && mask.stride == 1
        {
            let (dx, dy) = (r.mask_dx - r.x, r.mask_dy - r.y);
            for y in 0..bm.height as i64 {
                for x in 0..bm.width as i64 {
                    let (mx, my) = (x - dx, y - dy);
                    let m = if (0..mask.width as i64).contains(&mx)
                        && (0..mask.height as i64).contains(&my)
                    {
                        mask.data
                            .get((my as usize) * mask.width as usize + mx as usize)
                            .copied()
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    let at = (y as usize * bm.width as usize + x as usize) * 4 + 3;
                    if let Some(a) = bm.data.get_mut(at) {
                        *a = ((*a as u32 * m as u32 + 127) / 255) as u8;
                    }
                }
            }
        }
        out.push(Layer {
            name: r.name.clone(),
            groups: path.clone(),
            x: r.x as i32,
            y: r.y as i32,
            width: bm.width,
            height: bm.height,
            rgba: std::mem::take(&mut bm.data),
            visible: r.visible,
            opacity: r.opacity,
            clip_to_below: r.clip,
        });
    }
    Ok((Document { width: cw as u32, height: ch as u32, layers: out }, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Option<Vec<u8>> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../ayatsuri2d-research/vendor/clipdecode/assets")
            .join(name);
        std::fs::read(p).ok()
    }

    #[test]
    fn solid_colour_layers_decode_to_their_colours() {
        let Some(bytes) = fixture("colors.clip") else { return };
        let (doc, skipped) = read_clip(&bytes).unwrap();
        assert_eq!((doc.width, doc.height), (24, 24));
        let find = |n: &str| doc.layers.iter().find(|l| l.name == n).unwrap();
        for (name, rgba) in [
            ("layer-green", [0, 255, 0, 255]),
            ("layer-blue", [0, 0, 255, 255]),
            ("layer-red", [255, 0, 0, 255]),
            ("layer-clear", [0, 0, 0, 0]),
        ] {
            let l = find(name);
            assert_eq!((l.width, l.height), (24, 24));
            assert_eq!(&l.rgba[(12 * 24 + 12) * 4..][..4], &rgba, "{name}");
        }
        // the paper layer is the only one left out
        assert_eq!(skipped, 1);
    }

    #[test]
    fn folders_become_group_paths_and_order_is_bottom_first() {
        let Some(bytes) = fixture("layer-types.clip") else { return };
        let (doc, _) = read_clip(&bytes).unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names.first().copied(), Some("raster"));
        let inner = doc.layers.iter().find(|l| l.name == "raster:in-folder").unwrap();
        assert_eq!(inner.groups, vec!["folder".to_string()]);
        assert!(doc.layers.iter().all(|l| !l.name.starts_with("filter:")));
        assert!(
            doc.layers.iter().any(|l| l.name == "vector"),
            "vector layers import as their raster"
        );
    }

    #[test]
    fn a_masked_layer_takes_its_mask_into_alpha() {
        let Some(bytes) = fixture("layer-types.clip") else { return };
        let (doc, _) = read_clip(&bytes).unwrap();
        let plain = doc.layers.iter().find(|l| l.name == "raster").unwrap();
        let masked = doc.layers.iter().find(|l| l.name == "raster:masked").unwrap();
        let alpha = |l: &Layer| l.rgba.as_chunks::<4>().0.iter().map(|p| p[3] as u64).sum::<u64>();
        assert!(alpha(masked) < alpha(plain));
    }

    #[test]
    fn garbage_is_refused_not_crashed() {
        assert!(read_clip(b"CSFCHUNK\0\0\0\0").is_err());
        let mut junk = b"CSFCHUNK".to_vec();
        junk.extend_from_slice(&[0xff; 64]);
        assert!(read_clip(&junk).is_err());
    }
}
