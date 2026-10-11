//! The ABI driven the way a host drives it: raw pointers in, statuses and
//! views out, on a small model written to .aom first.

use std::ffi::{CStr, CString};

use ayatsuri_format::model::{
    ArtMesh, Binding, BindingAxis, Model as Rig, Parameter, Part, PhysicsGroup, PhysicsInput,
    PhysicsIo, PhysicsNode, PhysicsOutput, PhysicsRange, PhysicsRig, flags,
};
use ayatsuri_format::payload::AomPayload;

use super::*;

/// A canvas with one parameter that slides a quad right, a second the rig
/// drives, one part and one texture page.
fn small_aom() -> Vec<u8> {
    let mut model = Rig::new();
    model.canvas = ayatsuri_format::model::Canvas {
        pixels_per_unit: 1024.0,
        origin_x: 512.0,
        origin_y: 512.0,
        width: 1024.0,
        height: 1024.0,
    };
    model.parameters.push(Parameter {
        id: "ParamA".into(),
        min: -1.0,
        max: 1.0,
        default: 0.0,
        repeat: false,
        decimals: 3,
    });
    model.parameters.push(Parameter {
        id: "ParamB".into(),
        min: -10.0,
        max: 10.0,
        default: 0.0,
        repeat: false,
        decimals: 3,
    });
    model.parts.push(Part {
        id: "Part".into(),
        parent_part: None,
        visible: true,
        enabled: true,
        binding: Binding::default(),
        draw_orders: vec![500.0],
    });
    let quad = |dx: f32| vec![dx, 0.0, dx + 0.5, 0.0, dx + 0.5, 0.5, dx, 0.5];
    model.art_meshes.push(ArtMesh {
        id: "Quad".into(),
        parent_part: Some(0),
        parent_deformer: None,
        visible: true,
        enabled: true,
        texture: 0,
        flags: flags::DOUBLE_SIDED,
        vertex_count: 4,
        uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
        indices: vec![0, 1, 2, 0, 2, 3],
        masks: Vec::new(),
        binding: Binding { axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }] },
        opacities: vec![1.0, 1.0],
        draw_orders: vec![500.0, 500.0],
        positions: quad(0.0).into_iter().chain(quad(1.0)).collect(),
    });
    model.physics = Some(PhysicsRig {
        fps: 60.0,
        gravity_x: 0.0,
        gravity_y: -1.0,
        wind_x: 0.0,
        wind_y: 0.0,
        groups: vec![PhysicsGroup {
            id: "PhysicsSetting1".into(),
            name: "Sway".into(),
            inputs: vec![PhysicsInput {
                param: "ParamA".into(),
                weight: 100.0,
                kind: PhysicsIo::X,
                reflect: false,
            }],
            outputs: vec![PhysicsOutput {
                param: "ParamB".into(),
                vertex: 1,
                scale: 10.0,
                weight: 100.0,
                kind: PhysicsIo::Angle,
                reflect: false,
            }],
            nodes: vec![
                PhysicsNode {
                    x: 0.0,
                    y: 0.0,
                    mobility: 1.0,
                    delay: 1.0,
                    acceleration: 1.0,
                    radius: 0.0,
                },
                PhysicsNode {
                    x: 0.0,
                    y: 5.0,
                    mobility: 0.9,
                    delay: 1.0,
                    acceleration: 1.0,
                    radius: 5.0,
                },
            ],
            norm_position: PhysicsRange { min: -10.0, default: 0.0, max: 10.0 },
            norm_angle: PhysicsRange { min: -10.0, default: 0.0, max: 10.0 },
        }],
    });
    let payload = AomPayload {
        model,
        textures: vec![serde_bytes::ByteBuf::from(b"not really a png".to_vec())],
        ..Default::default()
    };
    ayatsuri_format::write(&payload).unwrap()
}

fn load() -> *mut Model {
    let bytes = small_aom();
    let m = unsafe { ay2d_load(bytes.as_ptr(), bytes.len()) };
    assert!(!m.is_null(), "{}", last_error());
    m
}

fn last_error() -> String {
    unsafe { CStr::from_ptr(ay2d_last_error()) }.to_string_lossy().into_owned()
}

