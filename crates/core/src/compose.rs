//! One premultiplied pixel drawn over another with a mesh's blend: the
//! colour blend picks how the two colours mix where they overlap (the W3C
//! Compositing and Blending formulas), the alpha blend how much of the
//! mixed colour, the source and the backdrop each keep (Porter-Duff over,
//! atop, out, and the conjoint and disjoint overlaps). The GPU shader in
//! ayatsuri-render does the same maths; the CPU renderer calls this.

use ayatsuri_format::model::{AlphaBlend, Blend, ColorBlend};

/// `src` drawn over `dst`, both premultiplied RGBA in 0..1.
pub fn over(blend: Blend, src: [f32; 4], dst: [f32; 4]) -> [f32; 4] {
    let [sr, sg, sb, sa] = src;
    let [dr, dg, db, da] = dst;
    match blend.color {
        // the two modes every moc3 has ignore the alpha blend and leave
        // the backdrop's alpha alone
        ColorBlend::AddCompatible => {
            return [(dr + sr).min(1.0), (dg + sg).min(1.0), (db + sb).min(1.0), da];
        }
        ColorBlend::MultiplyCompatible => {
            let keep = 1.0 - sa;
            return [sr * dr + dr * keep, sg * dg + dg * keep, sb * db + db * keep, da];
        }
        _ => {}
    }
    if blend == Blend::NORMAL {
        let keep = 1.0 - sa;
        return [sr + dr * keep, sg + dg * keep, sb + db * keep, sa + da * keep];
    }
    let cs = unpremultiply([sr, sg, sb], sa);
    let cb = unpremultiply([dr, dg, db], da);
    let mixed = mix(blend.color, cs, cb);
    let (p0, p1, p2) = match blend.alpha {
        AlphaBlend::Conjoint => (sa.min(da), (sa - da).max(0.0), (da - sa).max(0.0)),
        AlphaBlend::Disjoint => ((sa + da - 1.0).max(0.0), sa.min(1.0 - da), da.min(1.0 - sa)),
        _ => (sa * da, sa * (1.0 - da), da * (1.0 - sa)),
    };
    // which of the three areas survive: both, the source alone, the
    // backdrop alone
    let (x, y, z) = match blend.alpha {
        AlphaBlend::Atop => (1.0, 0.0, 1.0),
        AlphaBlend::Out => (0.0, 1.0, 0.0),
        _ => (1.0, 1.0, 1.0),
    };
    let ch = |i: usize| {
        let m = mixed.get(i).copied().unwrap_or(0.0);
        let s = cs.get(i).copied().unwrap_or(0.0);
        let b = cb.get(i).copied().unwrap_or(0.0);
        (x * p0 * m + y * p1 * s + z * p2 * b).clamp(0.0, 1.0)
    };
    [ch(0), ch(1), ch(2), (x * p0 + y * p1 + z * p2).clamp(0.0, 1.0)]
}

fn unpremultiply(c: [f32; 3], a: f32) -> [f32; 3] {
    if a <= 1e-6 { [0.0; 3] } else { c.map(|v| (v / a).clamp(0.0, 1.0)) }
}

/// The colour where source `s` meets backdrop `b`, both straight 0..1.
pub fn mix(mode: ColorBlend, s: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let each = |f: fn(f32, f32) -> f32| [f(s[0], b[0]), f(s[1], b[1]), f(s[2], b[2])];
    match mode {
        ColorBlend::Normal | ColorBlend::AddCompatible | ColorBlend::MultiplyCompatible => s,
        ColorBlend::Add => each(|s, b| (s + b).min(1.0)),
        // glow lets the sum run past white before the alpha weighs it
        ColorBlend::AddGlow => each(|s, b| s + b),
        ColorBlend::Darken => each(f32::min),
        ColorBlend::Multiply => each(|s, b| s * b),
        ColorBlend::ColorBurn => each(color_burn),
        ColorBlend::LinearBurn => each(|s, b| (s + b - 1.0).max(0.0)),
        ColorBlend::Lighten => each(f32::max),
        ColorBlend::Screen => each(screen),
        ColorBlend::ColorDodge => each(color_dodge),
        ColorBlend::Overlay => each(|s, b| hard_light(b, s)),
        ColorBlend::SoftLight => each(soft_light),
        ColorBlend::HardLight => each(hard_light),
        ColorBlend::LinearLight => each(|s, b| (b + 2.0 * s - 1.0).clamp(0.0, 1.0)),
        ColorBlend::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        ColorBlend::Color => set_lum(s, lum(b)),
    }
}

fn screen(s: f32, b: f32) -> f32 {
    s + b - s * b
}

