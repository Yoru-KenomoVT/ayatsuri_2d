//! Cubism Editor projects (`.cmo3`) as full models: art, meshes, parts, the
//! deformer tree, parameters with their folders, every keyform, physics
//! and draw order. Worked out from project files (see the research notes);
//! coordinates follow moc3's conventions, so the same conversions apply.

mod build;
mod caff;
mod caffw;
mod check;
mod extract;
mod write;
mod xml;
mod xmlw;

pub use build::Cmo3;
pub use write::{Cmo3Export, to_cmo3};

/// A project as a rig-ready model, with its display names and folders.
pub fn from_cmo3(bytes: &[u8], name: &str) -> Result<Cmo3, crate::ImportError> {
    let caff = caff::Caff::open(bytes)?;
    let xml = caff.main_xml()?;
    let doc = xml::Doc::parse(&xml)?;
    let ex = extract::extract(&caff, &doc)?;
    build::build(ex, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::DeformerKind;

    fn corpus(name: &str) -> Option<Vec<u8>> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../ayatsuri2d-research/corpus")
            .join(name);
        std::fs::read(p).ok()
    }

    /// A model built here goes out as a project and comes back the same:
    /// meshes under a warp and a rotation, keyforms, a folder, physics,
    /// and the art itself.
    #[test]
    fn a_model_round_trips_through_a_written_project() {
        use ayatsuri_format::model::*;
        let mut rgba = vec![0u8; 40 * 30 * 4];
        for px in rgba.chunks_mut(4) {
            px.copy_from_slice(&[220, 30, 30, 255]);
        }
        let mut blue = vec![0u8; 20 * 20 * 4];
        for px in blue.chunks_mut(4) {
            px.copy_from_slice(&[30, 30, 220, 255]);
        }
        let layer = |name: &str, x, y, w, h, rgba| crate::layers::Layer {
            name: name.into(),
            groups: vec!["Body".into()],
            x,
            y,
            width: w,
            height: h,
            rgba,
            visible: true,
            opacity: 1.0,
            clip_to_below: false,
            blend: crate::layers::Blend::Normal,
        };
        let doc = crate::layers::Document {
            width: 128,
            height: 96,
            layers: vec![layer("red", 10, 20, 40, 30, rgba), layer("blue", 60, 50, 20, 20, blue)],
        };
        let imported = crate::layers::build(&doc).unwrap();
        let mut model = imported.model;
        model.parameters.push(Parameter {
            id: "ParamA".into(),
            min: 0.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
        });
        let c = model.canvas;
        let unit = |px: f32, py: f32| {
            ((px - c.origin_x) / c.pixels_per_unit, (py - c.origin_y) / c.pixels_per_unit)
        };
        // a 2x2 warp over the canvas, identity at key 0 and bulged at key 1
        let (gx0, gy0) = unit(0.0, 0.0);
        let (gx1, gy1) = unit(128.0, 96.0);
        let mut grid = Vec::new();
        for r in 0..3 {
            for col in 0..3 {
                grid.push(gx0 + (gx1 - gx0) * col as f32 / 2.0);
                grid.push(gy0 + (gy1 - gy0) * r as f32 / 2.0);
            }
        }
        let mut bulged = grid.clone();
        bulged[8] += 0.1;
        bulged[9] -= 0.05;
        let axis = Binding { axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }] };
        model.deformers.push(Deformer {
            id: "Warp".into(),
            parent_part: Some(0),
            parent_deformer: None,
            visible: true,
            enabled: true,
            binding: axis.clone(),
            kind: DeformerKind::Warp(WarpData {
                rows: 2,
                cols: 2,
                quad: true,
                opacities: vec![1.0, 1.0],
                positions: grid.iter().chain(bulged.iter()).copied().collect(),
            }),
        });
        model.deformers.push(Deformer {
            id: "Rotation".into(),
            parent_part: Some(0),
            parent_deformer: None,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            kind: DeformerKind::Rotation(RotationData {
                base_angle: 0.0,
                opacities: vec![1.0],
                angles: vec![0.0],
                origins_x: vec![0.0],
                origins_y: vec![0.0],
                scales: vec![1.0],
                reflect_x: vec![false],
                reflect_y: vec![false],
            }),
        });
        // the red mesh under the warp, in grid space, two keyforms
        {
            let m = &mut model.art_meshes[0];
            m.parent_deformer = Some(0);
            m.binding = axis;
            let local: Vec<f32> = m
                .positions
                .chunks(2)
                .flat_map(|p| [(p[0] - gx0) / (gx1 - gx0), (p[1] - gy0) / (gy1 - gy0)])
                .collect();
            m.positions = local.iter().chain(local.iter()).copied().collect();
            m.opacities = vec![1.0, 0.5];
            m.draw_orders = vec![500.0, 600.0];
        }
        // the blue mesh under the rotation, offsets from its origin
        model.art_meshes[1].parent_deformer = Some(1);
        model.physics = Some(PhysicsRig {
            fps: 30.0,
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
                    param: "ParamA".into(),
                    vertex: 1,
                    scale: 7.5,
                    weight: 100.0,
                    kind: PhysicsIo::Angle,
                    reflect: true,
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
        let bytes = to_cmo3(&Cmo3Export {
            name: "roundtrip",
            model: &model,
            textures: &imported.textures,
            param_names: &["Sway amount".to_string()],
            part_names: &imported.part_names,
            mesh_names: &imported.mesh_names,
            param_groups: &[("Motion".to_string(), vec![0])],
        })
        .unwrap();
        let back = from_cmo3(&bytes, "roundtrip").unwrap();
        let m = &back.imported.model;
        assert_eq!(m.art_meshes.len(), 2);
        assert_eq!(m.deformers.len(), 2);
        assert_eq!(m.parameters.len(), 1);
        assert_eq!(back.param_names, vec!["Sway amount".to_string()]);
        assert_eq!(back.param_groups, vec![("Motion".to_string(), vec![0])]);
        assert_eq!(back.imported.mesh_names, vec!["red".to_string(), "blue".to_string()]);
        assert_eq!(back.imported.part_names, vec!["Body".to_string()]);
        assert_eq!(back.within, (2, 2), "meshes rest where they did: {:?}", back.off);
        let warp = m.deformers.iter().find(|d| d.id == "Warp").unwrap();
        assert_eq!(warp.binding.axes[0].keys, vec![0.0, 1.0]);
        let DeformerKind::Warp(w) = &warp.kind else { panic!("warp") };
        assert_eq!((w.rows, w.cols), (2, 2));
        let want: Vec<f32> = grid.iter().chain(bulged.iter()).copied().collect();
        for (a, b) in w.positions.iter().zip(&want) {
            assert!((a - b).abs() < 1e-3, "warp keyforms: {:?} vs {:?}", w.positions, want);
        }
        let red = m.art_meshes.iter().position(|a| a.id == "ArtMesh0").unwrap();
        let red_mesh = &m.art_meshes[red];
        assert_eq!(
            red_mesh.parent_deformer.map(|i| m.deformers[i as usize].id.clone()),
            Some("Warp".into())
        );
        assert_eq!(red_mesh.opacities, vec![1.0, 0.5]);
        assert_eq!(red_mesh.draw_orders, vec![500.0, 600.0]);
        for (a, b) in red_mesh.positions.iter().zip(&model.art_meshes[0].positions) {
            assert!((a - b).abs() < 1e-3, "mesh keyforms in grid space");
        }
        let blue = m.art_meshes.iter().position(|a| a.id == "ArtMesh1").unwrap();
        assert_eq!(
            m.art_meshes[blue].parent_deformer.map(|i| m.deformers[i as usize].id.clone()),
            Some("Rotation".into())
        );
        let rig = m.physics.as_ref().unwrap();
        assert_eq!(rig.fps, 30.0);
        assert_eq!(rig.groups.len(), 1);
        assert_eq!(rig.groups[0].name, "Sway");
        assert_eq!(rig.groups[0].inputs[0].param, "ParamA");
        assert_eq!(rig.groups[0].outputs[0].scale, 7.5);
        assert!(rig.groups[0].outputs[0].reflect);
        assert_eq!(rig.groups[0].nodes.len(), 2);
        // the art came back: the middle of the red mesh is red in its page
        let page = image::load_from_memory(&back.imported.textures[red_mesh.texture as usize])
            .unwrap()
            .to_rgba8();
        let n = red_mesh.vertex_count as f32;
        let (cu, cv) = (
            red_mesh.uvs.iter().step_by(2).sum::<f32>() / n,
            red_mesh.uvs.iter().skip(1).step_by(2).sum::<f32>() / n,
        );
        let px =
            page.get_pixel((cu * page.width() as f32) as u32, (cv * page.height() as f32) as u32).0;
        assert!(px[0] > 200 && px[2] < 60 && px[3] == 255, "red art came back: {px:?}");
    }

    /// For a look in Cubism Editor: `AYATSURI_CMO3_OUT=/path/out.cmo3 cargo
    /// test -p ayatsuri-import rewrite_for_a_look -- --ignored` writes the
    /// sample project back out through the writer.
    #[test]
    #[ignore]
    fn write_the_sample_project_rewrite_for_a_look() {
        let (Some(bytes), Ok(out)) =
            (corpus("Raiuji Vtuber model2.cmo3"), std::env::var("AYATSURI_CMO3_OUT"))
        else {
            return;
        };
        let first = from_cmo3(&bytes, "raiju").unwrap();
        let written = to_cmo3(&Cmo3Export {
            name: "raiju",
            model: &first.imported.model,
            textures: &first.imported.textures,
            param_names: &first.param_names,
            part_names: &first.imported.part_names,
            mesh_names: &first.imported.mesh_names,
            param_groups: &first.param_groups,
        })
        .unwrap();
        std::fs::write(out, written).unwrap();
    }

    /// The user's own project, read, written and read again, keeps every
    /// object the first read found.
    /// A project's graph and the names in its archive, for the check.
    fn strict(bytes: &[u8]) -> Vec<String> {
        let caff = caff::Caff::open(bytes).unwrap();
        let doc = xml::Doc::parse(&caff.main_xml().unwrap()).unwrap();
        let names: Vec<&str> = caff.entries.iter().map(|e| e.name.as_str()).collect();
        check::check(&doc, &check::Schema::builtin(), &names)
    }

    /// schema.txt is learned from the sample project, saved by the Editor:
    /// `cargo test -p ayatsuri-import -- --ignored write_the_schema`.
    #[test]
    #[ignore]
    fn write_the_schema() {
        let Some(bytes) = corpus("Raiuji Vtuber model2.cmo3") else { return };
        let caff = caff::Caff::open(&bytes).unwrap();
        let doc = xml::Doc::parse(&caff.main_xml().unwrap()).unwrap();
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cmo3/schema.txt");
        std::fs::write(out, check::Schema::learn(&doc).write()).unwrap();
    }

    /// The Editor's own project passes the check, so its rules are ones the
    /// Editor keeps.
    #[test]
    fn the_sample_project_passes_the_strict_check() {
        let Some(bytes) = corpus("Raiuji Vtuber model2.cmo3") else { return };
        let problems = strict(&bytes);
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// The check sees a missing field, a broken reference and a list that
    /// miscounts its items.
    #[test]
    fn the_strict_check_catches_damage() {
        let Some(bytes) = corpus("Raiuji Vtuber model2.cmo3") else { return };
        let caff = caff::Caff::open(&bytes).unwrap();
        let xml = String::from_utf8(caff.main_xml().unwrap()).unwrap();
        let damaged = xml
            .replacen("xs.ref=\"#5\"", "xs.ref=\"#999999\"", 1)
            .replace("<b xs.n=\"culling\">", "<b xs.n=\"cullingX\">")
            .replacen("count=\"", "count=\"9", 1);
        let doc = xml::Doc::parse(damaged.as_bytes()).unwrap();
        let names: Vec<&str> = caff.entries.iter().map(|e| e.name.as_str()).collect();
        let problems = check::check(&doc, &check::Schema::builtin(), &names).join("\n");
        assert!(problems.contains("is missing its field culling"), "{problems}");
        assert!(problems.contains("refers to #999999, which is not defined"), "{problems}");
        assert!(problems.contains("items and holds"), "{problems}");
    }

    #[test]
    fn the_sample_project_survives_a_rewrite() {
        let Some(bytes) = corpus("Raiuji Vtuber model2.cmo3") else { return };
        let first = from_cmo3(&bytes, "raiju").unwrap();
        let written = to_cmo3(&Cmo3Export {
            name: "raiju",
            model: &first.imported.model,
            textures: &first.imported.textures,
            param_names: &first.param_names,
            part_names: &first.imported.part_names,
            mesh_names: &first.imported.mesh_names,
            param_groups: &first.param_groups,
        })
        .unwrap();
        let second = from_cmo3(&written, "raiju").unwrap();
        let (a, b) = (&first.imported.model, &second.imported.model);
        assert_eq!(a.art_meshes.len(), b.art_meshes.len());
        assert_eq!(a.deformers.len(), b.deformers.len());
        assert_eq!(a.parameters.len(), b.parameters.len());
        assert_eq!(first.param_groups, second.param_groups);
        assert_eq!(first.param_names, second.param_names);
        assert_eq!(first.imported.mesh_names, second.imported.mesh_names);
        // blend, double sided and inverted mask: the six Multiply meshes
        // stay Multiply and every mesh keeps both sides
        use ayatsuri_format::model::flags;
        let mesh_flags = |m: &ayatsuri_format::model::Model| {
            m.art_meshes.iter().map(|x| x.flags).collect::<Vec<_>>()
        };
        assert_eq!(mesh_flags(a), mesh_flags(b));
        let multiply = a.art_meshes.iter().filter(|m| m.flags & flags::BLEND_MULTIPLICATIVE != 0);
        assert_eq!(multiply.count(), 6);
        assert!(a.art_meshes.iter().all(|m| m.flags & flags::DOUBLE_SIDED != 0));
        assert_eq!(
            a.physics.as_ref().map(|p| p.groups.len()),
            b.physics.as_ref().map(|p| p.groups.len())
        );
        assert_eq!(
            second.within.0, second.within.1,
            "every mesh rests where it did: {:?}",
            second.off
        );
        for d in &a.deformers {
            let e = b
                .deformers
                .iter()
                .find(|x| x.id == d.id)
                .unwrap_or_else(|| panic!("{} lost", d.id));
            assert_eq!(d.binding.axes.len(), e.binding.axes.len(), "{}", d.id);
            for (x, y) in d.binding.axes.iter().zip(&e.binding.axes) {
                assert_eq!(
                    a.parameters[x.parameter as usize].id,
                    b.parameters[y.parameter as usize].id
                );
                assert_eq!(x.keys, y.keys, "{}", d.id);
            }
            match (&d.kind, &e.kind) {
                (
                    ayatsuri_format::model::DeformerKind::Warp(p),
                    ayatsuri_format::model::DeformerKind::Warp(q),
                ) => {
                    assert_eq!(p.positions.len(), q.positions.len(), "{}", d.id);
                    let worst = p
                        .positions
                        .iter()
                        .zip(&q.positions)
                        .map(|(x, y)| (x - y).abs())
                        .fold(0.0, f32::max);
                    assert!(worst < 1e-2, "{}: warp keyforms drift by {worst}", d.id);
                }
                (
                    ayatsuri_format::model::DeformerKind::Rotation(p),
                    ayatsuri_format::model::DeformerKind::Rotation(q),
                ) => {
                    assert_eq!(p.angles, q.angles, "{}", d.id);
                    assert_eq!(p.scales, q.scales, "{}", d.id);
                }
                _ => panic!("{}: kind changed", d.id),
            }
        }
        for m in &a.art_meshes {
            let n = b
                .art_meshes
                .iter()
                .find(|x| x.id == m.id)
                .unwrap_or_else(|| panic!("{} lost", m.id));
            assert_eq!(m.vertex_count, n.vertex_count, "{}", m.id);
            assert_eq!(m.indices, n.indices, "{}", m.id);
            assert_eq!(m.positions.len(), n.positions.len(), "{}", m.id);
            assert_eq!(m.opacities, n.opacities, "{}", m.id);
            assert_eq!(m.masks.len(), n.masks.len(), "{}", m.id);
            let worst = m
                .positions
                .iter()
                .zip(&n.positions)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f32::max);
            assert!(worst < 1e-2, "{}: keyforms drift by {worst}", m.id);
        }
    }

    #[test]
    fn the_sample_project_comes_in_whole() {
        let Some(bytes) = corpus("Raiuji Vtuber model2.cmo3") else { return };
        let out = from_cmo3(&bytes, "raiju").unwrap();
        let m = &out.imported.model;
        assert_eq!((m.art_meshes.len(), m.deformers.len(), m.parameters.len()), (120, 70, 39));
        assert_eq!(m.physics.as_ref().map(|p| p.groups.len()), Some(5));
        assert_eq!(out.param_names.len(), m.parameters.len());
        assert_eq!(out.imported.mesh_names.len(), m.art_meshes.len());
        assert!(
            out.imported.mesh_names.iter().any(|n| n == "Neck base"),
            "names come from the project"
        );
        assert_eq!(out.param_groups.len(), 11);
        assert!(m.art_meshes.iter().any(|a| !a.masks.is_empty()), "clipping comes through");
    }

    /// The model the research pipeline produced on 2026-10-13 and checked
    /// by eye in the editor is the reference: this importer must build the
    /// same deformers, bindings, keyforms and meshes from the same project.
    #[test]
    fn matches_the_checked_reference_model() {
        let (Some(bytes), Some(aom)) = (corpus("Raiuji Vtuber model2.cmo3"), corpus("Raiju.aom"))
        else {
            return;
        };
        let ours = from_cmo3(&bytes, "raiju").unwrap().imported.model;
        let theirs = ayatsuri_format::read(&aom).unwrap().model;
        let parent = |m: &ayatsuri_format::model::Model, x: Option<u32>| {
            x.and_then(|i| m.deformers.get(i as usize)).map(|d| d.id.clone())
        };
        let keys = |b: &ayatsuri_format::model::Binding| {
            b.axes.iter().map(|a| (a.parameter, a.keys.clone())).collect::<Vec<_>>()
        };
        let close = |a: &[f32], b: &[f32]| {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
        };
        for d in &ours.deformers {
            let t = theirs
                .deformers
                .iter()
                .find(|x| x.id == d.id)
                .unwrap_or_else(|| panic!("{} missing", d.id));
            assert_eq!(
                parent(&ours, d.parent_deformer),
                parent(&theirs, t.parent_deformer),
                "{}",
                d.id
            );
            assert_eq!(keys(&d.binding), keys(&t.binding), "{}", d.id);
            match (&d.kind, &t.kind) {
                (DeformerKind::Warp(a), DeformerKind::Warp(b)) => {
                    assert_eq!((a.rows, a.cols), (b.rows, b.cols), "{}", d.id);
                    assert!(close(&a.positions, &b.positions), "{}", d.id);
                }
                (DeformerKind::Rotation(a), DeformerKind::Rotation(b)) => {
                    assert!(
                        close(&a.angles, &b.angles)
                            && close(&a.origins_x, &b.origins_x)
                            && close(&a.origins_y, &b.origins_y)
                            && close(&a.scales, &b.scales),
                        "{}",
                        d.id
                    );
                }
                _ => panic!("{}: kind differs", d.id),
            }
        }
        for m in &ours.art_meshes {
            let t = theirs
                .art_meshes
                .iter()
                .find(|x| x.id == m.id)
                .unwrap_or_else(|| panic!("{} missing", m.id));
            assert_eq!(
                parent(&ours, m.parent_deformer),
                parent(&theirs, t.parent_deformer),
                "{}",
                m.id
            );
            assert_eq!(keys(&m.binding), keys(&t.binding), "{}", m.id);
            assert!(close(&m.positions, &t.positions), "{}", m.id);
            assert_eq!(m.indices, t.indices, "{}", m.id);
            assert_eq!(m.masks.len(), t.masks.len(), "{}", m.id);
        }
    }

    #[test]
    fn garbage_is_refused_not_crashed() {
        assert!(from_cmo3(b"CAFF\0\0\0\0", "x").is_err());
        let mut junk = b"CAFF".to_vec();
        junk.extend_from_slice(&[0x5a; 200]);
        assert!(from_cmo3(&junk, "x").is_err());
    }
}
