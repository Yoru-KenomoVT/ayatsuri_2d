// What the importer pulls out of a project's object graph: each art mesh
// at rest with its layer image and keyforms, each deformer with its
// keyforms, the parameters and their folders, and the physics groups. The
// shapes below mirror main.xml's objects; the conversion into a model
// happens in build.rs.
//
// Cubism keeps a form in its parent's space: canvas pixels at the top
// level, 0..1 across the grid under a warp, pixel offsets under a
// rotation. Nothing here converts; it only reads.

use std::collections::HashMap;

use super::caff::Caff;
use super::xml::Doc;
use crate::ImportError;
use ayatsuri_format::model::flags;

fn err(msg: impl Into<String>) -> ImportError {
    ImportError::Export(format!("cmo3: {}", msg.into()))
}

pub struct Binding {
    pub param: String,
    pub keys: Vec<f32>,
}

/// One keyform: where it sits on the binding grid, and what it holds.
pub struct Form {
    pub coords: Vec<usize>,
    pub opacity: f32,
    pub draw_order: f32,
    /// Warp grid or mesh points, in the parent's space.
    pub positions: Vec<f32>,
    /// Rotation deformers only.
    pub angle: f32,
    pub origin: [f32; 2],
    pub scale: f32,
    pub reflect: [bool; 2],
}

pub struct Mesh {
    pub id: String,
    pub name: String,
    pub parts: Vec<String>,
    /// The deformer it hangs from, by uuid.
    pub deformer: Option<String>,
    /// The layer's place on the canvas: x, y, width, height.
    pub rect: [i32; 4],
    pub png: Vec<u8>,
    /// Resting points in canvas pixels, and the triangles over them.
    pub points: Vec<f32>,
    pub triangles: Vec<u16>,
    /// Meshes that clip this one, as indices into the sorted mesh list.
    pub clip: Vec<usize>,
    pub bindings: Vec<Binding>,
    pub forms: Vec<Form>,
    /// Blend, double sided and inverted mask, as the model's mesh flags.
    pub flags: u8,
}

pub enum DeformerKind {
    Warp { cols: u32, rows: u32 },
    Rotation,
}

pub struct Deformer {
    pub uuid: String,
    pub id: String,
    pub parent: Option<String>,
    pub kind: DeformerKind,
    pub bindings: Vec<Binding>,
    pub forms: Vec<Form>,
}

pub struct Param {
    pub id: String,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub repeat: bool,
    pub decimals: i32,
    pub folder: String,
    pub folder_rank: i64,
}

pub struct PhysIo {
    pub param: String,
    pub weight: f32,
    pub kind: ayatsuri_format::model::PhysicsIo,
    pub reflect: bool,
    /// Outputs: the pendulum node, and the scale for the kind.
    pub vertex: u32,
    pub scale: f32,
}

pub struct PhysGroup {
    pub id: String,
    pub name: String,
    pub inputs: Vec<PhysIo>,
    pub outputs: Vec<PhysIo>,
    pub nodes: Vec<ayatsuri_format::model::PhysicsNode>,
    pub position: [f32; 3],
    pub angle: [f32; 3],
}

pub struct Extract {
    pub canvas: (u32, u32),
    /// Bottom of the palette first.
    pub meshes: Vec<Mesh>,
    /// Parents before children.
    pub deformers: Vec<Deformer>,
    pub params: Vec<Param>,
    pub physics_fps: f32,
    pub physics: Vec<PhysGroup>,
}

/// Depth-first descendants of a node, the node included.
fn descendants(doc: &Doc, root: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        out.push(i);
        if out.len() > 4_000_000 {
            break;
        }
        if let Some(n) = doc.node(i) {
            stack.extend(n.children.iter().rev().copied());
        }
    }
    out
}

/// The ACForm block a keyform's guid sits in, down the `super` chain.
fn ac_form(doc: &Doc, form: usize) -> Option<usize> {
    let mut at = form;
    for _ in 0..16 {
        if doc.node(at)?.tag == "ACForm" {
            return Some(at);
        }
        at = doc.field_raw(at, "super")?;
    }
    None
}

/// The uuid or ref name of a guid element, whichever it carries.
fn guid_key(doc: &Doc, g: usize) -> Option<String> {
    doc.uuid_of(g).or_else(|| doc.attr(g, "xs.ref").map(str::to_string))
}

