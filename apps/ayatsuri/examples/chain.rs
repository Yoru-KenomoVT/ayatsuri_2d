//! Print each mesh's deformer parent chain, for cross-referencing diffs.

#![forbid(unsafe_code)]

use ayatsuri_format::model::DeformerKind;

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: chain <model.moc3>")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let model = ayatsuri_import::from_moc3(&bytes).map_err(|e| format!("{path}: {e}"))?;
    for (i, m) in model.art_meshes.iter().enumerate() {
        let mut chain = Vec::new();
        let mut d = m.parent_deformer;
        while let Some(di) = d {
            let Some(def) = model.deformers.get(di as usize) else { break };
            chain.push(match &def.kind {
                DeformerKind::Warp(w) => {
                    format!("W{di}({}x{}{})", w.rows, w.cols, if w.quad { ",quad" } else { "" })
                }
                DeformerKind::Rotation(_) => format!("R{di}"),
            });
            d = def.parent_deformer;
        }
        println!("M {i} {} axes={} chain={}", m.id, m.binding.axes.len(), chain.join(" -> "));
    }
    Ok(())
}