fn cstr(p: *const std::ffi::c_char) -> String {
    assert!(!p.is_null());
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

#[test]
fn versions_are_there() {
    assert_eq!(ay2d_abi_version(), 1);
    assert_eq!(cstr(ay2d_version()), env!("CARGO_PKG_VERSION"));
}

#[test]
fn garbage_is_a_null_with_a_reason() {
    let junk = b"definitely not a model";
    let m = unsafe { ay2d_load(junk.as_ptr(), junk.len()) };
    assert!(m.is_null());
    assert!(last_error().contains("not a model"), "{}", last_error());
    let m = unsafe { ay2d_load(std::ptr::null(), 4) };
    assert!(m.is_null());
    assert!(last_error().contains("null"));
}

#[test]
fn a_null_handle_answers_nothing() {
    assert_eq!(unsafe { ay2d_parameter_count(std::ptr::null()) }, 0);
    assert!(last_error().contains("null"));
    assert_eq!(unsafe { ay2d_update(std::ptr::null_mut(), 0.016) }, Status::Null);
    unsafe { ay2d_free(std::ptr::null_mut()) };
}

#[test]
fn counts_ids_and_lookups() {
    let m = load();
    unsafe {
        assert_eq!(ay2d_parameter_count(m), 2);
        assert_eq!(ay2d_part_count(m), 1);
        assert_eq!(ay2d_mesh_count(m), 1);
        assert_eq!(ay2d_texture_count(m), 1);
        assert_eq!(cstr(ay2d_parameter_id(m, 0)), "ParamA");
        assert_eq!(cstr(ay2d_parameter_id(m, 1)), "ParamB");
        assert!(ay2d_parameter_id(m, 2).is_null());
        assert_eq!(cstr(ay2d_part_id(m, 0)), "Part");
        assert_eq!(cstr(ay2d_mesh_id(m, 0)), "Quad");
        let a = CString::new("ParamA").unwrap();
        let nope = CString::new("ParamNope").unwrap();
        assert_eq!(ay2d_parameter_index(m, a.as_ptr()), 0);
        assert_eq!(ay2d_parameter_index(m, nope.as_ptr()), -1);
        let mut c = Canvas::default();
        assert_eq!(ay2d_canvas(m, &raw mut c), Status::Ok);
        assert_eq!((c.width, c.height, c.pixels_per_unit), (1024.0, 1024.0, 1024.0));
        ay2d_free(m);
    }
}

#[test]
fn parameters_go_in_and_out_as_arrays() {
    let m = load();
    unsafe {
        let (mut lo, mut hi, mut def) = ([0.0f32; 2], [0.0f32; 2], [9.0f32; 2]);
        assert_eq!(
            ay2d_parameter_ranges(m, lo.as_mut_ptr(), hi.as_mut_ptr(), def.as_mut_ptr(), 2),
            Status::Ok
        );
        assert_eq!((lo[0], hi[0], def[0]), (-1.0, 1.0, 0.0));
        assert_eq!((lo[1], hi[1]), (-10.0, 10.0));
        assert_eq!(
            ay2d_parameter_ranges(
                m,
                lo.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1
            ),
            Status::TooSmall
        );
        let idx = [0u32];
        let val = [0.75f32];
        assert_eq!(ay2d_set_parameters(m, idx.as_ptr(), val.as_ptr(), 1), Status::Ok);
        let mut out = [0.0f32; 2];
        assert_eq!(ay2d_get_parameters(m, out.as_mut_ptr(), 2), Status::Ok);
        assert_eq!(out[0], 0.75);
        // clamped to the range
        let big = [5.0f32];
        assert_eq!(ay2d_set_parameters(m, idx.as_ptr(), big.as_ptr(), 1), Status::Ok);
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert_eq!(out[0], 1.0);
        // a bad index fails before anything changes
        let bad = [0u32, 7];
        let vals = [-1.0f32, 3.0];
        assert_eq!(ay2d_set_parameters(m, bad.as_ptr(), vals.as_ptr(), 2), Status::Range);
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert_eq!(out[0], 1.0, "nothing moved on a failed call");
        let all = [-0.5f32, 2.0];
        assert_eq!(ay2d_set_all_parameters(m, all.as_ptr(), 2), Status::Ok);
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert_eq!(out, [-0.5, 2.0]);
        assert_eq!(ay2d_set_all_parameters(m, all.as_ptr(), 1), Status::TooSmall);
        assert_eq!(ay2d_get_parameters(m, out.as_mut_ptr(), 1), Status::TooSmall);
        ay2d_free(m);
    }
}

#[test]
fn an_update_moves_the_drawables() {
    let m = load();
    unsafe {
        let mut d = [Drawable {
            mesh: 0,
            texture: 0,
            flags: 0,
            opacity: 0.0,
            vertex_count: 0,
            index_count: 0,
            mask_count: 0,
            positions: std::ptr::null(),
            uvs: std::ptr::null(),
            indices: std::ptr::null(),
            masks: std::ptr::null(),
        }];
        assert_eq!(ay2d_drawables(m, std::ptr::null_mut(), 0), 1, "a count without an array");
        assert_eq!(ay2d_drawables(m, d.as_mut_ptr(), 1), 1);
        assert_eq!(d[0].vertex_count, 4);
        assert_eq!(d[0].index_count, 6);
        assert_eq!(d[0].flags & AY2D_FLAG_DOUBLE_SIDED, AY2D_FLAG_DOUBLE_SIDED);
        let x0 = *d[0].positions;
        assert!((x0 - 0.0).abs() < 1e-5, "at rest the quad sits at 0: {x0}");
        let idx = [0u32];
        let val = [1.0f32];
        ay2d_set_parameters(m, idx.as_ptr(), val.as_ptr(), 1);
        assert_eq!(ay2d_update(m, 0.0), Status::Ok);
        ay2d_drawables(m, d.as_mut_ptr(), 1);
        let x1 = *d[0].positions;
        assert!((x1 - 1.0).abs() < 1e-5, "at key 1 the quad sits at 1: {x1}");
        assert_eq!(ay2d_update(m, -1.0), Status::Range);
        // the geometry call sees the same mesh
        let mut g = d[0];
        assert_eq!(ay2d_mesh_geometry(m, 0, &raw mut g), Status::Ok);
        assert_eq!(g.vertex_count, 4);
        assert_eq!(ay2d_mesh_geometry(m, 3, &raw mut g), Status::Range);
        ay2d_free(m);
    }
}

#[test]
fn part_opacity_fades_the_mesh() {
    let m = load();
    unsafe {
        let mut out = [5.0f32];
        assert_eq!(ay2d_get_part_opacities(m, out.as_mut_ptr(), 1), Status::Ok);
        assert_eq!(out[0], 1.0);
        let idx = [0u32];
        let half = [0.5f32];
        assert_eq!(ay2d_set_part_opacities(m, idx.as_ptr(), half.as_ptr(), 1), Status::Ok);
        ay2d_update(m, 0.0);
        let mut d = std::mem::MaybeUninit::<Drawable>::uninit();
        assert_eq!(ay2d_drawables(m, d.as_mut_ptr(), 1), 1);
        assert!((d.assume_init().opacity - 0.5).abs() < 1e-5);
        let bad = [3u32];
        assert_eq!(ay2d_set_part_opacities(m, bad.as_ptr(), half.as_ptr(), 1), Status::Range);
        ay2d_free(m);
    }
}

#[test]
fn physics_runs_only_while_on() {
    let m = load();
    unsafe {
        assert_eq!(ay2d_has_physics(m), 1);
        let idx = [0u32];
        let mut out = [0.0f32; 2];
        // off: the output parameter stays where it is however the input swings
        assert_eq!(ay2d_set_physics(m, 0), Status::Ok);
        for i in 0..30 {
            let v = [if i % 2 == 0 { 1.0f32 } else { -1.0 }];
            ay2d_set_parameters(m, idx.as_ptr(), v.as_ptr(), 1);
            ay2d_update(m, 1.0 / 60.0);
        }
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert_eq!(out[1], 0.0);
        // on: the pendulum swings the output
        assert_eq!(ay2d_set_physics(m, 1), Status::Ok);
        assert_eq!(ay2d_set_wind(m, 0.5, 0.0), Status::Ok);
        for i in 0..60 {
            let v = [if (i / 10) % 2 == 0 { 1.0f32 } else { -1.0 }];
            ay2d_set_parameters(m, idx.as_ptr(), v.as_ptr(), 1);
            ay2d_update(m, 1.0 / 60.0);
        }
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert!(out[1] != 0.0, "the rig drove ParamB");
        assert_eq!(ay2d_reset_physics(m), Status::Ok);
        assert_eq!(ay2d_set_wind(m, f32::NAN, 0.0), Status::Range);
        ay2d_free(m);
    }
}

#[test]
fn textures_come_back_as_stored() {
    let m = load();
    unsafe {
        let mut p: *const u8 = std::ptr::null();
        let mut n: usize = 0;
        assert_eq!(ay2d_texture(m, 0, &raw mut p, &raw mut n), Status::Ok);
        assert_eq!(std::slice::from_raw_parts(p, n), b"not really a png");
        assert_eq!(ay2d_texture(m, 1, &raw mut p, &raw mut n), Status::Range);
        ay2d_free(m);
    }
}

/// Every function the crate exports is in the committed header; the xtask
/// check compares the whole text, this catches a missing regeneration
/// before it does.
#[test]
fn the_header_names_every_entry_point() {
    let header = include_str!("../include/ayatsuri.h");
    let sources = [
        include_str!("lib.rs"),
        include_str!("error.rs"),
        include_str!("handle.rs"),
        include_str!("params.rs"),
        include_str!("parts.rs"),
        include_str!("physics.rs"),
        include_str!("draw.rs"),
        include_str!("textures.rs"),
    ];
    let mut missing = Vec::new();
    for src in sources {
        for line in src.lines() {
            let Some(rest) = line.split("fn ay2d_").nth(1) else { continue };
            let name: String =
                rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if !header.contains(&format!("ay2d_{name}(")) {
                missing.push(name);
            }
        }
    }
    assert!(missing.is_empty(), "not in ayatsuri.h: {missing:?}; run cargo xtask capi");
}
