// A model as a Cubism Editor project: the reverse of extract.rs and
// build.rs, laid out the way a project saved by the Editor is. Each art
// mesh gets its art back as a layer of one layered image, cut from the
// model's texture at canvas resolution; the layers are packed into fresh
// atlases the meshes' uvs point into; parts, deformers, parameters with
// their folders, every keyform and the physics groups follow the object
// shapes read in extract.rs. Keyforms are written in the parent's space:
// canvas pixels at the top level, the 0..1 grid under a warp, pixel
// offsets under a rotation.
//
// A project only holds the rig. Animations, expressions and sound are not
// part of a .cmo3; the motion export carries those.

use std::collections::HashMap;

use ayatsuri_format::model::{DeformerKind, Model, PhysicsIo, flags};

use super::caffw;
use super::xmlw::{Tree, fmt_f};
use crate::ImportError;
use crate::layers::{self, Blend, Document, Layer, PAGE};

fn err(msg: impl Into<String>) -> ImportError {
    ImportError::Export(format!("cmo3: {}", msg.into()))
}

/// What a project is written from.
pub struct Cmo3Export<'a> {
    pub name: &'a str,
    pub model: &'a Model,
    /// The texture pages, encoded, by the meshes' texture index.
    pub textures: &'a [Vec<u8>],
    /// Display names, parallel to the model's parameters, parts and
    /// meshes; shorter lists fall back to the ids.
    pub param_names: &'a [String],
    pub part_names: &'a [String],
    pub mesh_names: &'a [String],
    /// Parameter folders in palette order: name and member indices.
    pub param_groups: &'a [(String, Vec<u32>)],
}

const HEAD: &str = include_str!("head.xml");
const NOTE_NONE: &str = "(no debug info)";
const TIME_MS: i64 = 1_767_268_800_000;
const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";

/// Deterministic uuids, so the same model writes the same project.
struct Ids(u64);

impl Ids {
    fn next(&mut self) -> u64 {
        // xorshift64*, seeded from the model name
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn uuid(&mut self) -> String {
        let (a, b) = (self.next(), self.next());
        let hi = (a & 0xffff_ffff_ffff_0fff) | 0x4000;
        let lo = (b & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
        format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            hi >> 32,
            (hi >> 16) & 0xffff,
            hi & 0xffff,
            lo >> 48,
            lo & 0xffff_ffff_ffff
        )
    }
}

/// A mesh's art cut back out of the texture, at canvas resolution.
struct Material {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    rgba: image::RgbaImage,
}

/// Where a material landed in the new atlases.
struct Placed {
    page: usize,
    ax: f32,
    ay: f32,
    sx: f32,
    sy: f32,
}

/// Which space a controllable's forms are written in, from its parent.
#[derive(Clone, Copy)]
enum Space {
    Root,
    Warp,
    Rotation,
}

/// The affine `uv px -> canvas px` that fits a mesh best, least squares;
/// a translation at scale one when the points cannot say.
fn fit_affine(uv: &[f32], px: &[f32]) -> [f64; 6] {
    let n = uv.len().min(px.len()) / 2;
    let (mut su, mut sv, mut suu, mut suv, mut svv) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let (mut sx, mut sy, mut sux, mut svx, mut suy, mut svy) =
        (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let u = uv.get(i * 2).copied().unwrap_or(0.0) as f64;
        let v = uv.get(i * 2 + 1).copied().unwrap_or(0.0) as f64;
        let x = px.get(i * 2).copied().unwrap_or(0.0) as f64;
        let y = px.get(i * 2 + 1).copied().unwrap_or(0.0) as f64;
        su += u;
        sv += v;
        suu += u * u;
        suv += u * v;
        svv += v * v;
        sx += x;
        sy += y;
        sux += u * x;
        svx += v * x;
        suy += u * y;
        svy += v * y;
    }
    let nf = n as f64;
    let fallback = || {
        let (mu, mv) = if n > 0 { (su / nf, sv / nf) } else { (0.0, 0.0) };
        let (mx, my) = if n > 0 { (sx / nf, sy / nf) } else { (0.0, 0.0) };
        [1.0, 0.0, mx - mu, 0.0, 1.0, my - mv]
    };
    if n < 3 {
        return fallback();
    }
    // normal equations, one solve per output axis
    let m = [[suu, suv, su], [suv, svv, sv], [su, sv, nf]];
    let det = det3(&m);
    let scale = (suu + svv + nf).max(1e-12).powi(3);
    if det.abs() < scale * 1e-12 {
        return fallback();
    }
    let solve = |b: [f64; 3]| -> [f64; 3] {
        let col = |k: usize| {
            let mut c = m;
            for (row, bv) in c.iter_mut().zip(b) {
                if let Some(slot) = row.get_mut(k) {
                    *slot = bv;
                }
            }
            det3(&c) / det
        };
        [col(0), col(1), col(2)]
    };
    let a = solve([sux, svx, sx]);
    let b = solve([suy, svy, sy]);
    let out = [a[0], a[1], a[2], b[0], b[1], b[2]];
    if out.iter().any(|v| !v.is_finite()) {
        return fallback();
    }
    out
}

fn det3(m: &[[f64; 3]; 3]) -> f64 {
    let g = |r: usize, c: usize| m.get(r).and_then(|row| row.get(c)).copied().unwrap_or(0.0);
    g(0, 0) * (g(1, 1) * g(2, 2) - g(1, 2) * g(2, 1))
        - g(0, 1) * (g(1, 0) * g(2, 2) - g(1, 2) * g(2, 0))
        + g(0, 2) * (g(1, 0) * g(2, 1) - g(1, 1) * g(2, 0))
}

/// The mesh's art at canvas resolution: every pixel of its bounding box
/// looked up in the texture through the inverse of the fitted affine.
fn material(
    tex: Option<&image::RgbaImage>,
    uvs: &[f32],
    px: &[f32],
) -> Result<Material, ImportError> {
    let Some(tex) = tex else { return Err(err("a mesh points at a missing texture")) };
    let (tw, th) = (tex.width() as f32, tex.height() as f32);
    let uvpx: Vec<f32> =
        uvs.as_chunks::<2>().0.iter().flat_map(|p| [p[0] * tw, p[1] * th]).collect();
    let a = fit_affine(&uvpx, px);
    let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in px.as_chunks::<2>().0 {
        lo_x = lo_x.min(p[0]);
        lo_y = lo_y.min(p[1]);
        hi_x = hi_x.max(p[0]);
        hi_y = hi_y.max(p[1]);
    }
    if !(lo_x.is_finite() && lo_y.is_finite() && hi_x.is_finite() && hi_y.is_finite()) {
        return Err(err("a mesh has no finite points"));
    }
    let left = (lo_x.floor() as i32).saturating_sub(1);
    let top = (lo_y.floor() as i32).saturating_sub(1);
    let right = (hi_x.ceil() as i32).saturating_add(1);
    let bottom = (hi_y.ceil() as i32).saturating_add(1);
    let (width, height) = ((right - left).max(1) as u32, (bottom - top).max(1) as u32);
    if (width as u64) * (height as u64) > 64 << 20 {
        return Err(err("a mesh spans more than 64 million pixels"));
    }
    // the inverse: canvas px -> texture px
    let det = a[0] * a[4] - a[1] * a[3];
    let inv = if det.abs() > 1e-12 {
        [a[4] / det, -a[1] / det, -a[3] / det, a[0] / det]
    } else {
        [1.0, 0.0, 0.0, 1.0]
    };
    // a plain copy when the art sits on the canvas pixel for pixel
    let whole = |v: f64| (v - v.round()).abs() < 1e-3;
    let exact = [a[0], a[1], a[3], a[4]].iter().all(|v| whole(*v) && v.abs() <= 1.0)
        && whole(a[2])
        && whole(a[5]);
    let mut rgba = image::RgbaImage::new(width, height);
    let (tw_i, th_i) = (tex.width() as i64, tex.height() as i64);
    let fetch = |x: i64, y: i64| -> [f32; 4] {
        if x < 0 || y < 0 || x >= tw_i || y >= th_i {
            return [0.0; 4];
        }
        let p = tex.get_pixel(x as u32, y as u32).0;
        [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]
    };
    for j in 0..height {
        for i in 0..width {
            let cx = left as f64 + i as f64 + 0.5;
            let cy = top as f64 + j as f64 + 0.5;
            let dx = cx - a[2];
            let dy = cy - a[5];
            let u = inv[0] * dx + inv[1] * dy;
            let v = inv[2] * dx + inv[3] * dy;
            let out = if exact {
                let p = fetch(u.floor() as i64, v.floor() as i64);
                [p[0] as u8, p[1] as u8, p[2] as u8, p[3] as u8]
            } else {
                // bilinear on premultiplied colour, so transparent texels
                // do not darken the edge
                let (fu, fv) = (u - 0.5, v - 0.5);
                let (x0, y0) = (fu.floor(), fv.floor());
                let (tx, ty) = ((fu - x0) as f32, (fv - y0) as f32);
                let (x0, y0) = (x0 as i64, y0 as i64);
                let mut acc = [0.0f32; 4];
                for (sx, sy, w) in [
                    (x0, y0, (1.0 - tx) * (1.0 - ty)),
                    (x0 + 1, y0, tx * (1.0 - ty)),
                    (x0, y0 + 1, (1.0 - tx) * ty),
                    (x0 + 1, y0 + 1, tx * ty),
                ] {
                    let p = fetch(sx, sy);
                    let al = p[3] / 255.0;
                    acc[0] += p[0] * al * w;
                    acc[1] += p[1] * al * w;
                    acc[2] += p[2] * al * w;
                    acc[3] += p[3] * w;
                }
                let al = acc[3] / 255.0;
                if al > 1e-4 {
                    [
                        (acc[0] / al).round().clamp(0.0, 255.0) as u8,
                        (acc[1] / al).round().clamp(0.0, 255.0) as u8,
                        (acc[2] / al).round().clamp(0.0, 255.0) as u8,
                        acc[3].round().clamp(0.0, 255.0) as u8,
                    ]
                } else {
                    [0; 4]
                }
            };
            rgba.put_pixel(i, j, image::Rgba(out));
        }
    }
    Ok(Material { left, top, width, height, rgba })
}

fn png(img: &image::RgbaImage) -> Result<Vec<u8>, ImportError> {
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| err(format!("png encode: {e}")))?;
    Ok(out)
}

