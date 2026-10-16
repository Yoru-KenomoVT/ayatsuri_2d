// `cargo xtask soak <model.aom> [seconds]`: hammer the paths a long editing
// session hammers, headless, and watch the process's memory. The rule in
// docs/goals.md is that memory must not grow with time; this is how that
// is checked instead of hoped for.
//
// Each round: every parameter swept and the puppet evaluated, an undo-sized
// model clone made and dropped, a texture page decoded and shrunk to a
// proxy the way a residency load does, and a physics step. Resident set
// size is sampled; after a warm-up, growth beyond a few percent fails.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

fn rss_bytes() -> Option<usize> {
    // Linux: statm's second field is resident pages
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: usize = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

fn main() -> Result<(), String> {
    let usage = "usage: soak <model.aom> [seconds]";
    let path = std::env::args().nth(1).ok_or(usage)?;
    let seconds: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let payload = ayatsuri_format::read(&bytes).map_err(|e| e.to_string())?;
    let model = payload.model;
    let pngs: Vec<Vec<u8>> = payload.textures.into_iter().map(|b| b.into_vec()).collect();
    let n_params = model.parameters.len();
    let meta: Vec<(f32, f32, f32)> =
        model.parameters.iter().map(|p| (p.min, p.max, p.default)).collect();
    let mut puppet = ayatsuri_core::Puppet::new(model.clone());
    let mut physics = ayatsuri_core::physics::Physics::new(&model);
    println!(
        "soak: {} meshes, {} deformers, {} parameters, {} texture pages, {seconds}s",
        model.art_meshes.len(),
        model.deformers.len(),
        n_params,
        pngs.len()
    );

    let start = Instant::now();
    let mut round = 0u64;
    let mut samples: Vec<(f32, usize)> = Vec::new();
    let mut next_sample = Instant::now();
    let mut seed = 0x9e37_79b9_u32;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    while start.elapsed() < Duration::from_secs(seconds) {
        round += 1;
        // a sweep over every parameter, as a rigger scrubbing sliders
        for (i, &(min, max, _)) in meta.iter().enumerate() {
            puppet.set_value(i, min + (max - min) * rnd());
        }
        if let Some(ph) = &mut physics {
            let mut values: Vec<f32> = (0..n_params).map(|k| puppet.value(k)).collect();
            ph.step(1.0 / 60.0, &mut values, |k| meta.get(k).copied().unwrap_or((0.0, 1.0, 0.0)));
            for (k, v) in values.iter().enumerate() {
                puppet.set_value(k, *v);
            }
        }
        puppet.update();
        let drawn: usize = puppet.drawables().map(|d| d.positions.len()).sum();
        // an undo step: the whole model cloned, then let go
        let snapshot = model.clone();
        std::hint::black_box(&snapshot);
        drop(snapshot);
        // a residency load: one page decoded and shrunk to its proxy
        if !pngs.is_empty() {
            let page = (round as usize) % pngs.len();
            if let Some(png) = pngs.get(page)
                && let Ok(img) = image::load_from_memory(png)
            {
                let img = img.into_rgba8();
                let long = img.width().max(img.height()).max(1);
                let k = (512.0 / long as f32).min(1.0);
                let small = image::imageops::resize(
                    &img,
                    ((img.width() as f32 * k) as u32).max(1),
                    ((img.height() as f32 * k) as u32).max(1),
                    image::imageops::FilterType::Triangle,
                );
                std::hint::black_box(&small);
            }
        }
        std::hint::black_box(drawn);
        if Instant::now() >= next_sample {
            next_sample = Instant::now() + Duration::from_secs(5);
            if let Some(rss) = rss_bytes() {
                samples.push((start.elapsed().as_secs_f32(), rss));
                println!(
                    "  {:6.1}s  round {round:6}  rss {:7.1} MB",
                    start.elapsed().as_secs_f32(),
                    rss as f32 / 1048576.0
                );
            }
        }
    }
    // judge: the mean of the first quarter (after the first sample, which is
    // the warm-up) against the mean of the last quarter
    if samples.len() < 4 {
        println!("soak: {round} rounds; too short to judge memory");
        return Ok(());
    }
    let q = samples.len() / 4;
    let mean =
        |s: &[(f32, usize)]| s.iter().map(|x| x.1 as f64).sum::<f64>() / s.len().max(1) as f64;
    let early = mean(samples.get(1..1 + q.max(1)).unwrap_or(&[]));
    let late = mean(samples.get(samples.len() - q.max(1)..).unwrap_or(&[]));
    let growth = (late - early) / early.max(1.0);
    println!(
        "soak: {round} rounds, rss {:.1} MB early -> {:.1} MB late ({:+.1}%)",
        early / 1048576.0,
        late / 1048576.0,
        growth * 100.0
    );
    if growth > 0.05 {
        return Err(format!("memory grew {:.1}% over the run", growth * 100.0));
    }
    println!("soak: ok, no growth");
    Ok(())
}
