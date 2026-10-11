//! Software-render a model to a PNG: the whole pipeline end to end with no
//! GPU and no UI toolkit in the way. The ground truth for the viewport.
//!
//!     cargo run --release -p ayatsuri --example render -- model.moc3 out.png

#![forbid(unsafe_code)]

use std::path::Path;

struct Tex {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

fn sample(t: &Tex, u: f32, v: f32) -> [f32; 4] {
    // moc3 UVs address the atlas top-down already
    let x = ((u * t.w as f32) as i64).clamp(0, t.w as i64 - 1) as usize;
    let y = ((v * t.h as f32) as i64).clamp(0, t.h as i64 - 1) as usize;
    let at = (y * t.w as usize + x) * 4;
    let px = t.rgba.get(at..at + 4).unwrap_or(&[0, 0, 0, 0]);
    [
        px.first().copied().unwrap_or(0) as f32 / 255.0,
        px.get(1).copied().unwrap_or(0) as f32 / 255.0,
        px.get(2).copied().unwrap_or(0) as f32 / 255.0,
        px.get(3).copied().unwrap_or(0) as f32 / 255.0,
    ]
}

fn main() -> Result<(), String> {
    let model_path = std::env::args().nth(1).ok_or("usage: render <model.moc3> <out.png>")?;
    let out_path = std::env::args().nth(2).unwrap_or_else(|| "render.png".into());
    let bytes = std::fs::read(&model_path).map_err(|e| format!("{model_path}: {e}"))?;
    let model = ayatsuri_import::from_moc3(&bytes).map_err(|e| format!("{model_path}: {e}"))?;

    // textures from the sibling model3.json, same rule as the editor
    let dir = Path::new(&model_path).parent().unwrap_or(Path::new("."));
    let mut textures: Vec<Tex> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut jsons: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".model3.json"))
            })
            .collect();
        jsons.sort();
        if let Some(j) = jsons.first()
            && let Ok(v) =
                serde_json::from_slice::<serde_json::Value>(&std::fs::read(j).unwrap_or_default())
        {
            let texs = v
                .get("FileReferences")
                .and_then(|f| f.get("Textures"))
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default();
            for t in texs {
                let p = dir.join(t.as_str().unwrap_or(""));
                let img =
                    image::open(&p).map_err(|e| format!("{}: {e}", p.display()))?.into_rgba8();
                textures.push(Tex { w: img.width(), h: img.height(), rgba: img.into_raw() });
            }
        }
    }
    println!("{} textures", textures.len());

    let mut puppet = ayatsuri_core::Puppet::new(model);
    puppet.update();

    if std::env::var_os("LIST_DRAWS").is_some() {
        let m = puppet.model();
        for d in puppet.drawables() {
            let part = m
                .art_meshes
                .get(d.mesh)
                .and_then(|am| am.parent_part)
                .and_then(|p| m.parts.get(p as usize))
                .map(|p| p.id.clone())
                .unwrap_or_default();
            println!(
                "{} part={part} opa={:.2} masks={:?} flags={:02x}",
                d.mesh, d.opacity, d.masks, d.flags
            );
        }
        return Ok(());
    }

    let c = puppet.model().canvas;
    let out_w = 700usize;
    let scale = out_w as f32 / c.width.max(1.0);
    let out_h = (c.height * scale) as usize;
    let mut fb = vec![1.0f32; out_w * out_h * 4]; // white canvas

    let sx = |x: f32| (c.origin_x + x * c.pixels_per_unit) * scale;
    let sy = |y: f32| (c.origin_y + y * c.pixels_per_unit) * scale;

    let mut drawn = 0;
    let mut coverage: Vec<f32> = vec![1.0; out_w * out_h];
    let mut coverage_for: Vec<u32> = Vec::new();
    for d in puppet.drawables() {
        if d.opacity <= 0.0 {
            continue;
        }
        let Some(tex) = textures.get(d.texture as usize) else { continue };
        drawn += 1;
        let inverted = d.flags & ayatsuri_format::model::flags::INVERTED_MASK != 0;
        let masked = !d.masks.is_empty();
        if masked && coverage_for != d.masks {
            coverage_for = d.masks.to_vec();
            coverage.iter_mut().for_each(|c| *c = 0.0);
            for &src in d.masks {
                let Some(g) = puppet.mesh_geometry(src as usize) else { continue };
                let Some(mt) = textures.get(g.texture as usize) else { continue };
                raster(&mut coverage, out_w, out_h, g.positions, g.uvs, g.indices, mt, &sx, &sy);
            }
            if std::env::var_os("MASK_DEBUG").is_some() {
                let sum: f32 = coverage.iter().sum();
                let name = &puppet
                    .model()
                    .art_meshes
                    .get(d.mesh)
                    .map(|m| m.id.clone())
                    .unwrap_or_default();
                println!(
                    "mesh {} ({name}) masks {:?} inverted {} coverage_px {:.0}",
                    d.mesh, d.masks, inverted, sum
                );
            }
        }
        let (pos, _) = d.positions.as_chunks::<2>();
        let (uv, _) = d.uvs.as_chunks::<2>();
        let (tris, _) = d.indices.as_chunks::<3>();
        for &[i0, i1, i2] in tris {
            let (Some(&p0), Some(&p1), Some(&p2)) =
                (pos.get(i0 as usize), pos.get(i1 as usize), pos.get(i2 as usize))
            else {
                continue;
            };
            let (Some(&t0), Some(&t1), Some(&t2)) =
                (uv.get(i0 as usize), uv.get(i1 as usize), uv.get(i2 as usize))
            else {
                continue;
            };
            let (x0, y0) = (sx(p0[0]), sy(p0[1]));
            let (x1, y1) = (sx(p1[0]), sy(p1[1]));
            let (x2, y2) = (sx(p2[0]), sy(p2[1]));
            let minx = x0.min(x1).min(x2).floor().max(0.0) as usize;
            let maxx = (x0.max(x1).max(x2).ceil() as usize).min(out_w.saturating_sub(1));
            let miny = y0.min(y1).min(y2).floor().max(0.0) as usize;
            let maxy = (y0.max(y1).max(y2).ceil() as usize).min(out_h.saturating_sub(1));
            let den = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2);
            if den.abs() < 1e-9 {
                continue;
            }
            for py in miny..=maxy {
                for px in minx..=maxx {
                    let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                    let w0 = ((y1 - y2) * (fx - x2) + (x2 - x1) * (fy - y2)) / den;
                    let w1 = ((y2 - y0) * (fx - x2) + (x0 - x2) * (fy - y2)) / den;
                    let w2 = 1.0 - w0 - w1;
                    if (w0 < -0.001 || w1 < -0.001 || w2 < -0.001)
                        && (w0 > 0.001 || w1 > 0.001 || w2 > 0.001)
                    {
                        continue;
                    }
                    let u = t0[0] * w0 + t1[0] * w1 + t2[0] * w2;
                    let v = t0[1] * w0 + t1[1] * w1 + t2[1] * w2;
                    let flat = std::env::args().nth(3).as_deref() == Some("flat");
                    let src = if flat {
                        let k = d.mesh as f32;
                        [(k * 0.37).fract(), (k * 0.61).fract(), (k * 0.83).fract(), 1.0]
                    } else {
                        sample(tex, u, v)
                    };
                    let a = src[3] * d.opacity;
                    if a <= 0.003 {
                        continue;
                    }
                    let at = (py * out_w + px) * 4;
                    if let Some(dst) = fb.get_mut(at..at + 4) {
                        for ch in 0..3 {
                            if let (Some(dc), Some(&sc)) = (dst.get_mut(ch), src.get(ch)) {
                                *dc = sc * a + *dc * (1.0 - a);
                            }
                        }
                        if let Some(da) = dst.get_mut(3) {
                            *da = a + *da * (1.0 - a);
                        }
                    }
                }
            }
        }
    }
    println!("{drawn} meshes drawn");

    let px: Vec<u8> = fb.iter().map(|v| (v * 255.0).clamp(0.0, 255.0) as u8).collect();
    image::RgbaImage::from_raw(out_w as u32, out_h as u32, px)
        .ok_or("bad buffer")?
        .save(&out_path)
        .map_err(|e| e.to_string())?;
    println!("wrote {out_path}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn raster(
    cov: &mut [f32],
    out_w: usize,
    out_h: usize,
    positions: &[f32],
    uvs: &[f32],
    indices: &[u16],
    tex: &Tex,
    sx: &dyn Fn(f32) -> f32,
    sy: &dyn Fn(f32) -> f32,
) {
    let (pos, _) = positions.as_chunks::<2>();
    let (uv, _) = uvs.as_chunks::<2>();
    let (tris, _) = indices.as_chunks::<3>();
    for &[i0, i1, i2] in tris {
        let (Some(&p0), Some(&p1), Some(&p2)) =
            (pos.get(i0 as usize), pos.get(i1 as usize), pos.get(i2 as usize))
        else {
            continue;
        };
        let (Some(&t0), Some(&t1), Some(&t2)) =
            (uv.get(i0 as usize), uv.get(i1 as usize), uv.get(i2 as usize))
        else {
            continue;
        };
        let (x0, y0) = (sx(p0[0]), sy(p0[1]));
        let (x1, y1) = (sx(p1[0]), sy(p1[1]));
        let (x2, y2) = (sx(p2[0]), sy(p2[1]));
        let minx = x0.min(x1).min(x2).floor().max(0.0) as usize;
        let maxx = (x0.max(x1).max(x2).ceil() as usize).min(out_w.saturating_sub(1));
        let miny = y0.min(y1).min(y2).floor().max(0.0) as usize;
        let maxy = (y0.max(y1).max(y2).ceil() as usize).min(out_h.saturating_sub(1));
        let den = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2);
        if den.abs() < 1e-9 {
            continue;
        }
        for py in miny..=maxy {
            for px in minx..=maxx {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let w0 = ((y1 - y2) * (fx - x2) + (x2 - x1) * (fy - y2)) / den;
                let w1 = ((y2 - y0) * (fx - x2) + (x0 - x2) * (fy - y2)) / den;
                let w2 = 1.0 - w0 - w1;
                if (w0 < -0.001 || w1 < -0.001 || w2 < -0.001)
                    && (w0 > 0.001 || w1 > 0.001 || w2 > 0.001)
                {
                    continue;
                }
                let u = t0[0] * w0 + t1[0] * w1 + t2[0] * w2;
                let v = t0[1] * w0 + t1[1] * w1 + t2[1] * w2;
                let a = sample(tex, u, v)[3];
                if let Some(c) = cov.get_mut(py * out_w + px) {
                    *c = c.max(a);
                }
            }
        }
    }
}
