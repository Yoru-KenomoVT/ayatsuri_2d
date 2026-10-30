//! Which of the later moc3 features a file uses, with the raw tables for
//! blend shapes and offscreens: what the exporter must reproduce.
//!
//!     cargo run -p openmoc3 --example features -- model.moc3

// a developer's look at files on disk, not the parser: indices from the
// file go through `get`, and plain arithmetic on them is fine here
#![allow(clippy::arithmetic_side_effects)]

use openmoc3::{slot, Moc3};

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: features <model.moc3>")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let moc = Moc3::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let c = *moc.counts();
    println!("{path}: {:?}", moc.version());
    println!(
        "  blend shapes: key tables {}, bindings {}, warps {}, meshes {}, parts {}, rotations {}, glue {}, offscreens {}",
        c.blend_shape_parameter_bindings,
        c.blend_shape_keyform_bindings,
        c.blend_shapes_warp_deformers,
        c.blend_shapes_art_meshes,
        c.blend_shapes_parts,
        c.blend_shapes_rotation_deformers,
        c.blend_shapes_glue,
        c.blend_shapes_offscreens
    );
    println!(
        "  limits: constraint indices {}, constraints {}, values {}",
        c.blend_shape_constraint_indices,
        c.blend_shape_constraints,
        c.blend_shape_constraint_values
    );
    println!("  offscreens {}, offscreen keyforms {}", c.offscreens, c.offscreen_keyforms);
    let i32s = |s: usize| moc.i32s(s).map(|v| v.iter().collect::<Vec<i32>>()).unwrap_or_default();
    let f32s = |s: usize| moc.f32s(s).map(|v| v.iter().collect::<Vec<f32>>()).unwrap_or_default();

    if c.blend_shape_parameter_bindings > 0 {
        let names: Vec<String> = moc
            .ids(slot::PARAMETER_IDS)
            .map(|i| i.map(str::to_string).collect())
            .unwrap_or_default();
        let (ptype, kt_begin, kt_count) = (i32s(114), i32s(115), i32s(116));
        let (keys_begin, keys_count) = (i32s(103), i32s(104));
        let pool = f32s(77);
        println!("  blend shape parameters (type, key tables, the parameter's own keys):");
        for (p, name) in names.iter().enumerate() {
            let n = kt_count.get(p).copied().unwrap_or(0);
            if n <= 0 {
                continue;
            }
            let (kb, kc) =
                (keys_begin.get(p).copied().unwrap_or(0), keys_count.get(p).copied().unwrap_or(0));
            let own = pool.get(kb as usize..(kb + kc) as usize).unwrap_or_default();
            println!(
                "    {p} {name}: type {}, tables {}..+{n}, keys {own:?}",
                ptype.get(p).copied().unwrap_or(-9),
                kt_begin.get(p).copied().unwrap_or(-9)
            );
        }
        let (tk_begin, tk_count, base) = (i32s(117), i32s(118), i32s(119));
        println!("  key tables (keys, base key index):");
        for t in 0..c.blend_shape_parameter_bindings as usize {
            let (b, n) =
                (tk_begin.get(t).copied().unwrap_or(0), tk_count.get(t).copied().unwrap_or(0));
            let keys = pool.get(b as usize..(b + n) as usize).unwrap_or_default();
            println!("    {t}: keys {keys:?} base {}", base.get(t).copied().unwrap_or(-9));
        }
        let (table, kf_off, kf_len, ci_b, ci_n) =
            (i32s(120), i32s(121), i32s(122), i32s(123), i32s(124));
        println!("  bindings (key table, keyform offset, keyform count, limits begin/count):");
        for b in 0..c.blend_shape_keyform_bindings as usize {
            println!(
                "    {b}: table {} off {} len {} limits {}..+{}",
                table.get(b).copied().unwrap_or(-9),
                kf_off.get(b).copied().unwrap_or(-9),
                kf_len.get(b).copied().unwrap_or(-9),
                ci_b.get(b).copied().unwrap_or(-9),
                ci_n.get(b).copied().unwrap_or(-9)
            );
        }
        let mesh_names: Vec<String> = moc
            .ids(slot::ART_MESH_IDS)
            .map(|i| i.map(str::to_string).collect())
            .unwrap_or_default();
        // a mesh's keyforms: begin (slot 35) and count (36)
        let (m_kf_begin, m_kf_count) = (i32s(35), i32s(36));
        let (target, bb, bn) = (i32s(128), i32s(129), i32s(130));
        println!("  art mesh blend shapes (mesh, its keyforms begin/count, bindings):");
        for s in 0..c.blend_shapes_art_meshes as usize {
            let t = target.get(s).copied().unwrap_or(-1);
            println!(
                "    {s}: mesh {t} {} keyforms {}..+{} bindings {}..+{}",
                mesh_names.get(t.max(0) as usize).map(String::as_str).unwrap_or("?"),
                m_kf_begin.get(t.max(0) as usize).copied().unwrap_or(-9),
                m_kf_count.get(t.max(0) as usize).copied().unwrap_or(-9),
                bb.get(s).copied().unwrap_or(-9),
                bn.get(s).copied().unwrap_or(-9)
            );
        }
        // warp blend shapes: the warp's keyforms (20/21, global), and per
        // binding keyform the size of its numbers, to tell deltas from forms
        let (w_kf_begin, w_kf_count, w_points) = (i32s(20), i32s(21), i32s(22));
        let (w_pos_begin, coords) = (i32s(60), f32s(71));
        let (wt, wbb, wbn) = (i32s(125), i32s(126), i32s(127));
        println!("  warp blend shapes (warp, its keyforms, bindings):");
        for s in 0..c.blend_shapes_warp_deformers as usize {
            let w = wt.get(s).copied().unwrap_or(-1).max(0) as usize;
            let (kb, kn) =
                (w_kf_begin.get(w).copied().unwrap_or(0), w_kf_count.get(w).copied().unwrap_or(0));
            let pts = w_points.get(w).copied().unwrap_or(0) as usize;
            println!(
                "    {s}: warp {w} keyforms {kb}..+{kn} ({pts} points) bindings {}..+{}",
                wbb.get(s).copied().unwrap_or(-9),
                wbn.get(s).copied().unwrap_or(-9)
            );
            let size = |k: usize| {
                let b = w_pos_begin.get(k).copied().unwrap_or(0).max(0) as usize;
                coords
                    .get(b..b + pts * 2)
                    .unwrap_or_default()
                    .iter()
                    .fold(0.0f32, |a, v| a.max(v.abs()))
            };
            println!("      normal keyform 0 largest |value| {:.4}", size(kb as usize));
            let (opac, w_kf_total) = (f32s(59), c.warp_deformer_keyforms);
            let normal_end = (0..c.warp_deformers as usize)
                .map(|w| {
                    w_kf_begin.get(w).copied().unwrap_or(0)
                        + w_kf_count.get(w).copied().unwrap_or(0)
                })
                .max()
                .unwrap_or(0);
            println!("      warp keyforms in file {w_kf_total}, normal ones end at {normal_end}");
            let first = wbb.get(s).copied().unwrap_or(0).max(0) as usize;
            if let (Some(&off), Some(&len)) = (kf_off.get(first), kf_len.get(first)) {
                let o: Vec<f32> = (off..off + len)
                    .map(|k| opac.get(k as usize).copied().unwrap_or(f32::NAN))
                    .collect();
                println!("      opacities at the blend keyforms {o:?}");
            }
            let first = wbb.get(s).copied().unwrap_or(0).max(0) as usize;
            for b in first..first + wbn.get(s).copied().unwrap_or(0).max(0) as usize {
                let (off, len) =
                    (kf_off.get(b).copied().unwrap_or(0), kf_len.get(b).copied().unwrap_or(0));
                for k in 0..len {
                    // the offset counted from the file's first warp keyform, or the warp's own
                    let global = (off + k) as usize;
                    let local = (kb + off + k) as usize;
                    println!(
                        "      binding {b} keyform {k}: as global {global} -> {:.4}, as local {local} -> {:.4}",
                        size(global),
                        size(local)
                    );
                }
            }
        }
        if c.blend_shape_constraints > 0 {
            let (cp, cvb, cvn) = (i32s(132), i32s(133), i32s(134));
            let (ck, cw) = (f32s(135), f32s(136));
            println!("  limits (parameter, keys -> weights):");
            for k in 0..c.blend_shape_constraints as usize {
                let (b, n) = (
                    cvb.get(k).copied().unwrap_or(0) as usize,
                    cvn.get(k).copied().unwrap_or(0) as usize,
                );
                println!(
                    "    {k}: param {} keys {:?} weights {:?}",
                    cp.get(k).copied().unwrap_or(-9),
                    ck.get(b..b + n).unwrap_or_default(),
                    cw.get(b..b + n).unwrap_or_default()
                );
            }
        }
    }
    if c.offscreens > 0 {
        let part_names: Vec<String> =
            moc.ids(slot::PART_IDS).map(|i| i.map(str::to_string).collect()).unwrap_or_default();
        let (owner, blend, mb, mn) = (i32s(155), i32s(157), i32s(158), i32s(159));
        let flags = moc.u8s(156).map(<[u8]>::to_vec).unwrap_or_default();
        println!("  offscreens (owner part, flags, blend, masks):");
        for o in 0..c.offscreens as usize {
            let p = owner.get(o).copied().unwrap_or(-1);
            println!(
                "    {o}: part {p} {} flags {:#04x} blend {:#06x} masks {}..+{}",
                part_names.get(p.max(0) as usize).map(String::as_str).unwrap_or("?"),
                flags.get(o).copied().unwrap_or(0),
                blend.get(o).copied().unwrap_or(0),
                mb.get(o).copied().unwrap_or(-9),
                mn.get(o).copied().unwrap_or(-9)
            );
        }
        let groups = (i32s(81), i32s(82), i32s(83));
        println!("  draw order groups (begin, count, total): {:?}", groups);
    }
    Ok(())
}
