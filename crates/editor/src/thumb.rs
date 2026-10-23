// Small CPU-rendered model previews for the home screen, cached as PNGs
// under the user cache dir. Rendering matches the GPU path closely enough
// for a thumbnail: textured, draw-ordered, soft masks included.

use std::path::{Path, PathBuf};

use ayatsuri_core::Puppet;
use ayatsuri_format::model::flags;
use ayatsuri_render::TexData;

pub fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("ayatsuri2d").join("thumbs"))
}

pub fn cache_path(source: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(source).ok()?;
    let stamp = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs()));
    let stem = source.file_stem()?.to_string_lossy();
    let mut hash = 0u64;
    for b in source.to_string_lossy().bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(b as u64);
    }
    Some(cache_dir()?.join(format!("{stem}-{hash:08x}-{}-{}.png", meta.len(), stamp.unwrap_or(0))))
}

fn texel(t: &TexData, x: i64, y: i64) -> [f32; 4] {
    let x = x.clamp(0, t.width as i64 - 1) as usize;
    let y = y.clamp(0, t.height as i64 - 1) as usize;
    let at = (y * t.width as usize + x) * 4;
    let px = t.rgba.get(at..at + 4).unwrap_or(&[0, 0, 0, 0]);
    [
        px.first().copied().unwrap_or(0) as f32 / 255.0,
        px.get(1).copied().unwrap_or(0) as f32 / 255.0,
        px.get(2).copied().unwrap_or(0) as f32 / 255.0,
        px.get(3).copied().unwrap_or(0) as f32 / 255.0,
    ]
}

/// The texture at (u, v), the four nearest texels blended, so art drawn
/// larger than its page is smooth rather than blocky. Colour is weighted
/// by alpha so a clear neighbour does not darken an edge.
fn sample(t: &TexData, u: f32, v: f32) -> [f32; 4] {
    let x = u * t.width as f32 - 0.5;
    let y = v * t.height as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let mut out = [0.0f32; 4];
    for (dx, dy, w) in [
        (0, 0, (1.0 - fx) * (1.0 - fy)),
        (1, 0, fx * (1.0 - fy)),
        (0, 1, (1.0 - fx) * fy),
        (1, 1, fx * fy),
    ] {
        let p = texel(t, x0 + dx, y0 + dy);
        let wa = w * p[3];
        out[0] += p[0] * wa;
        out[1] += p[1] * wa;
        out[2] += p[2] * wa;
        out[3] += wa;
    }
    if out[3] > 0.0 {
        out[0] /= out[3];
        out[1] /= out[3];
        out[2] /= out[3];
    }
    out
}

struct Raster {
    w: usize,
    h: usize,
    sx: f32,
    sy_off: f32,
    origin_x: f32,
    origin_y: f32,
    ppu: f32,
}

impl Raster {
    fn screen(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (self.origin_x + x * self.ppu) * self.sx,
            (self.origin_y + y * self.ppu) * self.sx + self.sy_off,
        )
    }

    fn tris<F: FnMut(usize, usize, f32, f32, f32, [&[f32; 2]; 3], [&[f32; 2]; 3])>(
        &self,
        positions: &[f32],
        uvs: &[f32],
        indices: &[u16],
        mut f: F,
    ) {
        let (pos, _) = positions.as_chunks::<2>();
        let (uv, _) = uvs.as_chunks::<2>();
        let (tris, _) = indices.as_chunks::<3>();
        for &[i0, i1, i2] in tris {
            let (Some(p0), Some(p1), Some(p2)) =
                (pos.get(i0 as usize), pos.get(i1 as usize), pos.get(i2 as usize))
            else {
                continue;
            };
            let (Some(t0), Some(t1), Some(t2)) =
                (uv.get(i0 as usize), uv.get(i1 as usize), uv.get(i2 as usize))
            else {
                continue;
            };
            let (x0, y0) = self.screen(p0[0], p0[1]);
            let (x1, y1) = self.screen(p1[0], p1[1]);
            let (x2, y2) = self.screen(p2[0], p2[1]);
            let minx = x0.min(x1).min(x2).floor().max(0.0) as usize;
            let maxx = (x0.max(x1).max(x2).ceil() as usize).min(self.w.saturating_sub(1));
            let miny = y0.min(y1).min(y2).floor().max(0.0) as usize;
            let maxy = (y0.max(y1).max(y2).ceil() as usize).min(self.h.saturating_sub(1));
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
                    let neg = w0 < -0.002 || w1 < -0.002 || w2 < -0.002;
                    let pos_ = w0 > 0.002 || w1 > 0.002 || w2 > 0.002;
                    if neg && pos_ {
                        continue;
                    }
                    f(px, py, w0, w1, w2, [t0, t1, t2], [p0, p1, p2]);
                }
            }
        }
    }
}

