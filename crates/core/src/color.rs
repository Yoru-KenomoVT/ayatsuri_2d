//! Multiply and screen colour (Cubism editor manual, "Multiply Color /
//! Screen Color"): per keyform on art meshes and deformers, interpolated
//! like opacity, and a parent deformer's passed down to everything under
//! it. Multiply darkens (white leaves the art as it is), screen lightens
//! (black leaves it as it is).

use crate::blend::Corner;

/// A multiply and a screen colour, rgb, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub multiply: [f32; 3],
    pub screen: [f32; 3],
}

impl Default for Tint {
    fn default() -> Self {
        Tint::NEUTRAL
    }
}

impl Tint {
    /// White multiply, black screen: the art as it is.
    pub const NEUTRAL: Tint = Tint { multiply: [1.0; 3], screen: [0.0; 3] };

    /// An object's colours at the current keyform weights. Its pools hold
    /// three floats per keyform; an empty pool, or a keyform past its end,
    /// is neutral.
    pub fn blend(corners: &[Corner], multiply: &[f32], screen: &[f32]) -> Tint {
        Tint { multiply: rgb(corners, multiply, 1.0), screen: rgb(corners, screen, 0.0) }
    }

    /// This colour on something under `parent`: multiplies multiply, and
    /// screens combine as screens do (a + b - ab).
    pub fn under(self, parent: Tint) -> Tint {
        let mut out = self;
        for ((m, s), (pm, ps)) in out
            .multiply
            .iter_mut()
            .zip(out.screen.iter_mut())
            .zip(parent.multiply.iter().zip(&parent.screen))
        {
            *m *= pm;
            *s = *s + ps - *s * ps;
        }
        out
    }

    /// Apply to a straight (not premultiplied) colour.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut out = rgb;
        for ((c, m), s) in out.iter_mut().zip(&self.multiply).zip(&self.screen) {
            let x = *c * m;
            *c = x + s - x * s;
        }
        out
    }

    pub fn is_neutral(&self) -> bool {
        *self == Tint::NEUTRAL
    }
}

fn rgb(corners: &[Corner], pool: &[f32], neutral: f32) -> [f32; 3] {
    if pool.is_empty() {
        return [neutral; 3];
    }
    let mut out = [0.0f32; 3];
    for &(k, w) in corners {
        for (c, o) in out.iter_mut().enumerate() {
            let v = k.checked_mul(3).and_then(|i| pool.get(i + c)).copied().unwrap_or(neutral);
            *o += v * w;
        }
    }
    out.map(|v| v.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_leaves_the_art_alone() {
        assert_eq!(Tint::NEUTRAL.apply([0.2, 0.5, 0.9]), [0.2, 0.5, 0.9]);
        assert_eq!(Tint::blend(&[(0, 1.0)], &[], &[]), Tint::NEUTRAL);
    }

    #[test]
    fn keyforms_blend_and_parents_pass_down() {
        // keyform 0 neutral, keyform 1 a red multiply and a blue screen
        let mul = [1.0, 1.0, 1.0, 1.0, 0.0, 0.0];
        let scr = [0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let t = Tint::blend(&[(0, 0.5), (1, 0.5)], &mul, &scr);
        assert_eq!(t.multiply, [1.0, 0.5, 0.5]);
        assert_eq!(t.screen, [0.0, 0.0, 0.5]);
        let parent = Tint { multiply: [0.5; 3], screen: [0.5, 0.0, 0.0] };
        let under = t.under(parent);
        assert_eq!(under.multiply, [0.5, 0.25, 0.25]);
        assert_eq!(under.screen, [0.5, 0.0, 0.5]);
    }

    #[test]
    fn black_multiply_fills_with_the_screen() {
        // the manual's "fill in the specified color"
        let t = Tint { multiply: [0.0; 3], screen: [0.2, 0.4, 0.6] };
        assert_eq!(t.apply([0.9, 0.1, 0.5]), [0.2, 0.4, 0.6]);
    }
}
