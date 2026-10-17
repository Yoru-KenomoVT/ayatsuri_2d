//! The ABI driven the way a host drives it: raw pointers in, statuses and
//! views out, on a small model written to .aom first.

use std::ffi::CString;

use super::*;

use crate::fixture::small_aom;

fn load() -> *mut Model {
    let bytes = small_aom();
    let m = { ay2d_load(bytes.as_ptr(), bytes.len()) };
    assert!(!m.is_null(), "{}", last_error());
    m
}

fn last_error() -> String {
    cstr(ay2d_last_error())
}

fn cstr(p: *const std::ffi::c_char) -> String {
    crate::handle::cstr_in(p).unwrap().to_string_lossy().into_owned()
}

/// A drawable with nothing in it, for an output slot.
fn blank() -> Drawable {
    Drawable {
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
    }
}

/// The first `n` floats behind a drawable's pointer.
fn floats(p: *const f32, n: usize) -> Vec<f32> {
    crate::handle::input(p, n).unwrap().to_vec()
}

#[test]
fn versions_are_there() {
    assert_eq!(ay2d_abi_version(), AY2D_ABI_VERSION);
    assert_eq!(cstr(ay2d_version()), env!("CARGO_PKG_VERSION"));
}

#[test]
fn garbage_is_a_null_with_a_reason() {
    let junk = b"definitely not a model";
    let m = { ay2d_load(junk.as_ptr(), junk.len()) };
    assert!(m.is_null());
    assert!(last_error().contains("not a model"), "{}", last_error());
    let m = { ay2d_load(std::ptr::null(), 4) };
    assert!(m.is_null());
    assert!(last_error().contains("null"));
}

#[test]
fn a_null_handle_answers_nothing() {
    assert_eq!({ ay2d_parameter_count(std::ptr::null()) }, 0);
    assert!(last_error().contains("null"));
    assert_eq!({ ay2d_update(std::ptr::null_mut(), 0.016) }, Status::Null);
    ay2d_free(std::ptr::null_mut());
}

