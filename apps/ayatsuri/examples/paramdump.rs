// print every parameter's range and default, for rig spelunking
#![forbid(unsafe_code)]
fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: paramdump <model.moc3>")?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let model = ayatsuri_import::from_moc3(&bytes).map_err(|e| e.to_string())?;
    for p in &model.parameters {
        if p.id.to_lowercase().contains("angle") {
            println!("{}: {} .. {} default {}", p.id, p.min, p.max, p.default);
        }
    }
    Ok(())
}
