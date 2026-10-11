// Photoshop documents through the `psd` crate (MIT/Apache). Each layer
// comes back at full canvas size, so it is cropped to its own bounds right
// away; holding every layer canvas-sized would cost gigabytes on a big
// illustration.
//
// Three things in psd 0.3.5 read backwards and are corrected here, checked
// against the crate's own fixtures and the published format notes:
//   visible()            flag bit 1 set means HIDDEN; the crate says visible
//   is_clipping_mask()   true for clipping BASES (byte 0), so a clipped
//                        layer is one where it reports false
//   layer_right/bottom   stored minus one, i.e. inclusive
// The crate can also panic on malformed files, so parsing runs behind
// catch_unwind: a bad PSD is an error message, never a crashed editor.

use crate::ImportError;
use crate::layers::{Document, Layer};

/// Read a .psd into a layered document, bottom layer first.
pub fn read_psd(bytes: &[u8]) -> Result<Document, ImportError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read_inner(bytes))) {
        Ok(result) => result,
        Err(_) => Err(ImportError::Export(
            "psd: the file is malformed in a way the PSD reader could not handle".into(),
        )),
    }
}

fn read_inner(bytes: &[u8]) -> Result<Document, ImportError> {
    let psd = psd::Psd::from_bytes(bytes).map_err(|e| ImportError::Export(format!("psd: {e}")))?;
    let (cw, ch) = (psd.width(), psd.height());
    let groups = psd.groups();

    // the crate lists layers top-down; the model wants bottom-up
    let mut layers = Vec::with_capacity(psd.layers().len());
    for l in psd.layers().iter().rev() {
        let left = l.layer_left().clamp(0, cw as i32);
        let top = l.layer_top().clamp(0, ch as i32);
        let right = l.layer_right().saturating_add(1).clamp(0, cw as i32);
        let bottom = l.layer_bottom().saturating_add(1).clamp(0, ch as i32);
        let (w, h) = ((right - left).max(0) as u32, (bottom - top).max(0) as u32);

        let mut path = Vec::new();
        let mut cur = l.parent_id();
        let mut hops = 0;
        while let Some(id) = cur {
            let Some(g) = groups.get(&id) else { break };
            path.push(g.name().to_string());
            cur = g.parent_id();
            hops += 1;
            if hops > 64 {
                break;
            }
        }
        path.reverse();

        let mut rgba = Vec::new();
        if w > 0 && h > 0 {
            let full = l.rgba();
            rgba.reserve((w * h * 4) as usize);
            for y in top..bottom {
                let row = ((y as u32 * cw + left as u32) * 4) as usize;
                if let Some(slice) = full.get(row..row + (w * 4) as usize) {
                    rgba.extend_from_slice(slice);
                } else {
                    rgba.extend(std::iter::repeat_n(0u8, (w * 4) as usize));
                }
            }
        }
        layers.push(Layer {
            name: l.name().to_string(),
            groups: path,
            x: left,
            y: top,
            width: w,
            height: h,
            rgba,
            visible: !l.visible(),
            opacity: l.opacity() as f32 / 255.0,
            clip_to_below: !l.is_clipping_mask(),
        });
    }
    Ok(Document { width: cw, height: ch, layers })
}
