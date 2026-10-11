//! Dump every mesh's evaluated vertices at default parameters, in the same
//! format as the PurismCore reference dumper, for numerical comparison.

#![forbid(unsafe_code)]

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: dump <model.moc3>")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let model = ayatsuri_import::from_moc3(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let mut puppet = ayatsuri_core::Puppet::new(model);
    if std::env::args().nth(2).as_deref() == Some("pose") {
        // the same deterministic pose as the reference dumper
        for i in 0..puppet.model().parameters.len() {
            let Some((min, max)) = puppet.model().parameters.get(i).map(|p| (p.min, p.max)) else {
                break;
            };
            let f = ((i * 37) % 100) as f32 / 100.0;
            puppet.set_value(i, min + (max - min) * f);
        }
    }
    puppet.update();

    if std::env::args().nth(2).as_deref() == Some("order") {
        for d in puppet.drawables() {
            println!("{}", d.mesh);
        }
        return Ok(());
    }

    let n = puppet.model().art_meshes.len();
    for mi in 0..n {
        let d = puppet.drawables().find(|d| d.mesh == mi);
        let (vc, opacity) = puppet
            .model()
            .art_meshes
            .get(mi)
            .map(|m| (m.vertex_count, d.as_ref().map_or(0.0, |d| d.opacity)))
            .unwrap_or((0, 0.0));
        print!("D {mi} {vc} {opacity:.6} 0");
        if let Some(d) = d {
            for xy in d.positions {
                print!(" {xy:.6}");
            }
        } else {
            for _ in 0..vc * 2 {
                print!(" 0.0");
            }
        }
        println!();
    }
    Ok(())
}
