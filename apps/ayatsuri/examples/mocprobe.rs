// count-vs-extent for the pool sections, to pin their units down before
// the exporter writes the same tables
#![forbid(unsafe_code)]

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: mocprobe <model.moc3>")?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let moc = openmoc3::Moc3::parse(&bytes).map_err(|e| format!("{e:?}"))?;
    let c = *moc.counts();
    println!("version {:?}  file {} bytes", moc.version(), bytes.len());
    let ci = moc.canvas_info().map_err(|e| format!("{e:?}"))?;
    println!(
        "canvas flags {:#x}  ppu {}  origin {},{}  size {}x{}",
        ci.flags, ci.pixels_per_unit, ci.origin_x, ci.origin_y, ci.width, ci.height
    );
    for (slot, label, count) in [
        (58usize, "part_keyforms", c.part_keyforms),
        (59, "warp_kf (opac)", c.warp_deformer_keyforms),
        (71, "keyform_positions", c.keyform_positions),
        (77, "keys", c.keys),
        (78, "uvs", c.uvs),
        (79, "position_indices", c.position_indices),
        (80, "drawable_masks", c.drawable_masks),
        (98, "glue_info", c.glue_info),
        (100, "glue_keyforms", c.glue_keyforms),
        (103, "param ext begins", c.parameters),
    ] {
        match moc.slot_state(slot) {
            openmoc3::SlotState::Present(e) => {
                println!(
                    "slot {slot:3} {label:18} count {count:8} extent {:8}  count*4={:8}  count*2={:8}",
                    e.size,
                    count as i64 * 4,
                    count as i64 * 2
                );
            }
            st => println!("slot {slot:3} {label:18} count {count:8} {st:?}"),
        }
    }
    // the param extension spans, to see what they index
    if let (Ok(b), Ok(n)) = (moc.i32s(103), moc.i32s(104)) {
        for p in 0..3usize {
            println!("param {p}: ext begin {:?} count {:?}", b.get(p), n.get(p));
        }
    }
    // alignment habits: first few slot offsets
    for slot in [0usize, 1, 2, 3, 4] {
        if let openmoc3::SlotState::Present(e) = moc.slot_state(slot) {
            println!("slot {slot} at {:#x} size {}", e.offset, e.size);
        }
    }
    Ok(())
}
