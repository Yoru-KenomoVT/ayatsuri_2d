// Run the outline auto mesher on real artwork and draw the result: each
// chosen layer's texture patch with the wireframe over it, side by side.
//
//     cargo run --release -p ayatsuri --example automesh -- art.psd out.png [mesh...]

#![forbid(unsafe_code)]

fn main() -> Result<(), String> {
    let path =
        std::env::args().nth(1).ok_or("usage: automesh <art.psd|art.kra> <out.png> [mesh...]")?;
    let out = std::env::args().nth(2).unwrap_or_else(|| "automesh.png".into());
    let picks: Vec<usize> = std::env::args().skip(3).filter_map(|a| a.parse().ok()).collect();
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let imported = if path.to_lowercase().ends_with(".kra") {
        ayatsuri_import::from_kra(&bytes)
    } else {
        ayatsuri_import::from_psd(&bytes)
    }
    .map_err(|e| e.to_string())?;
    let pages: Vec<image::RgbaImage> = imported
        .textures
        .iter()
        .filter_map(|p| image::load_from_memory(p).ok().map(|i| i.to_rgba8()))
        .collect();
    let picks = if picks.is_empty() { vec![0, 1, 2, 3] } else { picks };
    let params = ayatsuri_core::automesh::Params::default();

    let mut panels: Vec<image::RgbaImage> = Vec::new();
    for mi in picks {
        let Some(am) = imported.model.art_meshes.get(mi) else { continue };
        let Some(page) = pages.get(am.texture as usize) else { continue };
        let (tw, th) = (page.width() as f32, page.height() as f32);
        let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in am.uvs.chunks(2) {
            if let [u, v] = p {
                u0 = u0.min(*u);
                v0 = v0.min(*v);
                u1 = u1.max(*u);
                v1 = v1.max(*v);
            }
        }
        let pad = params.margin + 2.0;
        let x0 = (u0 * tw - pad).max(0.0) as u32;
        let y0 = (v0 * th - pad).max(0.0) as u32;
        let x1 = (u1 * tw + pad).min(tw) as u32;
        let y1 = (v1 * th + pad).min(th) as u32;
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut alpha = vec![0u8; w * h];
        let mut patch =
            image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([40, 40, 46, 255]));
        for y in 0..h {
            for x in 0..w {
                if let Some(px) = page.get_pixel_checked(x0 + x as u32, y0 + y as u32) {
                    if let Some(a) = alpha.get_mut(y * w + x) {
                        *a = px.0[3];
                    }
                    let k = px.0[3] as f32 / 255.0;
                    let mix = |c: u8, b: u8| (c as f32 * k + b as f32 * (1.0 - k)) as u8;
                    patch.put_pixel(
                        x as u32,
                        y as u32,
                        image::Rgba([mix(px.0[0], 40), mix(px.0[1], 40), mix(px.0[2], 46), 255]),
                    );
                }
            }
        }
        let started = std::time::Instant::now();
        let mesh = ayatsuri_core::automesh::generate(&alpha, w, h, &params);
        println!(
            "mesh {mi}: {w}x{h} px, {} points, {} triangles, {:.0} ms",
            mesh.points.len(),
            mesh.triangles.len(),
            started.elapsed().as_secs_f32() * 1000.0
        );
        let mut line = |a: (f32, f32), b: (f32, f32)| {
            let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) as usize).max(1);
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                if x >= 0.0 && y >= 0.0 && (x as u32) < patch.width() && (y as u32) < patch.height()
                {
                    patch.put_pixel(x as u32, y as u32, image::Rgba([240, 142, 30, 255]));
                }
            }
        };
        for t in &mesh.triangles {
            let p = |i: u16| mesh.points.get(i as usize).copied().unwrap_or((0.0, 0.0));
            line(p(t[0]), p(t[1]));
            line(p(t[1]), p(t[2]));
            line(p(t[2]), p(t[0]));
        }
        panels.push(patch);
    }
    // lay the panels out in a row, scaled to a common height
    let target_h = 420u32;
    let scaled: Vec<image::RgbaImage> = panels
        .iter()
        .map(|p| {
            let k = target_h as f32 / p.height().max(1) as f32;
            image::imageops::resize(
                p,
                ((p.width() as f32 * k) as u32).max(1),
                target_h,
                image::imageops::FilterType::Triangle,
            )
        })
        .collect();
    let total_w: u32 = scaled.iter().map(|p| p.width() + 8).sum();
    let mut sheet =
        image::RgbaImage::from_pixel(total_w.max(1), target_h, image::Rgba([24, 24, 28, 255]));
    let mut x = 0i64;
    for p in &scaled {
        image::imageops::replace(&mut sheet, p, x, 0);
        x += p.width() as i64 + 8;
    }
    sheet.save(&out).map_err(|e| e.to_string())?;
    println!("wrote {out}");
    Ok(())
}