/// Render the puppet to RGBA at `width` px wide, transparent background.
pub fn render(puppet: &Puppet, textures: &[TexData], width: usize) -> (usize, usize, Vec<u8>) {
    render_aa(puppet, textures, width, 1)
}

/// Pixels a band may hold at the supersampled size; bands keep a tall
/// canvas at 2× within memory.
const BAND_PX: usize = 4 << 20;

/// Render at `width` with `ss` by `ss` samples per pixel averaged, so
/// edges are smooth. The frame is drawn in horizontal bands across all
/// cores, so the supersampled buffer stays small and a tall canvas is
/// done in a fraction of the time.
pub fn render_aa(
    puppet: &Puppet,
    textures: &[TexData],
    width: usize,
    ss: usize,
) -> (usize, usize, Vec<u8>) {
    let c = puppet.model().canvas;
    let ss = ss.clamp(1, 4);
    let width = width.max(1);
    let scale = width as f32 / c.width.max(1.0);
    let height = ((c.height.max(1.0) * scale) as usize).max(1);
    // several bands per core: the art sits in the middle rows, so even
    // bands would leave most cores idle while one did the face
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    let band =
        (BAND_PX / (width * ss * ss).max(1)).min(height.div_ceil(threads * 4)).clamp(4, height);
    let mut out = vec![0u8; width * height * 4];
    std::thread::scope(|scope| {
        for (k, rows_out) in out.chunks_mut(band * width * 4).enumerate() {
            let y0 = k * band;
            scope.spawn(move || {
                let rows = rows_out.len() / (width * 4);
                let r = Raster {
                    w: width * ss,
                    h: rows * ss,
                    sx: scale * ss as f32,
                    sy_off: -((y0 * ss) as f32),
                    origin_x: c.origin_x,
                    origin_y: c.origin_y,
                    ppu: c.pixels_per_unit,
                };
                let fb = draw(&r, puppet, textures);
                resolve(&fb, r.w, ss, rows_out);
            });
        }
    });
    (width, height, out)
}

/// Average each `ss` by `ss` block of `fb` into one pixel of `rows_out`:
/// alpha averaged, colour weighted by alpha so clear samples don't
/// darken an edge.
fn resolve(fb: &[f32], fw: usize, ss: usize, rows_out: &mut [u8]) {
    let width = fw / ss.max(1);
    let n = (ss * ss) as f32;
    let to8 = |v: f32| (v * 255.0).clamp(0.0, 255.0) as u8;
    for (y, row) in rows_out.chunks_mut(width * 4).enumerate() {
        for (x, dst) in row.chunks_mut(4).enumerate() {
            let (mut sa, mut sr, mut sg, mut sb) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for j in 0..ss {
                for i in 0..ss {
                    let at = ((y * ss + j) * fw + x * ss + i) * 4;
                    if let Some(p) = fb.get(at..at + 4) {
                        let a = p.get(3).copied().unwrap_or(0.0);
                        sa += a;
                        sr += p.first().copied().unwrap_or(0.0) * a;
                        sg += p.get(1).copied().unwrap_or(0.0) * a;
                        sb += p.get(2).copied().unwrap_or(0.0) * a;
                    }
                }
            }
            let px = [
                if sa > 0.0 { to8(sr / sa) } else { 0 },
                if sa > 0.0 { to8(sg / sa) } else { 0 },
                if sa > 0.0 { to8(sb / sa) } else { 0 },
                to8(sa / n),
            ];
            dst.copy_from_slice(&px);
        }
    }
}

