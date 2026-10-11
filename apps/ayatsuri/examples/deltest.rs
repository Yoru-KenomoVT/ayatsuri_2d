// Mesh vertex deletion under stress: densely re-mesh real layers, delete
// vertices one at a time, and rebuild and evaluate the puppet after every
// step the way the editor does. Any panic prints with the step that hit it.
//
//     cargo run --release -p ayatsuri --example deltest -- art.psd

#![forbid(unsafe_code)]

fn grid(uvs: &[f32], step: f32) -> (Vec<f32>, Vec<u16>) {
    let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in uvs.chunks(2) {
        if let [u, v] = p {
            u0 = u0.min(*u);
            v0 = v0.min(*v);
            u1 = u1.max(*u);
            v1 = v1.max(*v);
        }
    }
    let cols = (((u1 - u0) / step).ceil() as usize).clamp(1, 60);
    let rows = (((v1 - v0) / step).ceil() as usize).clamp(1, 60);
    let mut out_uv = Vec::new();
    for r in 0..=rows {
        for c in 0..=cols {
            out_uv.push(u0 + (u1 - u0) * c as f32 / cols as f32);
            out_uv.push(v0 + (v1 - v0) * r as f32 / rows as f32);
        }
    }
    let mut idx = Vec::new();
    let w = cols + 1;
    for r in 0..rows {
        for c in 0..cols {
            let a = (r * w + c) as u16;
            let b = a + 1;
            let d = a + w as u16;
            idx.extend_from_slice(&[a, b, d + 1, a, d + 1, d]);
        }
    }
    (out_uv, idx)
}

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: deltest <art.psd|art.kra|model.moc3>")?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut model = if path.to_lowercase().ends_with(".psd") {
        ayatsuri_import::from_psd(&bytes).map_err(|e| e.to_string())?.model
    } else if path.to_lowercase().ends_with(".kra") {
        ayatsuri_import::from_kra(&bytes).map_err(|e| e.to_string())?.model
    } else {
        ayatsuri_import::from_moc3(&bytes).map_err(|e| e.to_string())?
    };
    let meshes = model.art_meshes.len().min(6);
    for mi in 0..meshes {
        let Some(src) = model.art_meshes.get(mi) else { continue };
        let (uvs, idx) = grid(&src.uvs, 0.004);
        if !ayatsuri_core::edit::remesh(&mut model, mi, &uvs, &idx) {
            println!("mesh {mi}: remesh refused");
            continue;
        }
        let start = model.art_meshes.get(mi).map_or(0, |m| m.vertex_count);
        // delete from the middle, the front and the back, many times over
        let mut step = 0usize;
        loop {
            let n = model.art_meshes.get(mi).map_or(0, |m| m.vertex_count as usize);
            if n <= 4 || step > 400 {
                break;
            }
            let victim = match step % 3 {
                0 => n / 2,
                1 => 0,
                _ => n - 1,
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut m = model.clone();
                let ok = ayatsuri_core::edit::delete_vertex(&mut m, mi, victim);
                let mut p = ayatsuri_core::Puppet::new(m.clone());
                p.update();
                let _ = p.drawables().count();
                let _ = p.mesh_geometry(mi).map(|g| g.positions.len());
                let _ = p.mesh_local_positions(mi);
                (ok, m)
            }));
            match result {
                Ok((true, m)) => model = m,
                Ok((false, _)) => break,
                Err(e) => {
                    let msg = e
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default();
                    return Err(format!(
                        "PANIC mesh {mi} step {step} deleting vertex {victim} of {n}: {msg}"
                    ));
                }
            }
            step += 1;
        }
        let Some(m) = model.art_meshes.get(mi) else { continue };
        println!(
            "mesh {mi}: {start} -> {} vertices, {} indices, uvs {}, positions {}",
            m.vertex_count,
            m.indices.len(),
            m.uvs.len(),
            m.positions.len()
        );
    }
    println!("DELETE STRESS OK");
    Ok(())
}
