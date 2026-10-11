// Krita documents. A .kra is a zip: maindoc.xml holds the layer tree, and
// each paint layer's pixels live in "<image>/layers/<file>" in Krita's
// tiled form, documented with the format:
//
//     VERSION 2
//     TILEWIDTH 64
//     TILEHEIGHT 64
//     PIXELSIZE 4
//     DATA <tiles>
//     <x>,<y>,LZF,<bytes>
//     <bytes of tile>      first byte 1 = LZF compressed, 0 = raw
//
// Tile pixels are stored channel-planar (all B, then G, R, A for 8-bit
// RGBA), so they are re-interleaved after decompression. Only 8-bit RGBA
// layers are read; other colour models are skipped and counted.

use std::io::Read;

use crate::ImportError;
use crate::layers::{Document, Layer};

fn err(msg: impl Into<String>) -> ImportError {
    ImportError::Export(format!("kra: {}", msg.into()))
}

/// liblzf's decompressor, bounds-checked. Literal runs are `ctrl < 32`;
/// otherwise a back reference of `len + 2` bytes.
fn lzf_decompress(input: &[u8], out_len: usize) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(out_len);
    let mut ip = 0usize;
    while ip < input.len() {
        let ctrl = *input.get(ip)? as usize;
        ip += 1;
        if ctrl < 32 {
            let run = ctrl + 1;
            out.extend_from_slice(input.get(ip..ip + run)?);
            ip += run;
        } else {
            let mut len = ctrl >> 5;
            if len == 7 {
                len += *input.get(ip)? as usize;
                ip += 1;
            }
            let back = ((ctrl & 0x1f) << 8) + *input.get(ip)? as usize + 1;
            ip += 1;
            let start = out.len().checked_sub(back)?;
            // overlapping copies are legal, so byte by byte
            for k in 0..len + 2 {
                let b = *out.get(start + k)?;
                out.push(b);
            }
        }
        if out.len() > out_len {
            return None;
        }
    }
    Some(out)
}

fn attr(tag: &str, key: &str) -> Option<String> {
    let needle = format!(" {key}=\"");
    let at = tag.find(&needle)? + needle.len();
    let rest = tag.get(at..)?;
    let end = rest.find('"')?;
    let raw = rest.get(..end)?;
    Some(
        raw.replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

/// One `<layer>` entry from maindoc.xml, document order (top first).
struct Entry {
    name: String,
    file: String,
    groups: Vec<String>,
    x: i32,
    y: i32,
    visible: bool,
    opacity: f32,
    paint: bool,
    rgba8: bool,
    inherit_alpha: bool,
}

/// A tiny tag walk: `<layer ...>` opens (self-closing or not), `</layer>`
/// closes, and group layers push their name for everything inside them.
fn parse_layers(xml: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut groups: Vec<String> = Vec::new();
    // per open non-self-closing layer: whether it pushed a group name
    let mut open: Vec<bool> = Vec::new();
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        let Some(after) = rest.get(lt..) else { break };
        let Some(gt) = after.find('>') else { break };
        let tag = after.get(..=gt).unwrap_or("");
        rest = after.get(gt + 1..).unwrap_or("");
        if tag.starts_with("</layer>") {
            if open.pop() == Some(true) {
                groups.pop();
            }
            continue;
        }
        if !tag.starts_with("<layer ") {
            continue;
        }
        let self_closing = tag.ends_with("/>");
        let kind = attr(tag, "nodetype").unwrap_or_default();
        let name = attr(tag, "name").unwrap_or_default();
        if kind == "grouplayer" {
            if !self_closing {
                groups.push(name);
                open.push(true);
            }
            continue;
        }
        out.push(Entry {
            name,
            file: attr(tag, "filename").unwrap_or_default(),
            groups: groups.clone(),
            x: attr(tag, "x").and_then(|v| v.parse().ok()).unwrap_or(0),
            y: attr(tag, "y").and_then(|v| v.parse().ok()).unwrap_or(0),
            visible: attr(tag, "visible").is_none_or(|v| v != "0"),
            opacity: attr(tag, "opacity").and_then(|v| v.parse::<f32>().ok()).unwrap_or(255.0)
                / 255.0,
            paint: kind == "paintlayer",
            rgba8: attr(tag, "colorspacename").is_none_or(|c| c == "RGBA"),
            inherit_alpha: attr(tag, "inheritalpha").is_some_and(|v| v == "1"),
        });
        if !self_closing {
            open.push(false);
        }
    }
    out
}

/// Decode one tiled layer into a canvas-placed RGBA buffer cropped to the
/// tiles it actually has.
fn read_tiles(data: &[u8], ox: i32, oy: i32) -> Option<(i32, i32, u32, u32, Vec<u8>)> {
    // header lines, then tile records
    let mut pos = 0usize;
    fn line(data: &[u8], pos: &mut usize) -> Option<String> {
        let rest = data.get(*pos..)?;
        let nl = rest.iter().position(|&b| b == b'\n')?;
        let s = String::from_utf8_lossy(rest.get(..nl)?).to_string();
        *pos += nl + 1;
        Some(s)
    }
    let (mut tw, mut th, mut px, mut count) = (64usize, 64usize, 4usize, 0usize);
    for _ in 0..5 {
        let l = line(data, &mut pos)?;
        let mut it = l.split_whitespace();
        match (it.next(), it.next()) {
            (Some("TILEWIDTH"), Some(v)) => tw = v.parse().ok()?,
            (Some("TILEHEIGHT"), Some(v)) => th = v.parse().ok()?,
            (Some("PIXELSIZE"), Some(v)) => px = v.parse().ok()?,
            (Some("DATA"), Some(v)) => count = v.parse().ok()?,
            _ => {}
        }
    }
    if px != 4 || tw == 0 || th == 0 || tw > 1024 || th > 1024 {
        return None;
    }
    let tile_bytes = tw * th * px;
    let mut tiles: Vec<(i32, i32, Vec<u8>)> = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        let header = line(data, &mut pos)?;
        let mut f = header.split(',');
        let (x, y) = (f.next()?.trim().parse::<i32>().ok()?, f.next()?.trim().parse::<i32>().ok()?);
        let _codec = f.next()?;
        let size: usize = f.next()?.trim().parse().ok()?;
        let body = data.get(pos..pos + size)?;
        pos += size;
        let (flag, payload) = body.split_first()?;
        let planar =
            if *flag == 1 { lzf_decompress(payload, tile_bytes)? } else { payload.to_vec() };
        if planar.len() < tile_bytes {
            return None;
        }
        tiles.push((x, y, planar));
    }
    if tiles.is_empty() {
        return None;
    }
    let x0 = tiles.iter().map(|t| t.0).min()?;
    let y0 = tiles.iter().map(|t| t.1).min()?;
    let x1 = tiles.iter().map(|t| t.0).max()? + tw as i32;
    let y1 = tiles.iter().map(|t| t.1).max()? + th as i32;
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if w * h > 64 * 1024 * 1024 {
        return None;
    }
    let mut rgba = vec![0u8; w * h * 4];
    let plane = tw * th;
    for (tx, ty, planar) in &tiles {
        for py in 0..th {
            for pxi in 0..tw {
                let i = py * tw + pxi;
                // planar BGRA back to interleaved RGBA
                let (b, g, r, a) = (
                    *planar.get(i)?,
                    *planar.get(plane + i)?,
                    *planar.get(2 * plane + i)?,
                    *planar.get(3 * plane + i)?,
                );
                let cx = (*tx - x0) as usize + pxi;
                let cy = (*ty - y0) as usize + py;
                let at = (cy * w + cx) * 4;
                if let Some(dst) = rgba.get_mut(at..at + 4) {
                    dst.copy_from_slice(&[r, g, b, a]);
                }
            }
        }
    }
    Some((x0 + ox, y0 + oy, w as u32, h as u32, rgba))
}