struct Ctx<'a> {
    doc: &'a Doc,
    forms_by_guid: HashMap<String, usize>,
    param_of_uuid: HashMap<String, String>,
    parts: HashMap<String, (String, Option<String>)>,
}

impl Ctx<'_> {
    /// The keyforms an object owns, by guid key. Copies made in Cubism keep
    /// their original's form guids, so a lookup across the whole file would
    /// hand a copy its original's forms; own forms come first.
    fn own_forms(&self, src: usize) -> HashMap<String, usize> {
        let doc = self.doc;
        let mut out = HashMap::new();
        let Some(list) = doc.field_raw(src, "keyforms") else { return out };
        for f in doc.items(list) {
            let Some(acf) = ac_form(doc, f) else { continue };
            let Some(g) = doc.field_raw(acf, "guid") else { continue };
            for key in
                [doc.attr(g, "xs.ref").map(str::to_string), doc.uuid_of(g)].into_iter().flatten()
            {
                out.entry(key).or_insert(f);
            }
        }
        out
    }

    /// (bindings, forms as (grid coords, form node)) for a controllable.
    fn keyforms(&self, ctrl: usize, src: usize) -> (Vec<Binding>, Vec<(Vec<usize>, usize)>) {
        let doc = self.doc;
        let Some(grid) = doc.field(ctrl, "keyformGridSource") else {
            return (Vec::new(), Vec::new());
        };
        let mut bindings = Vec::new();
        let mut slot: HashMap<String, usize> = HashMap::new();
        if let Some(blist) = doc.field_raw(grid, "keyformBindings") {
            for (i, braw) in doc.items_raw(blist).iter().copied().enumerate() {
                let key = doc.ref_key(braw);
                let Some(b) = doc.deref(braw) else { continue };
                let keys: Vec<f32> = doc
                    .field_raw(b, "keys")
                    .map(|k| {
                        doc.items(k)
                            .iter()
                            .filter_map(|&v| doc.text(v).trim().parse().ok())
                            .collect()
                    })
                    .unwrap_or_default();
                let param = doc
                    .uuid_field(b, "parameterGuid")
                    .and_then(|u| self.param_of_uuid.get(&u).cloned())
                    .unwrap_or_default();
                bindings.push(Binding { param, keys });
                if let Some(k) = key {
                    slot.insert(k, i);
                }
            }
        }
        let own = self.own_forms(src);
        let mut out = Vec::new();
        let Some(kogs) = doc.field_raw(grid, "keyformsOnGrid") else { return (bindings, out) };
        for kog in doc.items(kogs) {
            let mut coords = vec![0usize; bindings.len()];
            if let Some(ak) = doc.field_raw(kog, "accessKey")
                && let Some(list) = doc.field_raw(ak, "_keyOnParameterList")
            {
                for kop in doc.items(list) {
                    let Some(bref) = doc.field_raw(kop, "binding") else { continue };
                    let Some(at) = doc.ref_key(bref).and_then(|k| slot.get(&k).copied()) else {
                        continue;
                    };
                    let idx = doc.int_field(kop, "keyIndex").unwrap_or(0).max(0) as usize;
                    if let Some(c) = coords.get_mut(at) {
                        *c = idx;
                    }
                }
            }
            let Some(fg) = doc.field_raw(kog, "keyformGuid") else { continue };
            let form = doc
                .attr(fg, "xs.ref")
                .and_then(|r| own.get(r).copied())
                .or_else(|| doc.uuid_of(fg).and_then(|u| own.get(&u).copied()))
                .or_else(|| guid_key(doc, fg).and_then(|k| self.forms_by_guid.get(&k).copied()));
            if let Some(f) = form {
                out.push((coords, f));
            }
        }
        (bindings, out)
    }

    /// Opacity and draw order from the form's ACDrawableForm / ACDeformerForm block.
    fn form_common(&self, form: usize) -> (f32, f32) {
        let doc = self.doc;
        let sup = doc.field_raw(form, "super");
        let op = sup.and_then(|s| doc.float_field(s, "opacity")).unwrap_or(1.0);
        let dr = sup.and_then(|s| doc.float_field(s, "drawOrder")).unwrap_or(500.0);
        (op, dr)
    }

    fn part_path(&self, mut gid: Option<String>) -> Vec<String> {
        let mut out = Vec::new();
        let mut hops = 0;
        while let Some(g) = gid {
            hops += 1;
            if hops > 64 {
                break;
            }
            let Some((name, parent)) = self.parts.get(&g) else { break };
            out.push(name.clone());
            gid = parent.clone();
        }
        out.reverse();
        // the root part is Cubism's, not the artist's
        out.retain(|p| p != "Root Part");
        out
    }
}

