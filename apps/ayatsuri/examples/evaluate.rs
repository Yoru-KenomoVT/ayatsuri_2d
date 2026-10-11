//! Headless end-to-end check: import a `.moc3`, evaluate the puppet, sweep a
//! parameter, and time the frame loop.
//!
//!     cargo run --release -p ayatsuri --example evaluate -- model.moc3

#![forbid(unsafe_code)]

fn bbox(puppet: &ayatsuri_core::Puppet) -> (f32, f32, f32, f32) {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for d in puppet.drawables() {
        let (pairs, _) = d.positions.as_chunks::<2>();
        for &[x, y] in pairs {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    (min_x, min_y, max_x, max_y)
}

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: evaluate <model.moc3>")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let model = ayatsuri_import::from_moc3(&bytes).map_err(|e| format!("{path}: {e}"))?;
    println!(
        "{path}: {} parts, {} deformers, {} meshes, {} parameters",
        model.parts.len(),
        model.deformers.len(),
        model.art_meshes.len(),
        model.parameters.len()
    );

    let mut puppet = ayatsuri_core::Puppet::new(model);
    puppet.update();
    let visible = puppet.drawables().count();
    let verts: usize = puppet.drawables().map(|d| d.positions.len() / 2).sum();
    let (x0, y0, x1, y1) = bbox(&puppet);
    println!(
        "  default pose: {visible} visible meshes, {verts} vertices, bbox ({x0:.2}, {y0:.2}) to ({x1:.2}, {y1:.2})"
    );

    // sweep every parameter to its max and count which ones move vertices
    let n_params = puppet.model().parameters.len();
    let mut movers = 0;
    let mut dead: Vec<String> = Vec::new();
    for p in 0..n_params {
        let (id, min, max, default) = match puppet.model().parameters.get(p) {
            Some(q) => (q.id.clone(), q.min, q.max, q.default),
            None => break,
        };
        puppet.set_value(p, default);
        puppet.update();
        let snapshot = |pp: &ayatsuri_core::Puppet| -> Vec<f32> {
            pp.drawables()
                .flat_map(|d| d.positions.iter().copied().chain([d.opacity]).collect::<Vec<_>>())
                .collect()
        };
        let before = snapshot(&puppet);
        puppet.set_value(p, if default < max { max } else { min });
        puppet.update();
        let after = snapshot(&puppet);
        let moved = before.iter().zip(&after).filter(|(a, b)| (**a - **b).abs() > 1e-4).count();
        if moved > 0 || before.len() != after.len() {
            movers += 1;
        } else {
            dead.push(id);
        }
        puppet.set_value(p, default);
    }
    println!("  {movers} of {n_params} parameters move vertices or visibility");
    if !dead.is_empty() {
        println!("  dead: {}", dead.join(", "));
    }
    puppet.update();

    // p99.9 is the number the project cares about (docs/goals.md)
    let mut times: Vec<f64> = Vec::with_capacity(1000);
    for i in 0..1000u32 {
        let t = (i as f32) / 1000.0;
        for p in 0..puppet.model().parameters.len() {
            let (min, max) =
                puppet.model().parameters.get(p).map(|q| (q.min, q.max)).unwrap_or((0.0, 1.0));
            puppet.set_value(
                p,
                min + (max - min) * (0.5 + 0.5 * (t * std::f32::consts::TAU + p as f32).sin()),
            );
        }
        let start = std::time::Instant::now();
        puppet.update();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    let mean: f64 = times.iter().sum::<f64>() / times.len() as f64;
    let p999 = times.get(998).copied().unwrap_or(0.0);
    println!("  1000 updates, every parameter animated: mean {mean:.3} ms, p99.9 {p999:.3} ms");
    Ok(())
}
