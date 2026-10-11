// Paint documents end to end: import a .psd or .kra as a fresh model, then
// write it out as a moc3 bundle so the render harness and the reference
// core can both look at the result.
//
//     cargo run --release -p ayatsuri --example paintimport -- art.kra outdir/

#![forbid(unsafe_code)]

fn main() -> Result<(), String> {
    let input = std::env::args().nth(1).ok_or("usage: paintimport <art.psd|art.kra> <outdir>")?;
    let outdir = std::path::PathBuf::from(std::env::args().nth(2).unwrap_or_else(|| ".".into()));
    let bytes = std::fs::read(&input).map_err(|e| format!("{input}: {e}"))?;
    let lower = input.to_lowercase();
    let started = std::time::Instant::now();
    let imported = if lower.ends_with(".kra") {
        ayatsuri_import::from_kra(&bytes)
    } else {
        ayatsuri_import::from_psd(&bytes)
    }
    .map_err(|e| e.to_string())?;
    let m = &imported.model;
    println!(
        "imported in {:.0} ms: canvas {}x{}, {} parts, {} meshes, {} texture pages, {} layers skipped",
        started.elapsed().as_secs_f32() * 1000.0,
        m.canvas.width,
        m.canvas.height,
        m.parts.len(),
        m.art_meshes.len(),
        imported.textures.len(),
        imported.skipped
    );
    if imported.scaled > 0 {
        println!("  {} oversized layer(s) stored downscaled", imported.scaled);
    }
    for (i, n) in imported.part_names.iter().enumerate().take(12) {
        println!("  part {i}: {n}");
    }
    let masked = m.art_meshes.iter().filter(|a| !a.masks.is_empty()).count();
    println!("  {masked} mesh(es) clipped to the layer below");

    std::fs::create_dir_all(outdir.join("tex")).map_err(|e| e.to_string())?;
    let moc = ayatsuri_import::to_moc3(m, ayatsuri_import::ExportVersion::V4_02)
        .map_err(|e| e.to_string())?;
    std::fs::write(outdir.join("paint.moc3"), &moc).map_err(|e| e.to_string())?;
    let mut refs = Vec::new();
    for (i, png) in imported.textures.iter().enumerate() {
        let name = format!("tex/texture_{i:02}.png");
        std::fs::write(outdir.join(&name), png).map_err(|e| e.to_string())?;
        refs.push(serde_json::Value::String(name));
    }
    let model3 = serde_json::json!({
        "Version": 3,
        "FileReferences": { "Moc": "paint.moc3", "Textures": refs },
    });
    std::fs::write(
        outdir.join("paint.model3.json"),
        serde_json::to_vec_pretty(&model3).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!("wrote {}", outdir.join("paint.moc3").display());
    Ok(())
}
