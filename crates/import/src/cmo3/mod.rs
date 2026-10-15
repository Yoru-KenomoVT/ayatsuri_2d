//! Cubism Editor projects (`.cmo3`) as full models: art, meshes, parts, the
//! deformer tree, parameters with their folders, every keyform, physics
//! and draw order. Worked out from project files (see the research notes);
//! coordinates follow moc3's conventions, so the same conversions apply.

mod build;
mod caff;
mod extract;
mod xml;

pub use build::Cmo3;

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