/// Read a .kra into a layered document, bottom layer first.
pub fn read_kra(bytes: &[u8]) -> Result<(Document, usize), ImportError> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| err(format!("not a zip: {e}")))?;
    let mut xml = String::new();
    zip.by_name("maindoc.xml")
        .map_err(|e| err(format!("maindoc.xml: {e}")))?
        .take(16 << 20)
        .read_to_string(&mut xml)
        .map_err(|e| err(format!("maindoc.xml: {e}")))?;

    let image_tag = xml
        .find("<IMAGE ")
        .and_then(|at| xml.get(at..).and_then(|r| r.find('>').and_then(|e| r.get(..=e))))
        .ok_or_else(|| err("no IMAGE element"))?
        .to_string();
    let width: u32 = attr(&image_tag, "width").and_then(|v| v.parse().ok()).unwrap_or(0);
    let height: u32 = attr(&image_tag, "height").and_then(|v| v.parse().ok()).unwrap_or(0);
    let image_name = attr(&image_tag, "name").unwrap_or_default();

    let mut skipped = 0usize;
    let mut layers = Vec::new();
    for e in parse_layers(&xml) {
        if !e.paint || !e.rgba8 {
            skipped += 1;
            continue;
        }
        let path = format!("{image_name}/layers/{}", e.file);
        let mut data = Vec::new();
        let read = zip
            .by_name(&path)
            .map_err(|er| er.to_string())
            .and_then(|f| f.take(512 << 20).read_to_end(&mut data).map_err(|er| er.to_string()));
        if read.is_err() {
            skipped += 1;
            continue;
        }
        let Some((x, y, w, h, rgba)) = read_tiles(&data, e.x, e.y) else {
            skipped += 1;
            continue;
        };
        layers.push(Layer {
            name: e.name,
            groups: e.groups,
            x,
            y,
            width: w,
            height: h,
            rgba,
            visible: e.visible,
            opacity: e.opacity,
            clip_to_below: e.inherit_alpha,
        });
    }
    // maindoc lists the topmost layer first
    layers.reverse();
    Ok((Document { width, height, layers }, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lzf_literals_and_backrefs() {
        // "abcabcabc": literal "abc", then a 6-byte back reference of 3
        let compressed = [2u8, b'a', b'b', b'c', 4 << 5, 2];
        assert_eq!(lzf_decompress(&compressed, 9).as_deref(), Some(&b"abcabcabc"[..]));
        // a reference before the start is rejected, not a panic
        assert_eq!(lzf_decompress(&[0x20, 9], 9), None);
    }

    #[test]
    fn layer_tree_walk() {
        let xml = r#"<layers>
            <layer name="Top" filename="layer3" nodetype="paintlayer" x="4" y="5" visible="1" opacity="128"/>
            <layer name="Face" nodetype="grouplayer" filename="layer2">
              <layers>
                <layer name="Eye &amp; lid" filename="layer1" nodetype="paintlayer" inheritalpha="1"/>
              </layers>
            </layer>
            <layer name="Back" filename="layer0" nodetype="paintlayer" visible="0"/>
        </layers>"#;
        let e = parse_layers(xml);
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].name, "Top");
        assert_eq!((e[0].x, e[0].y), (4, 5));
        assert!((e[0].opacity - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(e[1].name, "Eye & lid");
        assert_eq!(e[1].groups, vec!["Face".to_string()]);
        assert!(e[1].inherit_alpha);
        assert!(e[2].groups.is_empty());
        assert!(!e[2].visible);
    }
}
