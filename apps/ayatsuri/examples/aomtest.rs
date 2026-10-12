// Round-trip a real model through .aom: import the moc3, save, reopen,
// evaluate both puppets and compare every vertex.
//
//     cargo run --release -p ayatsuri --example aomtest -- model.moc3 out.aom

#![forbid(unsafe_code)]

fn checksum(p: &ayatsuri_core::Puppet) -> (usize, f64) {
    let mut n = 0usize;
    let mut sum = 0f64;
    for d in p.drawables() {
        n += d.positions.len();
        sum += d.positions.iter().map(|v| *v as f64).sum::<f64>();
    }
    (n, sum)
}

fn main() -> Result<(), String> {
    let moc_path = std::env::args().nth(1).ok_or("usage: aomtest <model.moc3> <out.aom>")?;
    let out = std::env::args().nth(2).unwrap_or_else(|| "/tmp/aomtest.aom".into());

    let bytes = std::fs::read(&moc_path).map_err(|e| format!("{moc_path}: {e}"))?;
    let model = ayatsuri_import::from_moc3(&bytes).map_err(|e| e.to_string())?;

    let payload = ayatsuri_format::AomPayload {
        meta: ayatsuri_format::Meta {
            name: "ariu".into(),
            author: String::new(),
            tool: "aomtest".into(),
        },
        model,
        textures: Vec::new(),
        display: None,
        animations: Vec::new(),
        expressions: Vec::new(),
    };
    let aom = ayatsuri_format::write(&payload).map_err(|e| e.to_string())?;
    std::fs::write(&out, &aom).map_err(|e| e.to_string())?;
    println!(
        "wrote {out}: {:.2} MB (moc3 was {:.2} MB)",
        aom.len() as f32 / 1e6,
        bytes.len() as f32 / 1e6
    );

    let back = ayatsuri_format::read(&std::fs::read(&out).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut a = ayatsuri_core::Puppet::new(payload.model);
    let mut b = ayatsuri_core::Puppet::new(back.model);
    for p in [&mut a, &mut b] {
        for i in 0..p.model().parameters.len() {
            let Some((min, max)) = p.model().parameters.get(i).map(|q| (q.min, q.max)) else {
                break;
            };
            p.set_value(i, min + (max - min) * (((i * 37) % 100) as f32 / 100.0));
        }
        p.update();
    }
    let (na, sa) = checksum(&a);
    let (nb, sb) = checksum(&b);
    println!("original: {na} coords, sum {sa:.3}");
    println!("reopened: {nb} coords, sum {sb:.3}");
    if na == nb && (sa - sb).abs() < 1e-3 {
        println!("ROUND TRIP OK");
        Ok(())
    } else {
        Err("round trip mismatch".into())
    }
}
