// The moc3 round trip: import a real model, export it, reparse the export
// with the hostile-input reader, reimport, and compare every drawable under
// a deterministic pose sweep.
//
//     cargo run --release -p ayatsuri --example mocround -- model.moc3 [out.moc3]

#![forbid(unsafe_code)]

fn sweep(p: &mut ayatsuri_core::Puppet) {
    for i in 0..p.model().parameters.len() {
        let Some((min, max)) = p.model().parameters.get(i).map(|q| (q.min, q.max)) else { break };
        p.set_value(i, min + (max - min) * (((i * 37) % 100) as f32 / 100.0));
    }
    p.update();
}

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: mocround <model.moc3> [out.moc3]")?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let model = ayatsuri_import::from_moc3(&bytes).map_err(|e| e.to_string())?;

    let version = match std::env::var("MOC_VERSION").as_deref() {
        Ok("3.0") => ayatsuri_import::ExportVersion::V3_00,
        Ok("3.3") => ayatsuri_import::ExportVersion::V3_03,
        Ok("4.0") => ayatsuri_import::ExportVersion::V4_00,
        Ok("5.0") => ayatsuri_import::ExportVersion::V5_00,
        Ok("5.3") => ayatsuri_import::ExportVersion::V5_03,
        _ => ayatsuri_import::ExportVersion::V4_02,
    };
    let out = ayatsuri_import::to_moc3(&model, version).map_err(|e| e.to_string())?;
    println!("target version {}", version.label());
    println!("exported {} bytes (original {})", out.len(), bytes.len());
    if let Some(dst) = std::env::args().nth(2) {
        std::fs::write(&dst, &out).map_err(|e| e.to_string())?;
        println!("wrote {dst}");
    }

    // the reader must accept our own output
    let reparsed = openmoc3::Moc3::parse(&out).map_err(|e| format!("reparse: {e:?}"))?;
    println!("reparse ok, version {:?}", reparsed.version());
    let model2 = ayatsuri_import::from_moc3(&out).map_err(|e| format!("reimport: {e}"))?;

    let mut a = ayatsuri_core::Puppet::new(model);
    let mut b = ayatsuri_core::Puppet::new(model2);
    sweep(&mut a);
    sweep(&mut b);

    let da: Vec<_> = a.drawables().map(|d| (d.mesh, d.positions.to_vec())).collect();
    let db: Vec<_> = b.drawables().map(|d| (d.mesh, d.positions.to_vec())).collect();
    if da.len() != db.len() {
        return Err(format!("drawable count differs: {} vs {}", da.len(), db.len()));
    }
    let mut worst = 0.0f32;
    for ((ma, pa), (mb, pb)) in da.iter().zip(db.iter()) {
        if ma != mb || pa.len() != pb.len() {
            return Err(format!("drawable order or size differs at mesh {ma} vs {mb}"));
        }
        for (x, y) in pa.iter().zip(pb.iter()) {
            worst = worst.max((x - y).abs());
        }
    }
    println!("drawables: {}  worst coordinate delta: {worst}", da.len());
    if worst == 0.0 {
        println!("MOC3 ROUND TRIP EXACT");
        Ok(())
    } else if worst < 1e-4 {
        println!("MOC3 ROUND TRIP OK (within 1e-4)");
        Ok(())
    } else {
        Err(format!("round trip drifted by {worst}"))
    }
}
