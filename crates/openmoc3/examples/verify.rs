//! Parse a `.moc3` and print what it holds: the first visible output of the
//! build order in ayatsuri2d-research/docs/16-parser-guide.md section 9.
//!
//!     cargo run -p openmoc3 --example verify -- model.moc3

use openmoc3::{slot, Moc3};

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: verify <model.moc3>")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let moc = Moc3::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;

    println!("{path}: {:?}, {} bytes", moc.version(), bytes.len());
    let c = moc.counts();
    println!(
        "  parts {}  deformers {} (warp {} + rotation {})  art meshes {}  parameters {}",
        c.parts, c.deformers, c.warp_deformers, c.rotation_deformers, c.art_meshes, c.parameters
    );

    println!("  parts:");
    for name in moc.ids(slot::PART_IDS).map_err(|e| e.to_string())? {
        println!("    {name}");
    }

    let mins = moc.f32s(slot::PARAMETER_MIN).map_err(|e| e.to_string())?;
    let maxs = moc.f32s(slot::PARAMETER_MAX).map_err(|e| e.to_string())?;
    let defaults = moc.f32s(slot::PARAMETER_DEFAULT).map_err(|e| e.to_string())?;
    println!("  parameters:");
    for (i, name) in moc.ids(slot::PARAMETER_IDS).map_err(|e| e.to_string())?.enumerate() {
        let min = mins.get(i).unwrap_or(f32::NAN);
        let max = maxs.get(i).unwrap_or(f32::NAN);
        let default = defaults.get(i).unwrap_or(f32::NAN);
        println!("    {name}: {min} to {max}, default {default}");
    }
    Ok(())
}