#[test]
fn counts_ids_and_lookups() {
    let m = load();
    {
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
    {
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
    {
        let mut d = [blank()];
        assert_eq!(ay2d_drawables(m, std::ptr::null_mut(), 0), 1, "a count without an array");
        assert_eq!(ay2d_drawables(m, d.as_mut_ptr(), 1), 1);
        assert_eq!(d[0].vertex_count, 4);
        assert_eq!(d[0].index_count, 6);
        assert_eq!(d[0].flags & AY2D_FLAG_DOUBLE_SIDED, AY2D_FLAG_DOUBLE_SIDED);
        let x0 = floats(d[0].positions, 1)[0];
        assert!((x0 - 0.0).abs() < 1e-5, "at rest the quad sits at 0: {x0}");
        let idx = [0u32];
        let val = [1.0f32];
        ay2d_set_parameters(m, idx.as_ptr(), val.as_ptr(), 1);
        assert_eq!(ay2d_update(m, 0.0), Status::Ok);
        ay2d_drawables(m, d.as_mut_ptr(), 1);
        let x1 = floats(d[0].positions, 1)[0];
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
    {
        let mut out = [5.0f32];
        assert_eq!(ay2d_get_part_opacities(m, out.as_mut_ptr(), 1), Status::Ok);
        assert_eq!(out[0], 1.0);
        let idx = [0u32];
        let half = [0.5f32];
        assert_eq!(ay2d_set_part_opacities(m, idx.as_ptr(), half.as_ptr(), 1), Status::Ok);
        ay2d_update(m, 0.0);
        let mut d = [blank()];
        assert_eq!(ay2d_drawables(m, d.as_mut_ptr(), 1), 1);
        assert!((d[0].opacity - 0.5).abs() < 1e-5);
        let bad = [3u32];
        assert_eq!(ay2d_set_part_opacities(m, bad.as_ptr(), half.as_ptr(), 1), Status::Range);
        ay2d_free(m);
    }
}

#[test]
fn physics_runs_only_while_on() {
    let m = load();
    {
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
    {
        let mut p: *const u8 = std::ptr::null();
        let mut n: usize = 0;
        assert_eq!(ay2d_texture(m, 0, &raw mut p, &raw mut n), Status::Ok);
        assert_eq!(crate::handle::input(p, n).unwrap(), b"not really a png");
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
        include_str!("motions.rs"),
        include_str!("expressions.rs"),
        include_str!("bundle.rs"),
        include_str!("info.rs"),
        include_str!("changes.rs"),
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

#[test]
fn motions_play_fade_and_fire_events() {
    let m = load();
    {
        assert_eq!(ay2d_motion_count(m), 2, "the scene and the composition");
        assert_eq!(cstr(ay2d_motion_id(m, 0)), "wave");
        assert_eq!(cstr(ay2d_motion_id(m, 1)), "both");
        let name = CString::new("both").unwrap();
        assert_eq!(ay2d_motion_index(m, name.as_ptr()), 1);
        let mut info = MotionInfo::default();
        assert_eq!(ay2d_motion_info(m, 0, &raw mut info), Status::Ok);
        assert_eq!((info.duration, info.event_count, info.composition), (1.0, 1, 0));
        assert_eq!(info.fade_in, -1.0, "no authored fade");
        assert_eq!(ay2d_motion_info(m, 1, &raw mut info), Status::Ok);
        assert_eq!((info.duration, info.composition), (2.0, 1));
        assert_eq!(ay2d_motion_playing(m), -1);
        assert_eq!(ay2d_motion_play(m, 0, 0, 0.0), Status::Ok, "full weight at once");
        assert_eq!(ay2d_motion_playing(m), 0);
        let mut out = [0.0f32; 2];
        // half a second in, the curve says 0.5 and the event fired
        let mut fired = 0;
        for _ in 0..30 {
            ay2d_update(m, 1.0 / 60.0);
            let mut ev = [Event { motion: 0, time: 0.0, name: std::ptr::null() }];
            let n = ay2d_motion_events(m, ev.as_mut_ptr(), 1);
            if n > 0 {
                fired += n;
                assert_eq!(cstr(ev[0].name), "clap");
                assert_eq!(ev[0].motion, 0);
            }
        }
        assert_eq!(fired, 1, "the event fired exactly once");
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert!((out[0] - 0.5).abs() < 0.02, "the motion drives ParamA: {}", out[0]);
        assert!((ay2d_motion_time(m) - 0.5).abs() < 0.02);
        // play through and hold the last frame
        for _ in 0..60 {
            ay2d_update(m, 1.0 / 60.0);
        }
        assert_eq!(ay2d_motion_finished(m), 1);
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert!((out[0] - 1.0).abs() < 1e-4, "held at the end: {}", out[0]);
        // seek back, slower
        assert_eq!(ay2d_motion_seek(m, 0.0), Status::Ok);
        assert_eq!(ay2d_motion_set_speed(m, 0.5), Status::Ok);
        for _ in 0..60 {
            ay2d_update(m, 1.0 / 60.0);
        }
        assert!((ay2d_motion_time(m) - 0.5).abs() < 0.02, "half speed: {}", ay2d_motion_time(m));
        // the composition plays the scene at half speed too
        assert_eq!(ay2d_motion_play(m, 1, 0, 0.0), Status::Ok);
        for _ in 0..60 {
            ay2d_update(m, 1.0 / 60.0);
        }
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert!((out[0] - 0.5).abs() < 0.03, "composition at one second: {}", out[0]);
        // a fade out over half a second leaves the host's value in charge
        assert_eq!(ay2d_motion_stop(m, 0.5), Status::Ok);
        assert_eq!(ay2d_motion_playing(m), -1);
        let idx = [0u32];
        for _ in 0..40 {
            let v = [0.0f32];
            ay2d_set_parameters(m, idx.as_ptr(), v.as_ptr(), 1);
            ay2d_update(m, 1.0 / 60.0);
        }
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert_eq!(out[0], 0.0, "faded out: the host's value stands");
        assert_eq!(ay2d_motion_play(m, 5, 0, 0.0), Status::Range);
        assert_eq!(ay2d_motion_seek(m, 0.0), Status::Range, "nothing playing");
        ay2d_free(m);
    }
}

#[test]
fn expressions_lay_over_the_pose() {
    let m = load();
    {
        assert_eq!(ay2d_expression_count(m), 1);
        assert_eq!(cstr(ay2d_expression_id(m, 0)), "smile");
        let name = CString::new("smile").unwrap();
        assert_eq!(ay2d_expression_index(m, name.as_ptr()), 0);
        assert_eq!(ay2d_expression_shown(m), -1);
        let idx = [0u32];
        let v = [0.5f32];
        ay2d_set_parameters(m, idx.as_ptr(), v.as_ptr(), 1);
        assert_eq!(ay2d_expression_set(m, 0, -1.0), Status::Ok, "authored fade: none");
        assert_eq!(ay2d_expression_shown(m), 0);
        ay2d_update(m, 0.0);
        let mut out = [0.0f32; 2];
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert!((out[0] - 0.75).abs() < 1e-5, "0.5 plus the expression's 0.25: {}", out[0]);
        // cleared over a second: half way, half the add
        assert_eq!(ay2d_expression_clear(m, 1.0), Status::Ok);
        assert_eq!(ay2d_expression_shown(m), -1);
        for _ in 0..30 {
            ay2d_set_parameters(m, idx.as_ptr(), v.as_ptr(), 1);
            ay2d_update(m, 1.0 / 60.0);
        }
        ay2d_get_parameters(m, out.as_mut_ptr(), 2);
        assert!((out[0] - 0.625).abs() < 0.02, "half faded: {}", out[0]);
        assert_eq!(ay2d_expression_set(m, 3, 0.0), Status::Range);
        ay2d_free(m);
    }
}

/// The whole boundary sits in handle.rs: no other file of the crate uses
/// the `unsafe` keyword for a block or a function.
#[test]
fn unsafe_stays_in_one_file() {
    let others = [
        ("lib.rs", include_str!("lib.rs")),
        ("error.rs", include_str!("error.rs")),
        ("params.rs", include_str!("params.rs")),
        ("parts.rs", include_str!("parts.rs")),
        ("physics.rs", include_str!("physics.rs")),
        ("draw.rs", include_str!("draw.rs")),
        ("textures.rs", include_str!("textures.rs")),
        ("motions.rs", include_str!("motions.rs")),
        ("expressions.rs", include_str!("expressions.rs")),
        ("bundle.rs", include_str!("bundle.rs")),
        ("info.rs", include_str!("info.rs")),
        ("changes.rs", include_str!("changes.rs")),
        ("fixture.rs", include_str!("fixture.rs")),
        ("tests.rs", include_str!("tests.rs")),
    ];
    // the keyword is spelled in pieces so this file passes its own check
    let kw = ["un", "safe"].concat();
    let needles = [format!("{kw} {{"), format!("{kw} fn"), format!("{kw} extern")];
    for (name, src) in others {
        for line in src.lines() {
            let code = line.split("//").next().unwrap_or("");
            assert!(!needles.iter().any(|n| code.contains(n.as_str())), "{name}: {line}");
        }
    }
}

/// A moc3 is the rig alone; the bundle's JSON files come in by hand.
#[test]
fn a_moc3_loads_and_takes_its_bundle_files() {
    use ayatsuri_format::read;
    let payload = read(&small_aom()).unwrap();
    let moc =
        ayatsuri_import::to_moc3(&payload.model, ayatsuri_import::ExportVersion::DEFAULT).unwrap();
    let m = ay2d_load(moc.as_ptr(), moc.len());
    assert!(!m.is_null(), "{}", last_error());
    assert_eq!(ay2d_parameter_count(m), 2);
    assert_eq!(ay2d_mesh_count(m), 1);
    assert_eq!(ay2d_texture_count(m), 1, "the rig draws from one page");
    let mut p: *const u8 = std::ptr::null();
    let mut n: usize = 7;
    assert_eq!(ay2d_texture(m, 0, &raw mut p, &raw mut n), Status::Ok);
    assert_eq!(n, 0, "no bytes of its own");
    assert_eq!(ay2d_has_physics(m), 0, "a moc3 carries no rig; physics3.json does");
    assert_eq!(ay2d_motion_count(m), 0);
    assert_eq!(ay2d_expression_count(m), 0);
    // the bundle files, as the host would read them beside the moc3
    let physics = ayatsuri_import::to_physics3(payload.model.physics.as_ref().unwrap());
    assert_eq!(ay2d_add_physics3(m, physics.as_ptr(), physics.len()), Status::Ok);
    assert_eq!(ay2d_has_physics(m), 1);
    let motion = ayatsuri_import::to_motion3(&payload.animations[0]);
    let name = CString::new("wave").unwrap();
    assert_eq!(ay2d_add_motion3(m, name.as_ptr(), motion.as_ptr(), motion.len()), 0);
    assert_eq!(cstr(ay2d_motion_id(m, 0)), "wave");
    let expr = ayatsuri_import::to_exp3(&payload.expressions[0]);
    let ename = CString::new("smile").unwrap();
    assert_eq!(ay2d_add_expression3(m, ename.as_ptr(), expr.as_ptr(), expr.len()), 0);
    assert_eq!(ay2d_motion_play(m, 0, 0, 0.0), Status::Ok);
    for _ in 0..30 {
        ay2d_update(m, 1.0 / 60.0);
    }
    let mut out = [0.0f32; 2];
    ay2d_get_parameters(m, out.as_mut_ptr(), 2);
    assert!(
        (out[0] - 0.5).abs() < 0.02,
        "the added motion drives the moc3's parameter: {}",
        out[0]
    );
    let junk = b"{ not json";
    assert_eq!(ay2d_add_motion3(m, name.as_ptr(), junk.as_ptr(), junk.len()), -1);
    assert!(last_error().contains("motion3"));
    ay2d_free(m);
}

#[test]
fn names_keys_parents_and_groups() {
    let m = load();
    assert_eq!(cstr(ay2d_parameter_name(m, 0)), "Slide");
    assert_eq!(cstr(ay2d_parameter_name(m, 1)), "Sway");
    assert_eq!(cstr(ay2d_part_name(m, 0)), "Body");
    assert_eq!(cstr(ay2d_mesh_name(m, 0)), "The quad");
    assert!(ay2d_parameter_name(m, 9).is_null());
    assert_eq!(ay2d_parameter_repeat(m, 0), 0);
    let mut keys = [9.0f32; 4];
    assert_eq!(ay2d_parameter_keys(m, 0, keys.as_mut_ptr(), 4), 2, "rigged at 0 and 1");
    assert_eq!(&keys[..2], &[0.0, 1.0]);
    assert_eq!(ay2d_parameter_keys(m, 1, keys.as_mut_ptr(), 4), 0, "nothing rigged on it");
    assert_eq!(ay2d_parameter_keys(m, 7, keys.as_mut_ptr(), 4), 0);
    assert_eq!(ay2d_part_parent(m, 0), -1);
    assert_eq!(ay2d_mesh_part(m, 0), 0);
    assert_eq!(ay2d_parameter_group_count(m), 1);
    assert_eq!(cstr(ay2d_parameter_group_name(m, 0)), "Motion");
    let mut members = [7u32; 4];
    assert_eq!(ay2d_parameter_group(m, 0, members.as_mut_ptr(), 4), 2);
    assert_eq!(&members[..2], &[0, 1]);
    assert_eq!(ay2d_model_group_count(m), 1);
    assert_eq!(cstr(ay2d_model_group_name(m, 0)), "LipSync");
    assert_eq!(ay2d_model_group(m, 0, members.as_mut_ptr(), 4), 1);
    assert_eq!(members[0], 0);
    ay2d_free(m);
}

#[test]
fn change_flags_say_what_moved() {
    let m = load();
    let mut flags = [0u32; 1];
    assert_eq!(ay2d_mesh_changes(m, flags.as_mut_ptr(), 1), Status::Ok);
    assert_eq!(flags[0], 15, "everything is new after load");
    ay2d_update(m, 0.0);
    ay2d_mesh_changes(m, flags.as_mut_ptr(), 1);
    assert_eq!(flags[0], 0, "nothing moved");
    let idx = [0u32];
    let v = [1.0f32];
    ay2d_set_parameters(m, idx.as_ptr(), v.as_ptr(), 1);
    ay2d_update(m, 0.0);
    ay2d_mesh_changes(m, flags.as_mut_ptr(), 1);
    assert_eq!(flags[0], AY2D_CHANGED_POSITIONS, "only the vertices moved");
    let half = [0.5f32];
    ay2d_set_part_opacities(m, idx.as_ptr(), half.as_ptr(), 1);
    ay2d_update(m, 0.0);
    ay2d_mesh_changes(m, flags.as_mut_ptr(), 1);
    assert_eq!(flags[0], AY2D_CHANGED_OPACITY);
    assert_eq!(ay2d_mesh_changes(m, flags.as_mut_ptr(), 0), Status::TooSmall);
    ay2d_free(m);
}
