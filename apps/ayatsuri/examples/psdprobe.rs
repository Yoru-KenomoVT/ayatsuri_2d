// what the psd crate reports per layer, to debug the importer's filters
#![forbid(unsafe_code)]
fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: psdprobe <file.psd>")?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let psd = psd::Psd::from_bytes(&bytes).map_err(|e| e.to_string())?;
    println!("canvas {}x{}", psd.width(), psd.height());
    for l in psd.layers() {
        let px = l.rgba();
        let alpha_nonzero = px.chunks(4).filter(|p| p.get(3).copied().unwrap_or(0) > 0).count();
        println!(
            "{:20} vis {} op {} ltrb {},{},{},{} rgba {} alpha>0 px {} clip {}",
            l.name(),
            l.visible(),
            l.opacity(),
            l.layer_left(),
            l.layer_top(),
            l.layer_right(),
            l.layer_bottom(),
            px.len(),
            alpha_nonzero,
            l.is_clipping_mask()
        );
    }
    Ok(())
}