fn color_burn(s: f32, b: f32) -> f32 {
    if b >= 1.0 {
        1.0
    } else if s <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - b) / s).min(1.0)
    }
}

fn color_dodge(s: f32, b: f32) -> f32 {
    if b <= 0.0 {
        0.0
    } else if s >= 1.0 {
        1.0
    } else {
        (b / (1.0 - s)).min(1.0)
    }
}

fn hard_light(s: f32, b: f32) -> f32 {
    if s <= 0.5 { b * 2.0 * s } else { screen(2.0 * s - 1.0, b) }
}

fn soft_light(s: f32, b: f32) -> f32 {
    if s <= 0.5 {
        b - (1.0 - 2.0 * s) * b * (1.0 - b)
    } else {
        let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
        b + (2.0 * s - 1.0) * (d - b)
    }
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 && l - n > 1e-6 {
        out = out.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 && x - l > 1e-6 {
        out = out.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    out
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let x = c[0].max(c[1]).max(c[2]);
    let n = c[0].min(c[1]).min(c[2]);
    if x - n <= 1e-6 {
        return [0.0; 3];
    }
    c.map(|v| (v - n) * s / (x - n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    fn b(color: ColorBlend, alpha: AlphaBlend) -> Blend {
        Blend { color, alpha }
    }

    #[test]
    fn normal_over_is_plain_source_over() {
        let src = [0.25, 0.0, 0.0, 0.5];
        let dst = [0.0, 0.0, 1.0, 1.0];
        assert!(close(over(Blend::NORMAL, src, dst), [0.25, 0.0, 0.5, 1.0]));
        // the general path agrees with the fast one
        assert!(close(
            over(b(ColorBlend::Normal, AlphaBlend::Conjoint), [0.5, 0.5, 0.5, 1.0], dst),
            [0.5, 0.5, 0.5, 1.0]
        ));
    }

    #[test]
    fn opaque_modes_match_their_formulas() {
        let s = [0.6, 0.2, 0.9, 1.0];
        let d = [0.5, 0.5, 0.5, 1.0];
        let m = over(b(ColorBlend::Multiply, AlphaBlend::Over), s, d);
        assert!(close(m, [0.3, 0.1, 0.45, 1.0]), "{m:?}");
        let sc = over(b(ColorBlend::Screen, AlphaBlend::Over), s, d);
        assert!(close(sc, [0.8, 0.6, 0.95, 1.0]), "{sc:?}");
        // overlay on mid grey backdrop is the source pushed by hard light
        let o = over(b(ColorBlend::Overlay, AlphaBlend::Over), s, d);
        assert!(close(o, [0.6, 0.2, 0.9, 1.0]), "{o:?}");
        let dk = over(b(ColorBlend::Darken, AlphaBlend::Over), s, d);
        assert!(close(dk, [0.5, 0.2, 0.5, 1.0]), "{dk:?}");
    }

    #[test]
    fn a_transparent_backdrop_shows_the_source_untouched() {
        // nothing to blend with: every colour mode draws the source as is
        let s = [0.3, 0.2, 0.1, 0.5];
        for mode in ColorBlend::ALL {
            if matches!(mode, ColorBlend::AddCompatible | ColorBlend::MultiplyCompatible) {
                continue;
            }
            let out = over(b(mode, AlphaBlend::Over), s, [0.0; 4]);
            assert!(close(out, s), "{mode:?}: {out:?}");
        }
    }

    #[test]
    fn alpha_modes_keep_the_right_areas() {
        let s = [1.0, 0.0, 0.0, 1.0];
        let d = [0.0, 0.0, 0.5, 0.5];
        // atop keeps the backdrop's coverage
        assert!((over(b(ColorBlend::Normal, AlphaBlend::Atop), s, d)[3] - 0.5).abs() < 1e-4);
        // out keeps only the source outside the backdrop
        let out = over(b(ColorBlend::Normal, AlphaBlend::Out), s, d);
        assert!(close(out, [0.5, 0.0, 0.0, 0.5]), "{out:?}");
        // conjoint: the larger alpha wins
        assert!((over(b(ColorBlend::Normal, AlphaBlend::Conjoint), s, d)[3] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn compatible_modes_keep_the_backdrop_alpha() {
        let s = [0.2, 0.2, 0.2, 0.4];
        let d = [0.5, 0.5, 0.5, 0.8];
        assert!((over(b(ColorBlend::AddCompatible, AlphaBlend::Out), s, d)[3] - 0.8).abs() < 1e-6);
        assert!(
            (over(b(ColorBlend::MultiplyCompatible, AlphaBlend::Atop), s, d)[3] - 0.8).abs() < 1e-6
        );
    }
}