/// One straight-alpha source pixel drawn into a straight-alpha frame pixel
/// with the mesh's blend.
fn put(dst: &mut [f32], src: [f32; 4], blend: ayatsuri_format::model::Blend) {
    let [r, g, b, a] = src;
    let da = dst.get(3).copied().unwrap_or(0.0);
    let d = |i: usize| dst.get(i).copied().unwrap_or(0.0) * da;
    let out = ayatsuri_core::compose::over(blend, [r * a, g * a, b * a, a], [d(0), d(1), d(2), da]);
    let oa = out[3];
    for (i, v) in out.iter().enumerate() {
        if let Some(slot) = dst.get_mut(i) {
            *slot = if i == 3 {
                oa
            } else if oa > 1e-6 {
                (v / oa).clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
    }
}

/// Draw every drawable into a float RGBA buffer the raster's size.
fn draw(r: &Raster, puppet: &Puppet, textures: &[TexData]) -> Vec<f32> {
    let (width, height) = (r.w, r.h);
    let mut fb = vec![0f32; width * height * 4];
    let mut coverage = vec![0f32; width * height];
    let mut coverage_for: Vec<u32> = Vec::new();

    for d in puppet.drawables() {
        if d.opacity <= 0.0 {
            continue;
        }
        let Some(tex) = textures.get(d.texture as usize) else { continue };
        let inverted = d.flags & flags::INVERTED_MASK != 0;
        let masked = !d.masks.is_empty();
        if masked && coverage_for != d.masks {
            coverage_for = d.masks.to_vec();
            coverage.iter_mut().for_each(|cv| *cv = 0.0);
            for &src in d.masks {
                let Some(g) = puppet.mesh_geometry(src as usize) else { continue };
                let Some(mt) = textures.get(g.texture as usize) else { continue };
                r.tris(g.positions, g.uvs, g.indices, |px, py, w0, w1, w2, uv, _| {
                    let u = uv[0][0] * w0 + uv[1][0] * w1 + uv[2][0] * w2;
                    let v = uv[0][1] * w0 + uv[1][1] * w1 + uv[2][1] * w2;
                    let a = sample(mt, u, v)[3];
                    if let Some(cv) = coverage.get_mut(py * width + px) {
                        *cv = cv.max(a);
                    }
                });
            }
        }
        let opacity = d.opacity;
        r.tris(d.positions, d.uvs, d.indices, |px, py, w0, w1, w2, uv, _| {
            let u = uv[0][0] * w0 + uv[1][0] * w1 + uv[2][0] * w2;
            let v = uv[0][1] * w0 + uv[1][1] * w1 + uv[2][1] * w2;
            let mut src = sample(tex, u, v);
            let mut a = src[3] * opacity;
            if masked {
                let cv = coverage.get(py * width + px).copied().unwrap_or(0.0);
                a *= if inverted { 1.0 - cv } else { cv };
            }
            if a <= 0.004 {
                return;
            }
            src[3] = a;
            let at = (py * width + px) * 4;
            if let Some(dst) = fb.get_mut(at..at + 4) {
                put(dst, src, d.blend);
            }
        });
    }
    fb
}

/// Render just one set of meshes, cropped to their bounds: the Parts
/// palette hover previews. Textures can be any resolution; sampling is
/// normalized, so the 256px palette copies do fine.
pub fn render_filtered(
    puppet: &Puppet,
    textures: &[TexData],
    keep: &dyn Fn(usize) -> bool,
    width: usize,
) -> Option<(usize, usize, Vec<u8>)> {
    let c = puppet.model().canvas;
    // bounds of the kept drawables, in canvas pixels
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for d in puppet.drawables() {
        if !keep(d.mesh) {
            continue;
        }
        let (pts, _) = d.positions.as_chunks::<2>();
        for p in pts {
            let px = c.origin_x + p[0] * c.pixels_per_unit;
            let py = c.origin_y + p[1] * c.pixels_per_unit;
            x0 = x0.min(px);
            y0 = y0.min(py);
            x1 = x1.max(px);
            y1 = y1.max(py);
        }
    }
    if x0 > x1 || y0 > y1 {
        return None;
    }
    let pad = ((x1 - x0).max(y1 - y0) * 0.04).max(2.0);
    let (x0, y0, x1, y1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
    let scale = width as f32 / (x1 - x0).max(1.0);
    let height = (((y1 - y0) * scale) as usize).clamp(1, width * 4);
    let r = Raster {
        w: width,
        h: height,
        sx: scale,
        sy_off: 0.0,
        origin_x: c.origin_x - x0,
        origin_y: c.origin_y - y0,
        ppu: c.pixels_per_unit,
    };
    let mut fb = vec![0f32; width * height * 4];
    let mut coverage = vec![0f32; width * height];
    let mut coverage_for: Vec<u32> = Vec::new();
    for d in puppet.drawables() {
        if !keep(d.mesh) || d.opacity <= 0.0 {
            continue;
        }
        let Some(tex) = textures.get(d.texture as usize) else { continue };
        let inverted = d.flags & flags::INVERTED_MASK != 0;
        let masked = !d.masks.is_empty();
        if masked && coverage_for != d.masks {
            coverage_for = d.masks.to_vec();
            coverage.iter_mut().for_each(|cv| *cv = 0.0);
            for &src in d.masks {
                let Some(g) = puppet.mesh_geometry(src as usize) else { continue };
                let Some(mt) = textures.get(g.texture as usize) else { continue };
                r.tris(g.positions, g.uvs, g.indices, |px, py, w0, w1, w2, uv, _| {
                    let u = uv[0][0] * w0 + uv[1][0] * w1 + uv[2][0] * w2;
                    let v = uv[0][1] * w0 + uv[1][1] * w1 + uv[2][1] * w2;
                    let a = sample(mt, u, v)[3];
                    if let Some(cv) = coverage.get_mut(py * width + px) {
                        *cv = cv.max(a);
                    }
                });
            }
        }
        let opacity = d.opacity;
        r.tris(d.positions, d.uvs, d.indices, |px, py, w0, w1, w2, uv, _| {
            let u = uv[0][0] * w0 + uv[1][0] * w1 + uv[2][0] * w2;
            let v = uv[0][1] * w0 + uv[1][1] * w1 + uv[2][1] * w2;
            let mut src = sample(tex, u, v);
            let mut a = src[3] * opacity;
            if masked {
                let cv = coverage.get(py * width + px).copied().unwrap_or(0.0);
                a *= if inverted { 1.0 - cv } else { cv };
            }
            if a <= 0.004 {
                return;
            }
            src[3] = a;
            let at = (py * width + px) * 4;
            if let Some(dst) = fb.get_mut(at..at + 4) {
                put(dst, src, d.blend);
            }
        });
    }
    let px: Vec<u8> = fb.iter().map(|v| (v * 255.0).clamp(0.0, 255.0) as u8).collect();
    Some((width, height, px))
}

/// Make sure a cached thumbnail exists for this source file; render one if
/// not. Returns the cache path when available.
pub fn ensure(puppet: &Puppet, textures: &[TexData], source: &Path) -> Option<PathBuf> {
    let path = cache_path(source)?;
    if path.exists() {
        return Some(path);
    }
    if let Err(e) = std::fs::create_dir_all(path.parent()?) {
        log::warn!("thumbnail: could not make the cache folder: {e}");
        return None;
    }
    let started = std::time::Instant::now();
    let (w, h, rgba) = render(puppet, textures, 160);
    let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba)?;
    if let Err(e) = img.save(&path) {
        log::warn!("thumbnail: could not save {}: {e}", path.display());
        return None;
    }
    log::debug!(
        "thumbnail rendered in {:.0} ms -> {}",
        started.elapsed().as_secs_f32() * 1000.0,
        path.display()
    );
    Some(path)
}