fn thumb(img: &image::RgbaImage, side: u32) -> Result<Vec<u8>, ImportError> {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let k = (side as f32 / w as f32).min(side as f32 / h as f32).min(1.0);
    let (nw, nh) = (((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1));
    let small = image::imageops::thumbnail(img, nw, nh);
    let mut canvas = image::RgbaImage::new(side, side);
    image::imageops::replace(
        &mut canvas,
        &small,
        ((side - nw) / 2) as i64,
        ((side - nh) / 2) as i64,
    );
    png(&canvas)
}

/// The files of the archive as they are made.
struct Files {
    list: Vec<caffw::File>,
    images: usize,
    icons: usize,
}

impl Files {
    fn image(&mut self, bytes: Vec<u8>) -> String {
        let name = match self.images {
            0 => "imageFileBuf.png".to_string(),
            n => format!("imageFileBuf_{}.png", n - 1),
        };
        self.images += 1;
        self.list.push(caffw::File { name: name.clone(), bytes, main: false });
        name
    }

    fn icon(&mut self, bytes: Vec<u8>) -> String {
        let name = match self.icons {
            0 => "image.png".to_string(),
            n => format!("image_{}.png", n - 1),
        };
        self.icons += 1;
        self.list.push(caffw::File { name: name.clone(), bytes, main: false });
        name
    }
}

/// Keyform coordinates on a binding grid: the first axis changes fastest,
/// as in Cubism and in the model's keyform-major arrays.
fn coords(slot: usize, lens: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(lens.len());
    let mut stride = 1;
    for &len in lens {
        out.push((slot / stride) % len.max(1));
        stride *= len.max(1);
    }
    out
}

/// Everything the object builders share.
struct W {
    t: Tree,
    ids: Ids,
    files: Files,
    coord_basic: usize,
    coord_local: usize,
    blend: usize,
    opts: usize,
    root_deformer: usize,
    zero_deformer: usize,
    layered_guid: usize,
    layered: usize,
    layer_root: usize,
    group: usize,
    fv: FilterVals,
}

/// The filter graph's shared vocabulary: value ids, values and the two
/// filter definitions every model image goes through.
struct FilterVals {
    in_layer: usize,
    ilf_in: usize,
    cur: usize,
    ilf_cur: usize,
    out_layer: usize,
    out_image: usize,
    out_tf: usize,
    ilf_in_layer: usize,
    select: usize,
    import: usize,
    import_sel: usize,
    cur_v: usize,
    cur_sel: usize,
    out_img: usize,
    out_res: usize,
    tf: usize,
    tf2: usize,
    def_sel: usize,
    def_filter: usize,
}

impl W {
    fn label(&mut self, parent: usize) {
        let c = self.t.obj_with("CLabelColor", &[("customizedColorInt", "-1".into())]);
        self.t.enum_v(c, "CLabelColorType", "labelType", "UNDEFINED");
        self.t.set(parent, "labelColor", c);
    }

    fn icon(&mut self, side: u32, path: &str) -> usize {
        let icon = self.t.obj("CImageIcon");
        let img = self.t.obj_with(
            "CWritableImage",
            &[
                ("width", side.to_string()),
                ("height", side.to_string()),
                ("type", "INT_ARGB".into()),
            ],
        );
        let file = self.t.obj_with("file", &[("path", path.to_string())]);
        self.t.set(img, "image", file);
        self.t.set(icon, "image", img);
        icon
    }

    fn resource(&mut self, w: u32, h: u32, size: usize, path: &str) -> usize {
        let r = self.t.obj_with(
            "CImageResource",
            &[
                ("width", w.to_string()),
                ("height", h.to_string()),
                ("type", "INT_ARGB".into()),
                ("imageFileBuf_size", size.to_string()),
                ("previewFileBuf_size", "0".into()),
            ],
        );
        let file = self.t.obj_with("file", &[("path", path.to_string())]);
        self.t.set(r, "imageFileBuf", file);
        r
    }

    fn cache(&mut self, parent: usize, res: usize, w: u32, h: u32) {
        let m = self.t.obj("CCachedImageManager");
        self.t.enum_v(m, "CachedImageType", "defaultCacheType", "SCALE_1");
        self.t.set(m, "rawImage", res);
        let list = self.t.list(m, "array_list", "cachedImages");
        let c = self.t.obj("CCachedImage");
        self.t.set(c, "_cachedImageResource", res);
        self.t.b(c, "isSharedImage", true);
        let size = self.t.obj_with("CSize", &[("width", w.to_string()), ("height", h.to_string())]);
        self.t.set(c, "rawImageSize", size);
        self.t.i(c, "reductionRatio", 1);
        self.t.i(c, "mipmapLevel", 64);
        self.t.b(c, "hasMargin", false);
        self.t.b(c, "isCleaned", false);
        self.t.identity(c, "transformRawImageToCachedImage");
        self.t.push(list, c);
        self.t.i(m, "requiredMipmapLevel", 64);
        self.t.set(parent, "cachedImageManager", m);
    }

    fn guid(&mut self, tag: &'static str, note: &str) -> usize {
        let u = self.ids.uuid();
        self.t.guid(tag, &u, note)
    }

    fn extension_super(&mut self, ext: usize, owner: usize) {
        let sup = self.t.obj("ACExtension");
        let u = self.ids.uuid();
        let g = self.t.obj_with("CExtensionGuid", &[("uuid", u), ("note", NOTE_NONE.into())]);
        self.t.set(sup, "guid", g);
        self.t.set(sup, "_owner", owner);
        self.t.set(ext, "super", sup);
    }

    /// The common block of parts, deformers and meshes.
    fn controllable(
        &mut self,
        name: &str,
        visible: bool,
        parent: Option<usize>,
        grid: usize,
        extensions: &[usize],
    ) -> usize {
        let c = self.t.obj("ACParameterControllableSource");
        self.t.s(c, "localName", name);
        self.t.b(c, "isVisible", visible);
        self.t.b(c, "isLocked", false);
        match parent {
            Some(p) => self.t.set(c, "parentGuid", p),
            None => self.t.null(c, "parentGuid"),
        }
        self.t.set(c, "keyformGridSource", grid);
        let morph = self.t.obj("KeyFormMorphTargetSet");
        self.t.list(morph, "carray_list", "_morphTargets");
        let cons = self.t.obj("MorphTargetBlendWeightConstraintSet");
        self.t.list(cons, "carray_list", "_constraints");
        self.t.set(morph, "blendWeightConstraintSet", cons);
        self.t.set(c, "keyformMorphTargetSet", morph);
        let exts = self.t.list(c, "carray_list", "_extensions");
        for &e in extensions {
            self.t.push(exts, e);
        }
        self.t.null(c, "internalColor_direct_argb");
        self.label(c);
        c
    }

    /// A keyform grid over the binding axes: one KeyformOnGrid per slot,
    /// each with a fresh form guid. Returns the grid and the guids by slot.
    fn grid(
        &mut self,
        axes: &[(usize, Vec<f32>)],
        param_guids: &[usize],
        slots: usize,
    ) -> (usize, Vec<usize>) {
        let grid = self.t.obj("KeyformGridSource");
        let on_grid = self.t.list(grid, "array_list", "keyformsOnGrid");
        let mut bindings = Vec::with_capacity(axes.len());
        for (p, keys) in axes {
            let b = self.t.obj("KeyformBindingSource");
            self.t.share(b);
            self.t.set(b, "_gridSource", grid);
            if let Some(&g) = param_guids.get(*p) {
                self.t.set(b, "parameterGuid", g);
            }
            let kl = self.t.list(b, "array_list", "keys");
            for k in keys {
                let f = self.t.obj("f");
                if let Some(n) = self.t.nodes.get_mut(f) {
                    n.text = fmt_f(*k);
                }
                self.t.push(kl, f);
            }
            self.t.enum_v(b, "InterpolationType", "interpolationType", "LINEAR");
            self.t.enum_v(b, "ExtendedInterpolationType", "extendedInterpolationType", "LINEAR");
            self.t.i(b, "insertPointCount", 1);
            self.t.f(b, "extendedInterpolationScale", 1.0);
            self.t.s(b, "description", "");
            bindings.push(b);
        }
        let lens: Vec<usize> = axes.iter().map(|(_, k)| k.len().max(1)).collect();
        let mut guids = Vec::with_capacity(slots);
        for slot in 0..slots {
            let kog = self.t.obj("KeyformOnGrid");
            let ak = self.t.obj("KeyformGridAccessKey");
            let list = self.t.list(ak, "array_list", "_keyOnParameterList");
            for (axis, c) in coords(slot, &lens).into_iter().enumerate() {
                let kop = self.t.obj("KeyOnParameter");
                if let Some(&b) = bindings.get(axis) {
                    self.t.set(kop, "binding", b);
                }
                self.t.i(kop, "keyIndex", c as i64);
                self.t.push(list, kop);
            }
            self.t.set(kog, "accessKey", ak);
            let g = self.guid("CFormGuid", "Key [  ]");
            self.t.set(kog, "keyformGuid", g);
            self.t.push(on_grid, kog);
            guids.push(g);
        }
        let bl = self.t.list(grid, "array_list", "keyformBindings");
        for b in bindings {
            self.t.push(bl, b);
        }
        (grid, guids)
    }

    fn ac_form(&mut self, guid: usize, source: usize) -> usize {
        let f = self.t.obj("ACForm");
        self.t.set(f, "guid", guid);
        self.t.b(f, "isAnimatedForm", false);
        self.t.b(f, "isLocalAnimatedForm", false);
        self.t.set(f, "_source", source);
        self.t.null(f, "name");
        self.t.s(f, "notes", "");
        f
    }

    fn colors(&mut self, form: usize, coord: usize) {
        let m = self.t.obj_with(
            "CFloatColor",
            &[
                ("red", "1.0".into()),
                ("green", "1.0".into()),
                ("blue", "1.0".into()),
                ("alpha", "1.0".into()),
            ],
        );
        self.t.set(form, "multiplyColor", m);
        let s = self.t.obj_with(
            "CFloatColor",
            &[
                ("red", "0.0".into()),
                ("green", "0.0".into()),
                ("blue", "0.0".into()),
                ("alpha", "1.0".into()),
            ],
        );
        self.t.set(form, "screenColor", s);
        self.t.set(form, "coordType", coord);
    }

    /// The two-filter graph every model image reads its layer through.
    fn filter_set(&mut self) -> usize {
        let t = &mut self.t;
        let fv = &self.fv;
        let set = t.obj("ModelImageFilterSet");
        let sup = t.obj("FilterSet");
        let iid0 = t.obj_with("FilterInstanceId", &[("idstr", "filter0".into())]);
        let iid1 = t.obj_with("FilterInstanceId", &[("idstr", "filter1".into())]);
        let inst0 = t.obj_with("FilterInstance", &[("filterName", "CLayerSelector".into())]);
        let inst1 = t.obj_with("FilterInstance", &[("filterName", "CLayerFilter".into())]);
        let conn = t.obj("FilterOutputValueConnector");
        let c_sup = t.obj("AValueConnector");
        t.set(conn, "super", c_sup);
        t.set(conn, "instance", inst0);
        t.set(conn, "id", fv.out_layer);
        t.set(conn, "valueDef", fv.select);
        // filter0: picks the layer
        t.set(inst0, "filterDefGuid", fv.def_sel);
        t.null(inst0, "filterDef");
        t.set(inst0, "filterId", iid0);
        let inputs = t.list(inst0, "hash_map", "inputConnectors");
        for (key, env) in [(fv.ilf_in, fv.in_layer), (fv.ilf_cur, fv.cur)] {
            let e = t.obj("entry");
            t.set(e, "key", key);
            let v = t.obj("EnvValueConnector");
            let vs = t.obj("AValueConnector");
            t.set(v, "super", vs);
            t.set(v, "envValueId", env);
            t.set(e, "value", v);
            t.push(inputs, e);
        }
        let outputs = t.list(inst0, "hash_map", "outputConnectors");
        let e = t.obj("entry");
        t.set(e, "key", fv.out_layer);
        t.set(e, "value", conn);
        t.push(outputs, e);
        t.set(inst0, "ownerFilterSet", set);
        // filter1: the layer as an image
        t.set(inst1, "filterDefGuid", fv.def_filter);
        t.null(inst1, "filterDef");
        t.set(inst1, "filterId", iid1);
        let inputs = t.list(inst1, "hash_map", "inputConnectors");
        let e = t.obj("entry");
        t.set(e, "key", fv.ilf_in_layer);
        t.set(e, "value", conn);
        t.push(inputs, e);
        t.list(inst1, "hash_map", "outputConnectors");
        t.set(inst1, "ownerFilterSet", set);
        // the set itself
        let map = t.list(sup, "linked_map", "filterMap");
        for (k, v) in [(iid0, inst0), (iid1, inst1)] {
            let e = t.obj("entry");
            t.set(e, "key", k);
            t.set(e, "value", v);
            t.push(map, e);
        }
        let ins = t.list(sup, "linked_map", "_externalInputs");
        for (key, env_def, filter, val_def) in
            [(fv.in_layer, fv.import, inst0, fv.import_sel), (fv.cur, fv.cur_v, inst0, fv.cur_sel)]
        {
            let e = t.obj("entry");
            t.set(e, "key", key);
            let c = t.obj("EnvConnection");
            t.set(c, "_envValueDef", env_def);
            t.set(c, "filter", filter);
            t.set(c, "filterValueDef", val_def);
            t.set(e, "value", c);
            t.push(ins, e);
        }
        let outs = t.list(sup, "linked_map", "_externalOutputs");
        for (key, env_def, filter, val_def) in
            [(fv.out_image, fv.out_img, inst1, fv.out_res), (fv.out_tf, fv.tf, inst1, fv.tf2)]
        {
            let e = t.obj("entry");
            t.set(e, "key", key);
            let c = t.obj("EnvConnection");
            t.set(c, "_envValueDef", env_def);
            t.set(c, "filter", filter);
            t.set(c, "filterValueDef", val_def);
            t.set(e, "value", c);
            t.push(outs, e);
        }
        t.set(set, "super", sup);
        set
    }

    /// A model image reading one layer: the filter set, its environment
    /// pointing at the layer, the filtered image and the placement.
    fn model_image(
        &mut self,
        guid: usize,
        name: &str,
        layer: usize,
        res: usize,
        icon16: usize,
        mat: &Material,
    ) -> usize {
        let set = self.filter_set();
        let t = &mut self.t;
        let mi = t.obj_with("CModelImage", &[("modelImageVersion", "0".into())]);
        t.set(mi, "guid", guid);
        t.s(mi, "name", name);
        t.set(mi, "inputFilter", set);
        let env = t.obj("ModelImageFilterEnv");
        let fe = t.obj("FilterEnv");
        t.null(fe, "parentEnv");
        let values = t.list(fe, "hash_map", "envValues");
        let e = t.obj("entry");
        t.set(e, "key", self.fv.cur);
        let vs = t.obj("EnvValueSet");
        t.set(vs, "id", self.fv.cur);
        t.set(vs, "value", self.layered_guid);
        t.l(vs, "updateTimeMs", TIME_MS);
        t.set(e, "value", vs);
        t.push(values, e);
        let e = t.obj("entry");
        t.set(e, "key", self.fv.in_layer);
        let vs = t.obj("EnvValueSet");
        t.set(vs, "id", self.fv.in_layer);
        let sel = t.obj("CLayerSelectorMap");
        let m = t.list(sel, "linked_map", "_imageToLayerInput");
        let me = t.obj("entry");
        t.set(me, "key", self.layered_guid);
        let l = t.list(me, "array_list", "value");
        let lid = t.obj("CLayerInputData");
        t.set(lid, "layer", layer);
        t.identity(lid, "affine");
        t.null(lid, "clippingOnTexturePx");
        t.push(l, lid);
        t.push(m, me);
        t.set(vs, "value", sel);
        t.l(vs, "updateTimeMs", TIME_MS);
        t.set(e, "value", vs);
        t.push(values, e);
        t.set(env, "super", fe);
        t.set(mi, "inputFilterEnv", env);
        t.set(mi, "_filteredImage", res);
        t.set(mi, "icon16", icon16);
        t.affine(
            mi,
            "_materialLocalToCanvasTransform",
            [1.0, 0.0, mat.left as f32, 0.0, 1.0, mat.top as f32],
        );
        t.set(mi, "_group", self.group);
        let linked = t.list(mi, "carray_list", "linkedRawImageGuids");
        t.push(linked, self.layered_guid);
        self.cache(mi, res, mat.width, mat.height);
        self.t.s(mi, "memo", "");
        mi
    }

    /// One layer of the layered image, holding a material.
    fn layer(
        &mut self,
        name: &str,
        res: usize,
        mat: &Material,
        icon16: usize,
        icon64: usize,
    ) -> usize {
        let t = &mut self.t;
        let layer = t.obj("CLayer");
        t.share(layer);
        let il = t.obj("ACImageLayer");
        let entry = t.obj("ACLayerEntry");
        t.s(entry, "name", name);
        t.s(entry, "memo", "");
        t.b(entry, "isVisible", true);
        t.b(entry, "isClipping", false);
        t.set(entry, "blend", self.blend);
        let u = self.ids.uuid();
        let g = t.obj_with("CLayerGuid", &[("uuid", u), ("note", NOTE_NONE.into())]);
        t.set(entry, "guid", g);
        t.set(entry, "group", self.layer_root);
        t.i(entry, "opacity255", 255);
        t.set(entry, "_optionOfIOption", self.opts);
        t.set(entry, "_layeredImage", self.layered);
        t.set(il, "super", entry);
        t.set(layer, "super", il);
        t.set(layer, "imageResource", res);
        let rect = t.obj("CRect");
        t.i(rect, "x", i64::from(mat.left));
        t.i(rect, "y", i64::from(mat.top));
        t.i(rect, "width", i64::from(mat.width));
        t.i(rect, "height", i64::from(mat.height));
        t.set(layer, "boundsOnImageDoc", rect);
        let ident = t.obj("CLayerIdentifier");
        t.s(ident, "layerName", name);
        t.null(ident, "layerId");
        t.i(ident, "layerIdValue_testImpl", -1);
        t.set(layer, "layerIdentifier", ident);
        t.set(layer, "icon16", icon16);
        t.set(layer, "icon64", icon64);
        t.list(layer, "linked_map", "layerInfo");
        t.set(layer, "_optionOfIOption", self.opts);
        layer
    }
}

/// The project as archive bytes.
pub fn to_cmo3(x: &Cmo3Export) -> Result<Vec<u8>, ImportError> {
    let model = x.model;
    let c = model.canvas;
    let (cw, ch) = (c.width.round() as i64, c.height.round() as i64);
    if !(1..=65536).contains(&cw) || !(1..=65536).contains(&ch) {
        return Err(err("the canvas has no size"));
    }
    let (cw, ch) = (cw as u32, ch as u32);
    let ppu = if c.pixels_per_unit.is_finite() && c.pixels_per_unit.abs() > 1e-6 {
        c.pixels_per_unit
    } else {
        return Err(err("the canvas has no pixel scale"));
    };
    let to_px = |space: Space, v: f32, origin: f32| match space {
        Space::Root => v * ppu + origin,
        Space::Warp => v,
        Space::Rotation => v * ppu,
    };
    let convert = |space: Space, pts: &[f32]| -> Vec<f32> {
        pts.as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [to_px(space, p[0], c.origin_x), to_px(space, p[1], c.origin_y)])
            .collect()
    };
    if model.art_meshes.is_empty() {
        return Err(err("the model has no art meshes"));
    }

    // 1. where every mesh rests, with everything shown
    let mut shown = model.clone();
    for p in &mut shown.parts {
        p.visible = true;
        p.enabled = true;
    }
    for d in &mut shown.deformers {
        d.visible = true;
        d.enabled = true;
    }
    for m in &mut shown.art_meshes {
        m.visible = true;
        m.enabled = true;
    }
    shown.physics = None;
    let mut puppet = ayatsuri_core::Puppet::new(shown);
    puppet.update();
    let rest: HashMap<usize, Vec<f32>> =
        puppet.drawables().map(|d| (d.mesh, convert(Space::Root, d.positions))).collect();
    drop(puppet);

    // 2. the art of each mesh, back out of the texture
    let mut pages: Vec<Option<image::RgbaImage>> = Vec::with_capacity(x.textures.len());
    for t in x.textures {
        pages.push(image::load_from_memory(t).ok().map(|i| i.to_rgba8()));
    }
    let mut materials = Vec::with_capacity(model.art_meshes.len());
    let mut points: Vec<Vec<f32>> = Vec::with_capacity(model.art_meshes.len());
    for (k, m) in model.art_meshes.iter().enumerate() {
        let per = m.vertex_count as usize * 2;
        let px = match rest.get(&k) {
            Some(p) if p.len() == per => p.clone(),
            _ => convert(Space::Root, m.positions.get(..per).unwrap_or(&[])),
        };
        if px.len() != per || per == 0 {
            return Err(err(format!("{}: no resting shape", m.id)));
        }
        let tex = pages.get(m.texture as usize).and_then(Option::as_ref);
        materials.push(material(tex, &m.uvs, &px).map_err(|e| err(format!("{}: {e}", m.id)))?);
        points.push(px);
    }
    drop(pages);

    // 3. pack the materials into fresh atlases, the way an import would
    let mut layers_in = Vec::with_capacity(materials.len());
    for (m, mat) in model.art_meshes.iter().zip(&materials) {
        let mut img = mat.rgba.clone();
        let (w, h) = (img.width(), img.height());
        // an all but invisible pixel in each corner keeps the trim from
        // moving the layer
        for (px, py) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            let p = img.get_pixel_mut(px, py);
            if p.0[3] == 0 {
                p.0 = [0, 0, 0, 1];
            }
        }
        layers_in.push(Layer {
            name: m.id.clone(),
            groups: Vec::new(),
            x: mat.left,
            y: mat.top,
            width: w,
            height: h,
            rgba: img.into_raw(),
            visible: true,
            opacity: 1.0,
            clip_to_below: false,
            blend: Blend::Normal,
        });
    }
    let packed = layers::build(&Document { width: cw, height: ch, layers: layers_in })?;
    let pc = packed.model.canvas;
    let page = PAGE as f32;
    let mut placed: Vec<Placed> = Vec::with_capacity(materials.len());
    for (m, mat) in model.art_meshes.iter().zip(&materials) {
        let j = packed
            .mesh_names
            .iter()
            .position(|n| *n == m.id)
            .ok_or_else(|| err(format!("{}: lost in packing", m.id)))?;
        let q = packed.model.art_meshes.get(j).ok_or_else(|| err("packed mesh"))?;
        let g = |v: &[f32], i: usize| v.get(i).copied().unwrap_or(0.0);
        let left = g(&q.positions, 0) * pc.pixels_per_unit + pc.origin_x;
        let top = g(&q.positions, 1) * pc.pixels_per_unit + pc.origin_y;
        let right = g(&q.positions, 4) * pc.pixels_per_unit + pc.origin_x;
        let bottom = g(&q.positions, 5) * pc.pixels_per_unit + pc.origin_y;
        if (left - mat.left as f32).abs() > 0.51 || (top - mat.top as f32).abs() > 0.51 {
            return Err(err(format!("{}: the packer moved its art", m.id)));
        }
        let (ax, ay) = (g(&q.uvs, 0) * page, g(&q.uvs, 1) * page);
        let (aw, ah) = ((g(&q.uvs, 4) - g(&q.uvs, 0)) * page, (g(&q.uvs, 5) - g(&q.uvs, 1)) * page);
        let sx = if right - left > 0.0 { aw / (right - left) } else { 1.0 };
        let sy = if bottom - top > 0.0 { ah / (bottom - top) } else { 1.0 };
        placed.push(Placed { page: q.texture as usize, ax, ay, sx, sy });
    }

    // 4. the object graph
    let seed = x
        .name
        .bytes()
        .fold(0x9e37_79b9_7f4a_7c15u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    let mut t = Tree::default();
    let coord_basic = t.obj("CoordType");
    t.s(coord_basic, "coordName", "Basic Coord");
    t.share(coord_basic);
    let coord_local = t.obj("CoordType");
    t.s(coord_local, "coordName", "DeformerLocal");
    t.share(coord_local);
    let blend = t.obj("CBlend_Normal");
    let bs = t.obj("ACBlend");
    t.set(blend, "super", bs);
    t.share(blend);
    let opts = t.obj_with("hash_map", &[("keyType", "string".into())]);
    t.share(opts);
    let mut ids = Ids(seed | 1);
    let root_u = ids.uuid();
    let root_deformer = t.guid("CDeformerGuid", &root_u, "ROOT");
    let zero_deformer = t.guid("CDeformerGuid", ZERO_UUID, "NOT INITIALIZED");
    let lg = ids.uuid();
    let layered_guid = t.guid("CLayeredImageGuid", &lg, NOTE_NONE);
    let layered = t.obj("CLayeredImage");
    t.share(layered);
    let layer_root = t.obj("CLayerGroup");
    t.share(layer_root);
    let group = t.obj("CModelImageGroup");
    t.share(group);
    let fid = |t: &mut Tree, id: &str| {
        let n = t.obj_with("FilterValueId", &[("idstr", id.to_string())]);
        t.share(n)
    };
    let in_layer = fid(&mut t, "mi_input_layerInputData");
    let ilf_in = fid(&mut t, "ilf_inputLayerData");
    let cur = fid(&mut t, "mi_currentImageGuid");
    let ilf_cur = fid(&mut t, "ilf_currentImageGuid");
    let out_layer = fid(&mut t, "ilf_outputLayerData");
    let out_image = fid(&mut t, "mi_output_image");
    let out_tf = fid(&mut t, "mi_output_transform");
    let ilf_in_layer = fid(&mut t, "ilf_inputLayer");
    let fval = |t: &mut Tree, name: &str, id: usize| {
        let n = t.obj("FilterValue");
        t.s(n, "name", name);
        t.set(n, "id", id);
        t.null(n, "defaultValueInitializer");
        t.share(n)
    };
    let out_res_id = t.obj_with("FilterValueId", &[("idstr", "ilf_outputImageRes".into())]);
    let out_tf_id = t.obj_with("FilterValueId", &[("idstr", "ilf_outputTransform".into())]);
    let fv = FilterVals {
        select: fval(&mut t, "Select Layer", out_layer),
        import: fval(&mut t, "Import Layer", in_layer),
        import_sel: fval(&mut t, "Import Layer selection", ilf_in),
        cur_v: fval(&mut t, "Current GUID", cur),
        cur_sel: fval(&mut t, "GUID of Selected Source Image", ilf_cur),
        out_img: fval(&mut t, "Output image", out_image),
        out_res: fval(&mut t, "Output Image (Resource Format)", out_res_id),
        tf: fval(&mut t, "LayerToCanvas変換", out_tf),
        tf2: fval(&mut t, "LayerToCanvas変換", out_tf_id),
        def_sel: t.guid("StaticFilterDefGuid", "5e9fe1ea-0ec3-4d68-a5fa-018fc7abe301", NOTE_NONE),
        def_filter: t.guid(
            "StaticFilterDefGuid",
            "4083cd1f-40ba-4eda-8400-379019d55ed8",
            NOTE_NONE,
        ),
        in_layer,
        ilf_in,
        cur,
        ilf_cur,
        out_layer,
        out_image,
        out_tf,
        ilf_in_layer,
    };
    let mut w = W {
        t,
        ids,
        files: Files { list: Vec::new(), images: 0, icons: 0 },
        coord_basic,
        coord_local,
        blend,
        opts,
        root_deformer,
        zero_deformer,
        layered_guid,
        layered,
        layer_root,
        group,
        fv,
    };

    // 4a. parameters, their guids and folders
    let param_guids: Vec<usize> =
        model.parameters.iter().map(|p| w.guid("CParameterGuid", &p.id)).collect();
    let param_index: HashMap<&str, usize> =
        model.parameters.iter().enumerate().map(|(i, p)| (p.id.as_str(), i)).collect();
    let root_group_guid = w.guid("CParameterGroupGuid", "Root Parameter Group");
    let folder_guids: Vec<usize> =
        x.param_groups.iter().map(|_| w.guid("CParameterGroupGuid", NOTE_NONE)).collect();
    let mut folder_of: Vec<Option<usize>> = vec![None; model.parameters.len()];
    for (fi, (_, members)) in x.param_groups.iter().enumerate() {
        for &m in members {
            if let Some(slot) = folder_of.get_mut(m as usize) {
                *slot = Some(fi);
            }
        }
    }
    let name_or = |names: &[String], i: usize, id: &str| {
        names.get(i).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| id.to_string())
    };

    // 4b. parts: the model's tree under one root part
    let part_guids: Vec<usize> = model.parts.iter().map(|p| w.guid("CPartGuid", &p.id)).collect();
    let root_part_guid = w.guid("CPartGuid", "__RootPart__");
    let part_guid = |p: Option<u32>| -> usize {
        p.and_then(|i| part_guids.get(i as usize).copied()).unwrap_or(root_part_guid)
    };
    let part_nodes: Vec<usize> = model.parts.iter().map(|_| w.t.obj("CPartSource")).collect();
    let root_part = w.t.obj("CPartSource");
    for &n in part_nodes.iter().chain(std::iter::once(&root_part)) {
        w.t.share(n);
    }

    // 4c. deformers, parents first, each in its parent's space
    let deformer_guids: Vec<usize> =
        model.deformers.iter().map(|d| w.guid("CDeformerGuid", &d.id)).collect();
    let deformer_nodes: Vec<usize> = model
        .deformers
        .iter()
        .map(|d| {
            w.t.obj(match d.kind {
                DeformerKind::Warp(_) => "CWarpDeformerSource",
                DeformerKind::Rotation(_) => "CRotationDeformerSource",
            })
        })
        .collect();
    for &n in &deformer_nodes {
        w.t.share(n);
    }
    let mut spaces: Vec<Space> = Vec::with_capacity(model.deformers.len());
    let space_under = |parent: Option<u32>, spaces: &[Space]| -> Space {
        parent.and_then(|i| spaces.get(i as usize).copied()).unwrap_or(Space::Root)
    };
    for (i, d) in model.deformers.iter().enumerate() {
        let Some(&node) = deformer_nodes.get(i) else { continue };
        let space = space_under(d.parent_deformer, &spaces);
        let coord = match space {
            Space::Root => w.coord_basic,
            _ => w.coord_local,
        };
        let axes: Vec<(usize, Vec<f32>)> =
            d.binding.axes.iter().map(|a| (a.parameter as usize, a.keys.clone())).collect();
        let (opacities, slots) = match &d.kind {
            DeformerKind::Warp(wd) => (&wd.opacities, wd.opacities.len().max(1)),
            DeformerKind::Rotation(r) => (&r.opacities, r.angles.len().max(1)),
        };
        let (grid, form_guids) = w.grid(&axes, &param_guids, slots);
        let ctrl = w.controllable(&d.id, d.visible, Some(part_guid(d.parent_part)), grid, &[]);
        let dsrc = w.t.obj("ACDeformerSource");
        w.t.set(dsrc, "super", ctrl);
        if let Some(&g) = deformer_guids.get(i) {
            w.t.set(dsrc, "guid", g);
        }
        let did = w.t.obj_with("CDeformerId", &[("idstr", d.id.clone())]);
        w.t.set(dsrc, "id", did);
        let target = d
            .parent_deformer
            .and_then(|p| deformer_guids.get(p as usize).copied())
            .unwrap_or(w.root_deformer);
        w.t.set(dsrc, "targetDeformerGuid", target);
        w.t.set(node, "super", dsrc);
        match &d.kind {
            DeformerKind::Warp(wd) => {
                w.t.i(node, "col", i64::from(wd.cols));
                w.t.i(node, "row", i64::from(wd.rows));
                w.t.b(node, "isQuadTransform", wd.quad);
                let forms = w.t.list(node, "carray_list", "keyforms");
                let per = ((wd.rows + 1) * (wd.cols + 1) * 2) as usize;
                for (s, &g) in form_guids.iter().enumerate() {
                    let form = w.t.obj("CWarpDeformerForm");
                    let df = w.t.obj("ACDeformerForm");
                    let acf = w.ac_form(g, node);
                    w.t.set(df, "super", acf);
                    w.t.f(df, "opacity", opacities.get(s).copied().unwrap_or(1.0));
                    w.colors(df, coord);
                    w.t.set(form, "super", df);
                    let pts = wd.positions.get(s * per..(s + 1) * per).unwrap_or(&[]);
                    let pts = convert(space, pts);
                    w.t.floats(form, "positions", &pts);
                    w.t.push(forms, form);
                }
                spaces.push(Space::Warp);
            }
            DeformerKind::Rotation(r) => {
                w.t.b(node, "useBoneUi_testImpl", true);
                let forms = w.t.list(node, "carray_list", "keyforms");
                for (s, &g) in form_guids.iter().enumerate() {
                    let at = |v: &[f32], d: f32| v.get(s).copied().unwrap_or(d);
                    let (ox, oy) = (
                        to_px(space, at(&r.origins_x, 0.0), c.origin_x),
                        to_px(space, at(&r.origins_y, 0.0), c.origin_y),
                    );
                    let form = w.t.obj_with(
                        "CRotationDeformerForm",
                        &[
                            ("angle", fmt_f(at(&r.angles, 0.0))),
                            ("originX", fmt_f(ox)),
                            ("originY", fmt_f(oy)),
                            ("scale", fmt_f(at(&r.scales, 1.0))),
                            (
                                "isReflectX",
                                r.reflect_x.get(s).copied().unwrap_or(false).to_string(),
                            ),
                            (
                                "isReflectY",
                                r.reflect_y.get(s).copied().unwrap_or(false).to_string(),
                            ),
                        ],
                    );
                    let df = w.t.obj("ACDeformerForm");
                    let acf = w.ac_form(g, node);
                    w.t.set(df, "super", acf);
                    w.t.f(df, "opacity", opacities.get(s).copied().unwrap_or(1.0));
                    w.colors(df, coord);
                    w.t.set(form, "super", df);
                    w.t.push(forms, form);
                }
                w.t.f(node, "handleLengthOnCanvas", 400.0);
                w.t.f(node, "circleRadiusOnCanvas", 400.0 / 3.0);
                w.t.f(node, "baseAngle", r.base_angle);
                spaces.push(Space::Rotation);
            }
        }
    }

    // 4d. atlases, one per packed page
    let atlas_imgs: Vec<&Vec<u8>> = packed.textures.iter().collect();
    let mut atlas_nodes = Vec::with_capacity(atlas_imgs.len());
    let mut atlas_lists = Vec::with_capacity(atlas_imgs.len());
    let mut textures = Vec::with_capacity(atlas_imgs.len());
    let mut atlas_guids = Vec::with_capacity(atlas_imgs.len());
    for (n, bytes) in atlas_imgs.iter().enumerate() {
        let path = w.files.image((*bytes).clone());
        let res = w.resource(PAGE, PAGE, bytes.len(), &path);
        w.t.share(res);
        let atlas = w.t.obj("CTextureAtlas");
        w.t.share(atlas);
        let name = format!("TextureAtlas{}", n + 1);
        w.t.s(atlas, "name", &name);
        w.t.i(atlas, "width", i64::from(PAGE));
        w.t.i(atlas, "height", i64::from(PAGE));
        w.t.set(atlas, "cachedAtlasImage", res);
        w.t.b(atlas, "lockCachedAtlasImage", false);
        let g = w.guid("CTextureAtlasGuid", NOTE_NONE);
        w.t.set(atlas, "guid", g);
        let list = w.t.list(atlas, "carray_list", "modelImages");
        w.cache(atlas, res, PAGE, PAGE);
        let tex = w.t.obj("GTexture2D");
        w.t.share(tex);
        let gt = w.t.obj("GTexture");
        w.t.s(gt, "name", &name);
        w.t.enum_v(gt, "WrapMode", "wrapMode", "CLAMP_TO_BORDER");
        let fm = w.t.obj("FilterMode");
        w.t.set(fm, "owner", tex);
        w.t.enum_v(fm, "MinFilter", "minFilter", "LINEAR_MIPMAP_LINEAR");
        w.t.enum_v(fm, "MagFilter", "magFilter", "LINEAR");
        w.t.set(gt, "filterMode", fm);
        let u = w.ids.uuid();
        let tg = w.t.obj_with("GTextureGuid", &[("uuid", u), ("note", NOTE_NONE.into())]);
        w.t.set(gt, "guid", tg);
        w.t.enum_v(gt, "Anisotropy", "anisotropy", "ON");
        w.t.set(tex, "super", gt);
        w.t.set(tex, "srcImageResource", res);
        w.t.identity(tex, "transformImageResource01toLogical01");
        w.t.i(tex, "mipmapLevel", 64);
        w.t.b(tex, "isPremultiplied", true);
        atlas_nodes.push(atlas);
        atlas_lists.push(list);
        textures.push(tex);
        atlas_guids.push(g);
    }

    // 4e. the layered image the materials are layers of
    let layer_entries = w.t.obj("carray_list");
    let layer_children = w.t.obj("carray_list");
    let model_images = w.t.obj("carray_list");

    // 4f. meshes: art, mesh, keyforms and the texture pipeline
    let mesh_guids: Vec<usize> =
        model.art_meshes.iter().map(|m| w.guid("CDrawableGuid", &m.id)).collect();
    let mesh_nodes: Vec<usize> =
        model.art_meshes.iter().map(|_| w.t.obj("CArtMeshSource")).collect();
    for &n in &mesh_nodes {
        w.t.share(n);
    }
    for (k, m) in model.art_meshes.iter().enumerate() {
        let (Some(&node), Some(mat), Some(pl), Some(px)) =
            (mesh_nodes.get(k), materials.get(k), placed.get(k), points.get(k))
        else {
            continue;
        };
        let name = name_or(x.mesh_names, k, &m.id);
        // the art: layer, resources, icons, model image, atlas entry
        let bytes = png(&mat.rgba)?;
        let size = bytes.len();
        let path = w.files.image(bytes);
        let res_layer = w.resource(mat.width, mat.height, size, &path);
        let res_filtered = w.resource(mat.width, mat.height, size, &path);
        let i16 = thumb(&mat.rgba, 16)?;
        let i32_ = thumb(&mat.rgba, 32)?;
        let i64_ = thumb(&mat.rgba, 64)?;
        let (p16, p32, p64) = (w.files.icon(i16), w.files.icon(i32_), w.files.icon(i64_));
        let icon16 = w.icon(16, &p16);
        w.t.share(icon16);
        let icon32 = w.icon(32, &p32);
        let icon64 = w.icon(64, &p64);
        let layer = w.layer(&name, res_layer, mat, icon16, icon64);
        w.t.push(layer_entries, layer);
        w.t.push(layer_children, layer);
        let mi_guid = w.guid("CModelImageGuid", &name);
        let mi = w.model_image(mi_guid, &name, layer, res_filtered, icon16, mat);
        w.t.push(model_images, mi);
        let to_canvas = [
            1.0 / pl.sx.max(1e-6),
            0.0,
            mat.left as f32 - pl.ax / pl.sx.max(1e-6),
            0.0,
            1.0 / pl.sy.max(1e-6),
            mat.top as f32 - pl.ay / pl.sy.max(1e-6),
        ];
        if let (Some(&atlas), Some(&list)) = (atlas_nodes.get(pl.page), atlas_lists.get(pl.page)) {
            let e = w.t.obj("ModelImageEntry");
            w.t.set(e, "atlas", atlas);
            w.t.set(e, "modelImageGuid", mi_guid);
            w.t.affine(e, "atlasLocalToCanvasTransform", to_canvas);
            let tf = w.t.obj("GTransform2");
            let pos = w.t.obj("GVector2");
            w.t.f(pos, "x", pl.ax);
            w.t.f(pos, "y", pl.ay);
            w.t.set(tf, "position", pos);
            let sc = w.t.obj("GVector2");
            w.t.f(sc, "x", pl.sx);
            w.t.f(sc, "y", pl.sy);
            w.t.set(tf, "scale", sc);
            w.t.f(tf, "eulerAngle", 0.0);
            w.t.set(e, "materialLocalToAtlasTransform", tf);
            w.t.push(list, e);
        }
        // the texture input extension: the model image, and its atlas region
        let tie = w.t.obj("CTextureInputExtension");
        w.t.share(tie);
        w.extension_super(tie, node);
        let inputs = w.t.list(tie, "carray_list", "_textureInputs");
        let ti = w.t.obj("CTextureInput_ModelImage");
        let tis = w.t.obj("ACTextureInput");
        w.t.identity(tis, "optionalTransformOnCanvas");
        w.t.set(tis, "_owner", tie);
        w.t.set(ti, "super", tis);
        w.t.set(ti, "_modelImageGuid", mi_guid);
        w.t.push(inputs, ti);
        let region = w.t.obj("CTextureInput_TextureAtlasRegion");
        w.t.share(region);
        let rs = w.t.obj("ACTextureInput");
        w.t.identity(rs, "optionalTransformOnCanvas");
        w.t.set(rs, "_owner", tie);
        w.t.set(region, "super", rs);
        if let Some(&g) = atlas_guids.get(pl.page) {
            w.t.set(region, "textureAtlasGuid", g);
        }
        w.t.affine(region, "inputImageLocalToCanvasTransform", to_canvas);
        w.t.push(inputs, region);
        w.t.set(tie, "currentTextureInputData", region);
        // the editable mesh: points, edges and priorities
        let n = m.vertex_count as usize;
        let mut edges: Vec<(u16, u16)> = Vec::with_capacity(m.indices.len());
        for tri in m.indices.as_chunks::<3>().0 {
            for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                edges.push((a.min(b), a.max(b)));
            }
        }
        edges.sort_unstable();
        edges.dedup();
        let edge_vals: Vec<i64> =
            edges.iter().flat_map(|(a, b)| [i64::from(*a), i64::from(*b)]).collect();
        let em = w.t.obj("CEditableMeshExtension");
        w.extension_super(em, node);
        let gm = w.t.obj_with(
            "GEditableMesh2",
            &[("nextPointUid", n.to_string()), ("useDelaunayTriangulation", "false".into())],
        );
        w.t.floats(gm, "point", px);
        w.t.ints(gm, "byte-array", "pointPriority", &vec![10; n]);
        w.t.ints(gm, "short-array", "edge", &edge_vals);
        w.t.ints(gm, "byte-array", "edgePriority", &vec![30; edges.len()]);
        w.t.ints(gm, "int-array", "pointUid", &(0..n as i64).collect::<Vec<_>>());
        let u = w.ids.uuid();
        let mg = w.t.obj_with("GEditableMeshGuid", &[("uuid", u), ("note", NOTE_NONE.into())]);
        w.t.set(gm, "meshGuid", mg);
        w.t.set(gm, "coordType", w.coord_basic);
        w.t.set(em, "editableMesh", gm);
        w.t.b(em, "isLocked", false);
        let meshgen = w.t.obj("CMeshGeneratorExtension");
        w.extension_super(meshgen, node);
        let gs = w.t.obj("MeshGenerateSetting");
        for (k, v) in [
            ("polygonOuterDensity", 50),
            ("polygonInnerDensity", 50),
            ("polygonMargin", 14),
            ("polygonInnerMargin", 14),
            ("polygonMinMargin", 5),
            ("polygonMinBoundsPt", 10),
            ("thresholdAlpha", 0),
        ] {
            w.t.i(gs, k, v);
        }
        w.t.set(meshgen, "meshGenerateSetting", gs);
        // the mesh source itself
        let space = space_under(m.parent_deformer, &spaces);
        let coord = match space {
            Space::Root => w.coord_basic,
            _ => w.coord_local,
        };
        let axes: Vec<(usize, Vec<f32>)> =
            m.binding.axes.iter().map(|a| (a.parameter as usize, a.keys.clone())).collect();
        let per = n * 2;
        let slots = m.positions.len().checked_div(per).unwrap_or(0).max(1);
        let (grid, form_guids) = w.grid(&axes, &param_guids, slots);
        let ctrl = w.controllable(
            &name,
            m.visible,
            Some(part_guid(m.parent_part)),
            grid,
            &[em, tie, meshgen],
        );
        let dsrc = w.t.obj("ACDrawableSource");
        w.t.set(dsrc, "super", ctrl);
        let did = w.t.obj_with("CDrawableId", &[("idstr", m.id.clone())]);
        w.t.set(dsrc, "id", did);
        if let Some(&g) = mesh_guids.get(k) {
            w.t.set(dsrc, "guid", g);
        }
        let target = m
            .parent_deformer
            .and_then(|p| deformer_guids.get(p as usize).copied())
            .unwrap_or(w.root_deformer);
        w.t.set(dsrc, "targetDeformerGuid", target);
        let clips = w.t.list(dsrc, "carray_list", "clipGuidList");
        for &other in &m.masks {
            if let Some(&g) = mesh_guids.get(other as usize) {
                w.t.push(clips, g);
            }
        }
        w.t.b(dsrc, "invertClippingMask", m.flags & flags::INVERTED_MASK != 0);
        w.t.set(dsrc, "icon32", icon32);
        w.t.set(dsrc, "icon16", icon16);
        w.t.set(node, "super", dsrc);
        let idx: Vec<i64> = m.indices.iter().map(|&i| i64::from(i)).collect();
        w.t.ints(node, "int-array", "indices", &idx);
        let forms = w.t.list(node, "carray_list", "keyforms");
        for (s, &g) in form_guids.iter().enumerate() {
            let form = w.t.obj("CArtMeshForm");
            let df = w.t.obj("ACDrawableForm");
            let acf = w.ac_form(g, node);
            w.t.set(df, "super", acf);
            let order = m.draw_orders.get(s).copied().unwrap_or(500.0);
            w.t.i(df, "drawOrder", order.round() as i64);
            w.t.f(df, "opacity", m.opacities.get(s).copied().unwrap_or(1.0));
            w.colors(df, coord);
            w.t.set(form, "super", df);
            let pts = m.positions.get(s * per..(s + 1) * per).unwrap_or(&[]);
            let pts = convert(space, pts);
            w.t.floats(form, "positions", &pts);
            w.t.push(forms, form);
        }
        w.t.floats(node, "positions", px);
        let uvs: Vec<f32> = px
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| {
                let mx = p[0] - mat.left as f32;
                let my = p[1] - mat.top as f32;
                [(pl.ax + mx * pl.sx) / page, (pl.ay + my * pl.sy) / page]
            })
            .collect();
        w.t.floats(node, "uvs", &uvs);
        if let Some(&tex) = textures.get(pl.page) {
            w.t.set(node, "texture", tex);
        }
        let blend = if m.flags & flags::BLEND_ADDITIVE != 0 {
            "ADD"
        } else if m.flags & flags::BLEND_MULTIPLICATIVE != 0 {
            "MULTIPLY"
        } else {
            "NORMAL"
        };
        w.t.enum_v(node, "ColorComposition", "colorComposition", blend);
        w.t.b(node, "culling", m.flags & flags::DOUBLE_SIDED == 0);
        w.t.enum_v(node, "TextureState", "textureState", "TEXTURE_ATLAS");
        w.t.s(node, "userData", "");
    }

    // 4g. parts, with their children in palette order: higher model
    //     indices draw in front, so they come first
    let np = model.parts.len();
    let mut max_mesh: Vec<i64> = vec![-1; np + 1];
    let slot_of = |p: Option<u32>| p.map_or(np, |i| i as usize);
    for (k, m) in model.art_meshes.iter().enumerate() {
        let mut at = m.parent_part;
        for _ in 0..64 {
            let s = slot_of(at);
            if let Some(mx) = max_mesh.get_mut(s) {
                *mx = (*mx).max(k as i64);
            }
            match at.and_then(|i| model.parts.get(i as usize)) {
                Some(p) => at = p.parent_part,
                None => break,
            }
        }
    }
    for (pi, node) in part_nodes.iter().copied().enumerate().chain(std::iter::once((np, root_part)))
    {
        let is_root = pi == np;
        let parent_of_this =
            if is_root { None } else { model.parts.get(pi).and_then(|p| p.parent_part) };
        let mut kids: Vec<(i64, usize)> = Vec::new();
        let mut defs: Vec<usize> = Vec::new();
        for (ci, p) in model.parts.iter().enumerate() {
            if slot_of(p.parent_part) == pi
                && let Some(&g) = part_guids.get(ci)
            {
                kids.push((max_mesh.get(ci).copied().unwrap_or(-1), g));
            }
        }
        for (k, m) in model.art_meshes.iter().enumerate() {
            if slot_of(m.parent_part) == pi
                && let Some(&g) = mesh_guids.get(k)
            {
                kids.push((k as i64, g));
            }
        }
        for (di, d) in model.deformers.iter().enumerate() {
            if slot_of(d.parent_part) == pi
                && let Some(&g) = deformer_guids.get(di)
            {
                defs.push(g);
            }
        }
        kids.sort_by_key(|k| std::cmp::Reverse(k.0));
        let (id, name, visible, binding, orders) = match model.parts.get(pi) {
            Some(p) if !is_root => (
                p.id.clone(),
                name_or(x.part_names, pi, &p.id),
                p.visible,
                p.binding.clone(),
                p.draw_orders.clone(),
            ),
            _ => (
                "__RootPart__".to_string(),
                "Root Part".to_string(),
                true,
                Default::default(),
                vec![500.0],
            ),
        };
        let axes: Vec<(usize, Vec<f32>)> =
            binding.axes.iter().map(|a| (a.parameter as usize, a.keys.clone())).collect();
        let (grid, form_guids) = w.grid(&axes, &param_guids, orders.len().max(1));
        let parent = if is_root { None } else { Some(part_guid(parent_of_this)) };
        let ctrl = w.controllable(&name, visible, parent, grid, &[]);
        w.t.set(node, "super", ctrl);
        let g = if is_root {
            root_part_guid
        } else {
            part_guids.get(pi).copied().unwrap_or(root_part_guid)
        };
        w.t.set(node, "guid", g);
        let pid = w.t.obj_with("CPartId", &[("idstr", id)]);
        w.t.set(node, "id", pid);
        w.t.b(node, "enableDrawOrderGroup", false);
        w.t.i(node, "defaultOrder_forEditor", 500);
        w.t.b(node, "isSketch", false);
        let col = w.t.obj("CColor");
        w.t.set(node, "partsEditColor", col);
        let list = w.t.list(node, "carray_list", "_childGuids");
        for d in defs {
            w.t.push(list, d);
        }
        for (_, g) in kids {
            w.t.push(list, g);
        }
        w.t.set(node, "targetDeformerGuid", w.zero_deformer);
        let forms = w.t.list(node, "carray_list", "keyforms");
        for (s, &fg) in form_guids.iter().enumerate() {
            let form = w.t.obj("CPartForm");
            let acf = w.ac_form(fg, node);
            w.t.set(form, "super", acf);
            w.t.i(form, "drawOrder", orders.get(s).copied().unwrap_or(500.0).round() as i64);
            w.t.push(forms, form);
        }
    }

    // 4h. the layered image and its root group
    {
        let W { t, layered, layer_root, layered_guid, blend, opts, ids, .. } = &mut w;
        let (layered, layer_root) = (*layered, *layer_root);
        let doc_name = format!("{}.psd", x.name);
        t.s(layered, "name", &doc_name);
        t.s(layered, "memo", "");
        t.i(layered, "width", i64::from(cw));
        t.i(layered, "height", i64::from(ch));
        t.leaf(layered, "file", "psdFile", doc_name);
        t.s(layered, "description", "");
        t.set(layered, "guid", *layered_guid);
        t.null(layered, "psdBytes");
        t.l(layered, "psdFileLastModified", TIME_MS);
        t.set(layered, "_rootLayer", layer_root);
        let ls = t.obj("LayerSet");
        t.set(ls, "_layeredImage", layered);
        t.set(ls, "_layerEntryList", layer_entries);
        t.set(layered, "layerSet", ls);
        t.null(layered, "icon16");
        t.null(layered, "icon64");
        let lg = t.obj("ACLayerGroup");
        let entry = t.obj("ACLayerEntry");
        t.s(entry, "name", "root");
        t.s(entry, "memo", "");
        t.b(entry, "isVisible", true);
        t.b(entry, "isClipping", false);
        t.set(entry, "blend", *blend);
        let u = ids.uuid();
        let g = t.obj_with("CLayerGuid", &[("uuid", u), ("note", NOTE_NONE.into())]);
        t.set(entry, "guid", g);
        t.null(entry, "group");
        t.i(entry, "opacity255", 255);
        t.set(entry, "_optionOfIOption", *opts);
        t.set(entry, "_layeredImage", layered);
        t.set(lg, "super", entry);
        t.set(lg, "_children", layer_children);
        t.set(layer_root, "super", lg);
        t.null(layer_root, "layerIdentifier");
        // the root group heads the entry list
        if let Some(n) = t.nodes.get_mut(layer_entries) {
            n.kids.insert(0, (None, layer_root));
        }
    }
    {
        let group = w.group;
        w.t.s(group, "memo", "");
        w.t.s(group, "groupName", x.name);
        let linked = w.t.list(group, "carray_list", "_linkedRawImageGuids");
        w.t.push(linked, w.layered_guid);
        w.t.set(group, "_modelImages", model_images);
    }

    // 4i. the model source
    let ms = w.t.obj_with("CModelSource", &[("isDefaultKeyformLocked", "false".into())]);
    let u = w.ids.uuid();
    let mg = w.t.obj_with("CModelGuid", &[("uuid", u), ("note", NOTE_NONE.into())]);
    w.t.set(ms, "guid", mg);
    w.t.s(ms, "name", x.name);
    let ed = w.t.obj("EditorEdition");
    w.t.i(ed, "edition", 15);
    w.t.set(ms, "editorEdition", ed);
    let canvas = w.t.obj("CImageCanvas");
    w.t.i(canvas, "pixelWidth", i64::from(cw));
    w.t.i(canvas, "pixelHeight", i64::from(ch));
    let bg = w.t.obj("CColor");
    w.t.set(canvas, "background", bg);
    w.t.set(ms, "canvas", canvas);
    // parameters
    let pset = w.t.obj("CParameterSourceSet");
    let sources = w.t.list(pset, "carray_list", "_sources");
    for (i, p) in model.parameters.iter().enumerate() {
        let ps = w.t.obj("CParameterSource");
        let dec = p.decimals.clamp(0, 10);
        w.t.i(ps, "decimalPlaces", i64::from(dec));
        if let Some(&g) = param_guids.get(i) {
            w.t.set(ps, "guid", g);
        }
        w.t.f(ps, "snapEpsilon", 0.1f32.powi(dec));
        w.t.f(ps, "minValue", p.min);
        w.t.f(ps, "maxValue", p.max);
        w.t.f(ps, "defaultValue", p.default);
        w.t.b(ps, "isRepeat", p.repeat);
        let pid = w.t.obj_with("CParameterId", &[("idstr", p.id.clone())]);
        w.t.set(ps, "id", pid);
        w.t.enum_v(ps, "Type", "paramType", "NORMAL");
        w.t.s(ps, "name", &name_or(x.param_names, i, &p.id));
        w.t.s(ps, "description", "");
        w.t.b(ps, "combined", false);
        let fg = folder_of
            .get(i)
            .copied()
            .flatten()
            .and_then(|f| folder_guids.get(f).copied())
            .unwrap_or(root_group_guid);
        w.t.set(ps, "parentGroupGuid", fg);
        w.t.push(sources, ps);
    }
    w.t.set(ms, "parameterSourceSet", pset);
    // textures
    let tm = w.t.obj("CTextureManager");
    let tig = w.t.obj("TextureImageGroup");
    w.t.list(tig, "carray_list", "children");
    w.t.set(tm, "textureList", tig);
    let raw = w.t.list(tm, "carray_list", "_rawImages");
    let wrap = w.t.obj("LayeredImageWrapper");
    w.t.set(wrap, "image", w.layered);
    w.t.l(wrap, "importedTimeMSec", TIME_MS);
    w.t.l(wrap, "lastModifiedTimeMSec", TIME_MS);
    w.t.b(wrap, "isReplaced", false);
    w.t.push(raw, wrap);
    let groups = w.t.list(tm, "carray_list", "_modelImageGroups");
    w.t.push(groups, w.group);
    let atl = w.t.list(tm, "carray_list", "_textureAtlases");
    for &a in &atlas_nodes {
        w.t.push(atl, a);
    }
    w.t.b(tm, "isTextureInputModelImageMode", false);
    w.t.i(tm, "previewReductionRatio", 1);
    w.t.list(tm, "carray_list", "artPathBrushUsingLayeredImageIds");
    w.t.set(ms, "textureManager", tm);
    w.t.b(ms, "useLegacyDrawOrder__testImpl", false);
    // drawables, deformers, affecters, parts
    let ds = w.t.obj("CDrawableSourceSet");
    let l = w.t.list(ds, "carray_list", "_sources");
    for &n in &mesh_nodes {
        w.t.push(l, n);
    }
    w.t.set(ms, "drawableSourceSet", ds);
    let dfs = w.t.obj("CDeformerSourceSet");
    let l = w.t.list(dfs, "carray_list", "_sources");
    for &n in &deformer_nodes {
        w.t.push(l, n);
    }
    w.t.set(ms, "deformerSourceSet", dfs);
    let afs = w.t.obj("CAffecterSourceSet");
    w.t.list(afs, "carray_list", "_sources");
    w.t.set(ms, "affecterSourceSet", afs);
    let pss = w.t.obj("CPartSourceSet");
    let l = w.t.list(pss, "carray_list", "_sources");
    w.t.push(l, root_part);
    for &n in &part_nodes {
        w.t.push(l, n);
    }
    w.t.set(ms, "partSourceSet", pss);
    // physics
    let phys = w.t.obj("CPhysicsSettingsSourceSet");
    let list = w.t.list(phys, "carray_list", "_sourceCubismPhysics");
    let mut first_phys = None;
    let mut fps = 60.0;
    if let Some(rig) = &model.physics {
        if rig.fps > 0.0 {
            fps = rig.fps;
        }
        let kind = |k: PhysicsIo| match k {
            PhysicsIo::X => "SRC_TO_X",
            PhysicsIo::Y => "SRC_TO_Y",
            PhysicsIo::Angle => "SRC_TO_G_ANGLE",
        };
        for (gi, g) in rig.groups.iter().enumerate() {
            let src = w.t.obj("CPhysicsSettingsSource");
            w.t.s(src, "name", if g.name.is_empty() { &g.id } else { &g.name });
            let pg = w.guid("CPhysicsSettingsGuid", NOTE_NONE);
            w.t.set(src, "guid", pg);
            first_phys.get_or_insert(pg);
            let id =
                if g.id.is_empty() { format!("PhysicsSetting{}", gi + 1) } else { g.id.clone() };
            let pid = w.t.obj_with("CPhysicsSettingId", &[("idstr", id)]);
            w.t.set(src, "id", pid);
            let ins = w.t.list(src, "carray_list", "inputs");
            for i in &g.inputs {
                let Some(&pg) = param_index.get(i.param.as_str()).and_then(|&p| param_guids.get(p))
                else {
                    log::warn!("cmo3: physics input {} names no parameter; left out", i.param);
                    continue;
                };
                let n = w.t.obj("CPhysicsInput");
                let u = w.ids.uuid();
                let dg =
                    w.t.obj_with("CPhysicsDataGuid", &[("uuid", u), ("note", NOTE_NONE.into())]);
                w.t.set(n, "guid", dg);
                w.t.set(n, "source", pg);
                w.t.f(n, "angleScale", 0.0);
                let v = w.t.obj("GVector2");
                w.t.f(v, "x", 0.0);
                w.t.f(v, "y", 0.0);
                w.t.set(n, "translationScale", v);
                w.t.f(n, "weight", i.weight);
                w.t.enum_v(n, "CPhysicsSourceType", "type", kind(i.kind));
                w.t.b(n, "isReverse", i.reflect);
                w.t.push(ins, n);
            }
            let outs = w.t.list(src, "carray_list", "outputs");
            for o in &g.outputs {
                let Some(&pg) = param_index.get(o.param.as_str()).and_then(|&p| param_guids.get(p))
                else {
                    log::warn!("cmo3: physics output {} names no parameter; left out", o.param);
                    continue;
                };
                let n = w.t.obj("CPhysicsOutput");
                let u = w.ids.uuid();
                let dg =
                    w.t.obj_with("CPhysicsDataGuid", &[("uuid", u), ("note", NOTE_NONE.into())]);
                w.t.set(n, "guid", dg);
                w.t.set(n, "destination", pg);
                w.t.i(n, "vertexIndex", i64::from(o.vertex));
                let v = w.t.obj("GVector2");
                let (tx, ty, ang) = match o.kind {
                    PhysicsIo::X => (o.scale, 0.0, 0.0),
                    PhysicsIo::Y => (0.0, o.scale, 0.0),
                    PhysicsIo::Angle => (0.0, 0.0, o.scale),
                };
                w.t.f(v, "x", tx);
                w.t.f(v, "y", ty);
                w.t.set(n, "translationScale", v);
                w.t.f(n, "angleScale", ang);
                w.t.f(n, "weight", o.weight);
                w.t.enum_v(n, "CPhysicsSourceType", "type", kind(o.kind));
                w.t.b(n, "isReverse", o.reflect);
                w.t.push(outs, n);
            }
            let verts = w.t.list(src, "carray_list", "vertices");
            for node in &g.nodes {
                let n = w.t.obj("CPhysicsVertex");
                let u = w.ids.uuid();
                let dg =
                    w.t.obj_with("CPhysicsDataGuid", &[("uuid", u), ("note", NOTE_NONE.into())]);
                w.t.set(n, "guid", dg);
                let v = w.t.obj("GVector2");
                w.t.f(v, "x", node.x);
                w.t.f(v, "y", node.y);
                w.t.set(n, "position", v);
                w.t.f(n, "mobility", node.mobility);
                w.t.f(n, "delay", node.delay);
                w.t.f(n, "acceleration", node.acceleration);
                w.t.f(n, "radius", node.radius);
                w.t.push(verts, n);
            }
            w.t.f(src, "normalizedPositionValueMax", g.norm_position.max);
            w.t.f(src, "normalizedPositionValueMin", g.norm_position.min);
            w.t.f(src, "normalizedPositionDefaultValue", g.norm_position.default);
            w.t.f(src, "normalizedAngleValueMax", g.norm_angle.max);
            w.t.f(src, "normalizedAngleValueMin", g.norm_angle.min);
            w.t.f(src, "normalizedAngleDefaultValue", g.norm_angle.default);
            w.t.push(list, src);
        }
    }
    match first_phys {
        Some(g) => w.t.set(phys, "selectedCubismPhysics", g),
        None => w.t.null(phys, "selectedCubismPhysics"),
    }
    w.t.i(phys, "settingFPS", fps.round() as i64);
    w.t.set(ms, "physicsSettingsSourceSet", phys);
    w.t.set(ms, "rootPart", root_part);
    // parameter folders
    let pgs = w.t.obj("CParameterGroupSet");
    let gl = w.t.list(pgs, "carray_list", "_groups");
    let root_group = w.t.obj("CParameterGroup");
    w.t.share(root_group);
    w.t.s(root_group, "name", "Root Parameter Group");
    w.t.s(root_group, "description", "");
    w.t.b(root_group, "folderIsOpened", false);
    w.t.set(root_group, "guid", root_group_guid);
    w.t.null(root_group, "parentGroupGuid");
    let rk = w.t.list(root_group, "carray_list", "_childGuids");
    for &g in &folder_guids {
        w.t.push(rk, g);
    }
    for (i, f) in folder_of.iter().enumerate() {
        if f.is_none()
            && let Some(&g) = param_guids.get(i)
        {
            w.t.push(rk, g);
        }
    }
    let rid = w.t.obj_with("CParameterGroupId", &[("idstr", "ParamGroupRoot".into())]);
    w.t.set(root_group, "id", rid);
    w.label(root_group);
    w.t.push(gl, root_group);
    for (fi, (fname, members)) in x.param_groups.iter().enumerate() {
        let g = w.t.obj("CParameterGroup");
        w.t.s(g, "name", fname);
        w.t.s(g, "description", "");
        w.t.b(g, "folderIsOpened", true);
        if let Some(&fg) = folder_guids.get(fi) {
            w.t.set(g, "guid", fg);
        }
        w.t.set(g, "parentGroupGuid", root_group_guid);
        let kids = w.t.list(g, "carray_list", "_childGuids");
        for &m in members {
            if let Some(&pg) = param_guids.get(m as usize) {
                w.t.push(kids, pg);
            }
        }
        let gid = w.t.obj_with("CParameterGroupId", &[("idstr", format!("ParamGroup{}", fi + 1))]);
        w.t.set(g, "id", gid);
        w.label(g);
        w.t.push(gl, g);
    }
    w.t.set(ms, "parameterGroupSet", pgs);
    w.t.set(ms, "rootParameterGroup", root_group);
    // model info: lip sync and blink from the standard ids
    let info = w.t.obj("CModelInfo");
    w.t.f(info, "pixelsPerUnit", 1.0);
    let op = w.t.obj("CPoint");
    w.t.i(op, "x", 0);
    w.t.i(op, "y", 0);
    w.t.set(info, "originInPixels", op);
    let eff = w.t.obj("CEffectParameterGroups");
    let em = w.t.list(eff, "hash_map", "_parameterGroups");
    for (key, ids_) in [
        ("LIPSYNC", &["ParamMouthOpenY"][..]),
        ("EYE_BLINK", &["ParamEyeLOpen", "ParamEyeROpen"][..]),
    ] {
        let e = w.t.obj("entry");
        w.t.enum_v(e, "EffectParameterType", "key", key);
        let v = w.t.list(e, "carray_list", "value");
        for id in ids_ {
            if let Some(&g) = param_index.get(id).and_then(|&p| param_guids.get(p)) {
                w.t.push(v, g);
            }
        }
        w.t.push(em, e);
    }
    w.t.set(info, "_effectParameterGroups", eff);
    w.t.set(ms, "modelInfo", info);
    // export settings, as the Editor's dialog remembers them
    let opts = w.t.obj_with("hash_map", &[("keyType", "string".into())]);
    let ex = w.t.obj("CModelExportSettingDialogData");
    w.t.enum_v(ex, "FormatVersion", "version", "VERSION_50");
    for (k, v) in [
        ("exportInvisiblePart", false),
        ("exportSketchPart", false),
        ("exportInvisibleArtMesh", false),
        ("exportPhysicsJson", true),
        ("exportUserDataJson", true),
        ("exportDisplayInfoJson", true),
        ("exportTrackCursorJson", true),
    ] {
        w.t.b(ex, k, v);
    }
    w.t.f(ex, "normalOriginX", (c.origin_x / cw as f32).clamp(0.0, 1.0));
    w.t.f(ex, "normalOriginY", (c.origin_y / ch as f32).clamp(0.0, 1.0));
    w.t.i(ex, "exportTextureScale", 1);
    w.t.f(ex, "pixelsPerUnit", ppu);
    w.t.enum_v(ex, "CMocExportType", "exportType", "SDK");
    w.t.i(ex, "exportNizimaPreviewTextureScale", 2);
    w.t.b(ex, "enableExtendEdgeColor", true);
    w.t.b(ex, "exportMotionSyncJson", false);
    w.t.set(opts, "MODEL_EXPORT_SETTING_KEY", ex);
    w.t.set(ms, "modelOptions", opts);
    // icons of the model: blank
    for (side, field) in [(64, "_icon64"), (32, "_icon32"), (16, "_icon16")] {
        let blank = png(&image::RgbaImage::new(side, side))?;
        let path = w.files.icon(blank);
        let icon = w.icon(side, &path);
        w.t.set(ms, field, icon);
    }
    let gms = w.t.obj("CGameMotionSet");
    w.t.list(gms, "carray_list", "gameMotions");
    w.t.list(gms, "carray_list", "gameMotionGroups");
    w.t.set(ms, "gameMotionSet", gms);
    let viewer = w.t.obj("ModelViewerSetting");
    let tcs = w.t.list(viewer, "array_list", "trackCursorSettings");
    for (i, p) in model.parameters.iter().enumerate() {
        let Some(&g) = param_guids.get(i) else { continue };
        let tc = w.t.obj("TrackCursorSetting");
        w.t.set(tc, "guid", g);
        let v = match p.id.as_str() {
            "ParamAngleX" => "X",
            "ParamAngleY" => "Y",
            _ => "NONE",
        };
        w.t.enum_v(tc, "TrackCursorSettingItemType", "value", v);
        w.t.f(tc, "weight", 1.0);
        w.t.push(tcs, tc);
    }
    w.t.set(ms, "modelViewerSetting", viewer);
    let guides = w.t.obj("CGuidesSetting");
    w.t.list(guides, "carray_list", "guidesModeling");
    w.t.set(ms, "guides", guides);
    w.t.i(ms, "targetVersionNo", 5_000_000);
    w.t.i(ms, "latestVersionOfLastModelerNo", 5_000_000);
    let brushes = w.t.obj("CArtPathBrushSetting");
    w.t.list(brushes, "carray_list", "brushes");
    w.t.set(ms, "artPathBrushesSetting", brushes);
    let rp = w.t.obj("CRandomPoseSettingManager");
    w.t.list(rp, "array_list", "_settings");
    w.t.i(rp, "currentIndex", 0);
    w.t.set(ms, "randomPoseSetting", rp);
    let msync = w.t.obj("CMotionSyncSettingSourceSet");
    w.t.list(msync, "linked_set", "_settingSourceSetMotionSync");
    w.t.set(ms, "motionSyncSettingsSet", msync);

    // 5. the archive
    let xml = w.t.emit(HEAD, ms);
    let W { mut files, .. } = w;
    // nothing leaves that the strict reading refuses
    let doc = super::xml::Doc::parse(xml.as_bytes())?;
    let names: Vec<&str> = files.list.iter().map(|f| f.name.as_str()).collect();
    let problems = super::check::check(&doc, &super::check::Schema::builtin(), &names);
    if !problems.is_empty() {
        let shown: Vec<&str> = problems.iter().take(8).map(String::as_str).collect();
        return Err(err(format!(
            "the written project fails the strict check: {}",
            shown.join("; ")
        )));
    }
    files.list.push(caffw::File {
        name: "main.xml".into(),
        bytes: caffw::zip_one(xml.as_bytes())?,
        main: true,
    });
    let key = (seed >> 16) as u32 | 0x0101_0101;
    caffw::write(&files.list, key)
}