/// Read the whole project.
pub fn extract(caff: &Caff, doc: &Doc) -> Result<Extract, ImportError> {
    // every keyform by its guid, for the keyform grids
    let mut forms_by_guid = HashMap::new();
    for tag in ["CArtMeshForm", "CWarpDeformerForm", "CRotationDeformerForm"] {
        for &f in doc.all(tag) {
            if let Some(acf) = ac_form(doc, f)
                && let Some(g) = doc.field_raw(acf, "guid")
                && let Some(k) = guid_key(doc, g)
            {
                forms_by_guid.entry(k).or_insert(f);
            }
        }
    }
    // parameters by guid; some guids carry the id as their note
    let mut param_of_uuid: HashMap<String, String> = HashMap::new();
    for &el in doc.all("CParameterSource") {
        if let (Some(u), Some(id)) =
            (doc.uuid_field(el, "guid"), doc.field(el, "id").and_then(|i| doc.attr(i, "idstr")))
        {
            param_of_uuid.insert(u, id.to_string());
        }
    }
    let known: std::collections::HashSet<String> = param_of_uuid.values().cloned().collect();
    for &g in doc.all("CParameterGuid") {
        if let (Some(u), Some(note)) = (doc.attr(g, "uuid"), doc.attr(g, "note"))
            && known.contains(note)
        {
            param_of_uuid.entry(u.to_string()).or_insert_with(|| note.to_string());
        }
    }
    // parts: guid ref -> (name, parent guid ref)
    let mut parts = HashMap::new();
    for &el in doc.all("CPartSource") {
        if doc.attr(el, "xs.id").is_none() {
            continue;
        }
        let Some(gid) = doc.field_raw(el, "guid").and_then(|g| doc.attr(g, "xs.ref")) else {
            continue;
        };
        let name =
            doc.walk_super(el, "localName").map(|n| doc.text(n).to_string()).unwrap_or_default();
        let mut parent = None;
        let mut at = Some(el);
        for _ in 0..16 {
            let Some(p) = at else { break };
            if let Some(c) = doc.field_raw(p, "parentGuid") {
                parent = doc.attr(c, "xs.ref").map(str::to_string);
                break;
            }
            at = doc.field_raw(p, "super");
        }
        parts.insert(gid.to_string(), (name, parent));
    }
    let cx = Ctx { doc, forms_by_guid, param_of_uuid, parts };

    // model images: guid -> the layer element they read
    let mut image_layer: HashMap<String, usize> = HashMap::new();
    for &el in doc.all("CModelImage") {
        let Some(g) = doc.uuid_field(el, "guid") else { continue };
        for d in descendants(doc, el) {
            if doc.node(d).map(|n| n.tag.as_str()) == Some("CLayerInputData")
                && let Some(layer) = doc.field(d, "layer")
            {
                image_layer.insert(g.clone(), layer);
                break;
            }
        }
    }

    // deformers whose parent guid means "none"
    let mut no_parent: std::collections::HashSet<String> =
        ["00000000-0000-0000-0000-000000000000".to_string()].into_iter().collect();
    for &g in doc.all("CDeformerGuid") {
        if doc.attr(g, "note") == Some("ROOT")
            && let Some(u) = doc.attr(g, "uuid")
        {
            no_parent.insert(u.to_string());
        }
    }
    let parent_uuid = |src: usize| -> Option<String> {
        doc.uuid_field(src, "targetDeformerGuid").filter(|u| !no_parent.contains(u))
    };

    // deformers, with their resting shape and every keyform
    let mut deformers = Vec::new();
    for tag in ["CWarpDeformerSource", "CRotationDeformerSource"] {
        for &el in doc.all(tag) {
            if doc.attr(el, "xs.id").is_none() || doc.node(el).is_none_or(|n| n.children.is_empty())
            {
                continue;
            }
            let Some(dsrc) = doc.field_raw(el, "super") else { continue };
            let Some(ctrl) = doc.field_raw(dsrc, "super") else { continue };
            let mut shape = None;
            if let Some(exts) = doc.field_raw(ctrl, "_extensions") {
                for ext in doc.items(exts) {
                    if doc.node(ext).map(|n| n.tag.as_str())
                        == Some("CDeformerOriginalShapeExtension")
                    {
                        shape = doc.field_raw(ext, "originalShape");
                    }
                }
            }
            // Cubism 4 projects keep no original shape; nothing here needs it
            let _ = shape;
            let uuid = doc.uuid_field(dsrc, "guid").unwrap_or_default();
            let id =
                doc.field(dsrc, "id").and_then(|i| doc.attr(i, "idstr")).unwrap_or("").to_string();
            let (bindings, kfs) = cx.keyforms(ctrl, el);
            let is_warp = tag == "CWarpDeformerSource";
            let mut forms = Vec::new();
            for (coords, form) in kfs {
                let (opacity, _) = cx.form_common(form);
                let mut f = Form {
                    coords,
                    opacity,
                    draw_order: 500.0,
                    positions: Vec::new(),
                    angle: 0.0,
                    origin: [0.0, 0.0],
                    scale: 1.0,
                    reflect: [false, false],
                };
                if is_warp {
                    f.positions =
                        doc.field(form, "positions").map(|p| doc.floats(p)).unwrap_or_default();
                } else {
                    let num = |k: &str| {
                        doc.attr(form, k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0)
                    };
                    f.angle = num("angle");
                    f.origin = [num("originX"), num("originY")];
                    f.scale = doc.attr(form, "scale").and_then(|v| v.parse().ok()).unwrap_or(1.0);
                    f.reflect = [
                        doc.attr(form, "isReflectX") == Some("true"),
                        doc.attr(form, "isReflectY") == Some("true"),
                    ];
                }
                forms.push(f);
            }
            let kind = if is_warp {
                DeformerKind::Warp {
                    cols: doc.int_field(el, "col").unwrap_or(1).max(1) as u32,
                    rows: doc.int_field(el, "row").unwrap_or(1).max(1) as u32,
                }
            } else {
                DeformerKind::Rotation
            };
            deformers.push(Deformer { uuid, id, parent: parent_uuid(dsrc), kind, bindings, forms });
        }
    }
    // parents before children
    let depth_of = |d: &Deformer, all: &[Deformer]| -> usize {
        let mut n = 0;
        let mut cur = d.parent.clone();
        let mut seen = std::collections::HashSet::new();
        while let Some(p) = cur {
            if !seen.insert(p.clone()) || n > 64 {
                break;
            }
            match all.iter().find(|x| x.uuid == p) {
                Some(x) => {
                    n += 1;
                    cur = x.parent.clone();
                }
                None => break,
            }
        }
        n
    };
    let depths: Vec<usize> = deformers.iter().map(|d| depth_of(d, &deformers)).collect();
    let mut order: Vec<usize> = (0..deformers.len()).collect();
    order.sort_by_key(|&i| depths.get(i).copied().unwrap_or(0));
    let mut sorted = Vec::with_capacity(deformers.len());
    let mut taken: Vec<Option<Deformer>> = deformers.into_iter().map(Some).collect();
    for i in order {
        if let Some(d) = taken.get_mut(i).and_then(Option::take) {
            sorted.push(d);
        }
    }
    let deformers = sorted;

    // parameters and their folders, in palette order
    let mut folders: HashMap<String, String> = HashMap::new();
    let mut root_children: Vec<String> = Vec::new();
    for &el in doc.all("CParameterGroup") {
        let Some(gu) = doc.field_raw(el, "guid").and_then(|g| guid_key(doc, g)) else { continue };
        folders.insert(gu, doc.str_field(el, "name"));
        if doc
            .field_raw(el, "parentGroupGuid")
            .and_then(|p| doc.node(p))
            .is_some_and(|n| n.tag == "null")
            && let Some(kids) = doc.field_raw(el, "_childGuids")
        {
            root_children = doc.items_raw(kids).iter().filter_map(|&k| guid_key(doc, k)).collect();
        }
    }
    let root_uuid =
        folders.iter().find(|(_, v)| *v == "Root Parameter Group").map(|(k, _)| k.clone());
    let mut params = Vec::new();
    for &el in doc.all("CParameterSource") {
        let Some(id) = doc.field(el, "id").and_then(|i| doc.attr(i, "idstr")) else { continue };
        let gu = doc.field_raw(el, "parentGroupGuid").and_then(|g| guid_key(doc, g));
        let folder = match &gu {
            Some(g) if Some(g) != root_uuid.as_ref() => folders.get(g).cloned().unwrap_or_default(),
            _ => String::new(),
        };
        let folder_rank = gu
            .as_ref()
            .and_then(|g| root_children.iter().position(|c| c == g))
            .map_or(-1, |p| p as i64);
        params.push(Param {
            id: id.to_string(),
            name: doc.str_field(el, "name"),
            min: doc.float_field(el, "minValue").unwrap_or(0.0),
            max: doc.float_field(el, "maxValue").unwrap_or(1.0),
            default: doc.float_field(el, "defaultValue").unwrap_or(0.0),
            repeat: doc.bool_field(el, "isRepeat", false),
            decimals: doc.int_field(el, "decimalPlaces").unwrap_or(3) as i32,
            folder,
            folder_rank,
        });
    }

    // physics, as physics3 has it
    use ayatsuri_format::model::{PhysicsIo, PhysicsNode};
    let mut physics_fps = 60.0;
    for &pset in doc.all("CPhysicsSettingsSourceSet") {
        if let Some(f) = doc.float_field(pset, "settingFPS") {
            physics_fps = f;
        }
    }
    let kind_of = |el: usize| match doc.field(el, "type").and_then(|t| doc.attr(t, "v")) {
        Some("SRC_TO_Y") => PhysicsIo::Y,
        Some("SRC_TO_G_ANGLE") => PhysicsIo::Angle,
        _ => PhysicsIo::X,
    };
    let vec2 = |el: usize, name: &str| -> (f32, f32) {
        match doc.field(el, name) {
            Some(v) => {
                (doc.float_field(v, "x").unwrap_or(0.0), doc.float_field(v, "y").unwrap_or(0.0))
            }
            None => (0.0, 0.0),
        }
    };
    let mut physics = Vec::new();
    for &el in doc.all("CPhysicsSettingsSource") {
        if doc.node(el).is_none_or(|n| n.children.is_empty()) {
            continue;
        }
        let mut g = PhysGroup {
            id: doc.field(el, "id").and_then(|i| doc.attr(i, "idstr")).unwrap_or("").to_string(),
            name: doc.str_field(el, "name"),
            inputs: Vec::new(),
            outputs: Vec::new(),
            nodes: Vec::new(),
            position: [-10.0, 0.0, 10.0],
            angle: [-10.0, 0.0, 10.0],
        };
        if let Some(list) = doc.field_raw(el, "inputs") {
            for i in doc.items(list) {
                g.inputs.push(PhysIo {
                    param: doc
                        .uuid_field(i, "source")
                        .and_then(|u| cx.param_of_uuid.get(&u).cloned())
                        .unwrap_or_default(),
                    weight: doc.float_field(i, "weight").unwrap_or(0.0),
                    kind: kind_of(i),
                    reflect: doc.bool_field(i, "isReverse", false),
                    vertex: 0,
                    scale: 0.0,
                });
            }
        }
        if let Some(list) = doc.field_raw(el, "outputs") {
            for o in doc.items(list) {
                let kind = kind_of(o);
                let (tx, ty) = vec2(o, "translationScale");
                let scale = match kind {
                    PhysicsIo::Angle => doc.float_field(o, "angleScale").unwrap_or(0.0),
                    PhysicsIo::X => tx,
                    PhysicsIo::Y => ty,
                };
                g.outputs.push(PhysIo {
                    param: doc
                        .uuid_field(o, "destination")
                        .and_then(|u| cx.param_of_uuid.get(&u).cloned())
                        .unwrap_or_default(),
                    weight: doc.float_field(o, "weight").unwrap_or(0.0),
                    kind,
                    reflect: doc.bool_field(o, "isReverse", false),
                    vertex: doc.int_field(o, "vertexIndex").unwrap_or(0).max(0) as u32,
                    scale,
                });
            }
        }
        if let Some(list) = doc.field_raw(el, "vertices") {
            for v in doc.items(list) {
                let (x, y) = vec2(v, "position");
                g.nodes.push(PhysicsNode {
                    x,
                    y,
                    mobility: doc.float_field(v, "mobility").unwrap_or(1.0),
                    delay: doc.float_field(v, "delay").unwrap_or(1.0),
                    acceleration: doc.float_field(v, "acceleration").unwrap_or(1.0),
                    radius: doc.float_field(v, "radius").unwrap_or(0.0),
                });
            }
        }
        let f = |k: &str, d: f32| doc.float_field(el, k).unwrap_or(d);
        g.position = [
            f("normalizedPositionValueMin", -10.0),
            f("normalizedPositionDefaultValue", 0.0),
            f("normalizedPositionValueMax", 10.0),
        ];
        g.angle = [
            f("normalizedAngleValueMin", -10.0),
            f("normalizedAngleDefaultValue", 0.0),
            f("normalizedAngleValueMax", 10.0),
        ];
        physics.push(g);
    }

    // meshes: the layer each draws, its points, triangles, clips and forms
    struct Raw {
        mesh: Mesh,
        guid_key: Option<String>,
        guid_uuid: Option<String>,
        clip_keys: Vec<String>,
    }
    let mut raws: Vec<Raw> = Vec::new();
    for &el in doc.all("CArtMeshSource") {
        if doc.attr(el, "xs.id").is_none() || doc.node(el).is_none_or(|n| n.children.is_empty()) {
            continue;
        }
        let Some(draw_src) = doc.field_raw(el, "super") else { continue };
        let Some(ctrl) = doc.field_raw(draw_src, "super") else { continue };
        let name = doc.str_field(ctrl, "localName");
        let pg = doc
            .field_raw(ctrl, "parentGuid")
            .and_then(|g| doc.attr(g, "xs.ref"))
            .map(str::to_string);
        let mid = doc
            .field(draw_src, "id")
            .and_then(|i| doc.attr(i, "idstr"))
            .map(str::to_string)
            .unwrap_or_else(|| name.clone());
        let guid = doc.field_raw(draw_src, "guid");
        let clip_keys: Vec<String> = doc
            .field_raw(draw_src, "clipGuidList")
            .map(|l| {
                doc.items_raw(l)
                    .iter()
                    .filter_map(|&c| doc.attr(c, "xs.ref").map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let (Some(positions), Some(indices)) =
            (doc.field(el, "positions"), doc.field(el, "indices"))
        else {
            continue;
        };
        let mut mesh_flags = 0;
        match doc.field(el, "colorComposition").and_then(|c| doc.attr(c, "v")) {
            Some("ADD") => mesh_flags |= flags::BLEND_ADDITIVE,
            Some("MULTIPLY") => mesh_flags |= flags::BLEND_MULTIPLICATIVE,
            Some("NORMAL") | None => {}
            Some(other) => log::info!("cmo3: {mid}: blend {other} is drawn normal"),
        }
        // culling off is the Editor's default: both sides drawn
        if !doc.bool_field(el, "culling", false) {
            mesh_flags |= flags::DOUBLE_SIDED;
        }
        if doc.bool_field(draw_src, "invertClippingMask", false) {
            mesh_flags |= flags::INVERTED_MASK;
        }
        let mut layer = None;
        if let Some(exts) = doc.field_raw(ctrl, "_extensions") {
            for ext in doc.items(exts) {
                if doc.node(ext).map(|n| n.tag.as_str()) != Some("CTextureInputExtension") {
                    continue;
                }
                if let Some(tis) = doc.field_raw(ext, "_textureInputs") {
                    for ti in doc.items(tis) {
                        if doc.node(ti).map(|n| n.tag.as_str()) == Some("CTextureInput_ModelImage")
                            && let Some(hit) = doc
                                .uuid_field(ti, "_modelImageGuid")
                                .and_then(|u| image_layer.get(&u).copied())
                        {
                            layer = Some(hit);
                        }
                    }
                }
            }
        }
        let Some(layer) = layer else {
            log::warn!("cmo3: {mid} draws no layer; skipped");
            continue;
        };
        let rect = match doc.field(layer, "boundsOnImageDoc") {
            Some(r) => [
                doc.int_field(r, "x").unwrap_or(0) as i32,
                doc.int_field(r, "y").unwrap_or(0) as i32,
                doc.int_field(r, "width").unwrap_or(0) as i32,
                doc.int_field(r, "height").unwrap_or(0) as i32,
            ],
            None => [0, 0, 0, 0],
        };
        let png = doc
            .field(layer, "imageResource")
            .and_then(|r| doc.field_raw(r, "imageFileBuf"))
            .and_then(|b| doc.attr(b, "path"))
            .and_then(|p| caff.read(p));
        let Some(png) = png else {
            log::warn!("cmo3: {mid}: its layer image is missing from the archive; skipped");
            continue;
        };
        let (bindings, kfs) = cx.keyforms(ctrl, el);
        let mut forms = Vec::new();
        for (coords, form) in kfs {
            let (opacity, draw_order) = cx.form_common(form);
            forms.push(Form {
                coords,
                opacity,
                draw_order,
                positions: doc.field(form, "positions").map(|p| doc.floats(p)).unwrap_or_default(),
                angle: 0.0,
                origin: [0.0, 0.0],
                scale: 1.0,
                reflect: [false, false],
            });
        }
        let triangles: Vec<u16> =
            doc.ints(indices).iter().map(|&v| v.clamp(0, u16::MAX as i64) as u16).collect();
        raws.push(Raw {
            mesh: Mesh {
                id: mid,
                name,
                parts: cx.part_path(pg),
                deformer: parent_uuid(draw_src),
                rect,
                png,
                points: doc.floats(positions),
                triangles,
                clip: Vec::new(),
                bindings,
                forms,
                flags: mesh_flags,
            },
            guid_key: guid.and_then(|g| doc.attr(g, "xs.ref").map(str::to_string)),
            guid_uuid: guid.and_then(|g| doc.uuid_of(g)),
            clip_keys,
        });
    }
    if raws.is_empty() {
        return Err(err("no art meshes in this project"));
    }

    // Cubism sorts every mesh together: by draw order, ties by the Parts
    // palette, where higher draws in front. Walk the palette from the root
    // part, top to bottom, to rank the meshes.
    let mut part_src: HashMap<String, usize> = HashMap::new();
    let mut root_part = None;
    for &el in doc.all("CPartSource") {
        if doc.attr(el, "xs.id").is_none() || doc.node(el).is_none_or(|n| n.children.is_empty()) {
            continue;
        }
        if let Some(u) = doc.uuid_field(el, "guid") {
            part_src.insert(u, el);
        }
        if doc.walk_super(el, "localName").map(|n| doc.text(n)) == Some("Root Part") {
            root_part = Some(el);
        }
    }
    let mut rank: HashMap<String, usize> = HashMap::new();
    if let Some(root) = root_part {
        // the palette in order: a sub-part's meshes rank where the sub-part
        // sits among its siblings, so each frame walks its list in turn
        let children = |part: usize| -> Vec<usize> {
            doc.field_raw(part, "_childGuids")
                .map(|k| doc.items_raw(k).to_vec())
                .unwrap_or_default()
        };
        let mut frames: Vec<(Vec<usize>, usize)> = vec![(children(root), 0)];
        let mut visited = std::collections::HashSet::new();
        visited.insert(root);
        while let Some((items, cursor)) = frames.last_mut() {
            let Some(&k) = items.get(*cursor) else {
                frames.pop();
                continue;
            };
            *cursor += 1;
            let tag = doc.node(k).map(|n| n.tag.as_str()).unwrap_or("");
            let Some(u) = doc.uuid_of(k) else { continue };
            match tag {
                "CPartGuid" => {
                    if let Some(&p) = part_src.get(&u)
                        && visited.insert(p)
                        && frames.len() < 64
                    {
                        frames.push((children(p), 0));
                    }
                }
                "CDrawableGuid" => {
                    let n = rank.len();
                    rank.entry(u).or_insert(n);
                }
                _ => {}
            }
        }
    }
    let total = rank.len();
    raws.sort_by_key(|r| {
        let k = r.guid_uuid.as_ref().and_then(|u| rank.get(u).copied()).unwrap_or(total);
        std::cmp::Reverse(k)
    });
    let index: HashMap<String, usize> =
        raws.iter().enumerate().filter_map(|(i, r)| r.guid_key.clone().map(|k| (k, i))).collect();
    let mut meshes = Vec::with_capacity(raws.len());
    for r in raws {
        let mut m = r.mesh;
        m.clip = r.clip_keys.iter().filter_map(|k| index.get(k).copied()).collect();
        meshes.push(m);
    }

    let canvas = doc
        .all("CImageCanvas")
        .first()
        .map(|&c| {
            (
                doc.int_field(c, "pixelWidth").unwrap_or(0).max(0) as u32,
                doc.int_field(c, "pixelHeight").unwrap_or(0).max(0) as u32,
            )
        })
        .ok_or_else(|| err("no canvas"))?;
    if canvas.0 == 0 || canvas.1 == 0 {
        return Err(err("canvas has no size"));
    }
    Ok(Extract { canvas, meshes, deformers, params, physics_fps, physics })
}
