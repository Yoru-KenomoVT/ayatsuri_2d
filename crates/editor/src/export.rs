// The embedded export: everything a Cubism-compatible runtime loads, laid
// out the way those runtimes expect. The .aom keeps all of this inside one
// file; this is the spread-out form for other people's players.
//
//     <dir>/<name>.moc3
//     <dir>/<name>.model3.json
//     <dir>/<name>.physics3.json      when the model has physics
//     <dir>/<name>.cdi3.json          display names and parameter folders
//     <dir>/<name>.textures/texture_00.png ...
//     <dir>/motions/<clip>.motion3.json

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::session::Session;

pub struct Options {
    pub name: String,
    pub version: ayatsuri_import::ExportVersion,
    pub physics: bool,
    pub motions: bool,
    pub expressions: bool,
}

/// File names keep to what every filesystem accepts.
fn safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if cleaned.is_empty() { "model".into() } else { cleaned }
}

/// The models already in a folder: its `*.model3.json` files.
pub fn existing_models(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.to_string_lossy().ends_with(".model3.json"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// A path a model3.json names, if it stays inside `dir` (no absolute paths,
/// no `..`): an export folder's files are the only ones ever removed.
fn inside(dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    let safe = rel.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    (safe && !rel.as_os_str().is_empty()).then(|| dir.join(rel))
}

/// Make a folder ready for a fresh export over an earlier one: for every
/// model3.json in it, remove the files it lists (moc3, textures, physics,
/// display info, pose, user data, every motion and expression) and the
/// model3.json itself, then any folder those leave empty. Files it does not
/// list stay: VTube Studio's settings, items, icons, the user's own files,
/// and motion sounds, which a fresh export does not write. Returns how many
/// files went.
pub fn clear_previous(dir: &Path) -> Result<usize, String> {
    let mut removed = 0;
    let mut folders: Vec<PathBuf> = Vec::new();
    for model in existing_models(dir) {
        let text =
            std::fs::read_to_string(&model).map_err(|e| format!("{}: {e}", model.display()))?;
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", model.display()))?;
        let none = serde_json::Value::Null;
        let refs = json.get("FileReferences").unwrap_or(&none);
        let mut listed: Vec<String> = Vec::new();
        for key in ["Moc", "Physics", "DisplayInfo", "Pose", "UserData"] {
            if let Some(f) = refs.get(key).and_then(|v| v.as_str()) {
                listed.push(f.to_string());
            }
        }
        for t in refs.get("Textures").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(f) = t.as_str() {
                listed.push(f.to_string());
            }
        }
        for group in
            refs.get("Motions").and_then(|v| v.as_object()).into_iter().flat_map(|o| o.values())
        {
            for m in group.as_array().into_iter().flatten() {
                if let Some(f) = m.get("File").and_then(|v| v.as_str()) {
                    listed.push(f.to_string());
                }
            }
        }
        for e in refs.get("Expressions").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(f) = e.get("File").and_then(|v| v.as_str()) {
                listed.push(f.to_string());
            }
        }
        for rel in listed {
            let Some(path) = inside(dir, &rel) else { continue };
            if !path.is_file() {
                continue;
            }
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("could not remove {} of the previous export: {e}", path.display());
            } else {
                removed += 1;
                if let Some(parent) = path.parent()
                    && parent != dir
                    && !folders.iter().any(|f| f == parent)
                {
                    folders.push(parent.to_path_buf());
                }
            }
        }
        std::fs::remove_file(&model).map_err(|e| format!("{}: {e}", model.display()))?;
        removed += 1;
    }
    // folders the old export made, deepest first, once they are empty
    folders.sort_by_key(|f| std::cmp::Reverse(f.components().count()));
    for f in folders {
        // a folder that still holds the user's own files stays
        if let Err(e) = std::fs::remove_dir(&f) {
            log::debug!("kept {}: {e}", f.display());
        }
    }
    Ok(removed)
}

fn write(path: &Path, bytes: &[u8], written: &mut Vec<PathBuf>) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    written.push(path.to_path_buf());
    Ok(())
}

pub fn export(s: &Session, dir: &Path, opt: &Options) -> Result<Vec<PathBuf>, String> {
    let name = safe(&opt.name);
    let model = s.puppet.model();
    let mut written = Vec::new();

    let moc = ayatsuri_import::to_moc3(model, opt.version).map_err(|e| e.to_string())?;
    write(&dir.join(format!("{name}.moc3")), &moc, &mut written)?;

    // textures, in slot order
    let tex_dir_name = format!("{name}.textures");
    let tex_dir = dir.join(&tex_dir_name);
    std::fs::create_dir_all(&tex_dir).map_err(|e| format!("{}: {e}", tex_dir.display()))?;
    let mut tex_refs = Vec::new();
    for (i, png) in s.texture_pngs.iter().enumerate() {
        let file = format!("texture_{i:02}.png");
        write(&tex_dir.join(&file), png, &mut written)?;
        tex_refs.push(format!("{tex_dir_name}/{file}"));
    }

    let mut refs = serde_json::Map::new();
    refs.insert("Moc".into(), json!(format!("{name}.moc3")));
    refs.insert("Textures".into(), json!(tex_refs));

    if opt.physics
        && let Some(rig) = &model.physics
    {
        let file = format!("{name}.physics3.json");
        write(&dir.join(&file), &ayatsuri_import::to_physics3(rig), &mut written)?;
        refs.insert("Physics".into(), json!(file));
    }

    // display info, so other editors show the names riggers gave things
    let cdi = json!({
        "Version": 3,
        "Parameters": model.parameters.iter().enumerate().map(|(i, p)| {
            let group = s
                .groups
                .iter()
                .position(|g| g.params.contains(&i))
                .map(|gi| format!("ParamGroup{gi}"))
                .unwrap_or_default();
            json!({
                "Id": p.id,
                "GroupId": group,
                "Name": s.param_names.get(i).cloned().unwrap_or_else(|| p.id.clone()),
            })
        }).collect::<Vec<_>>(),
        "ParameterGroups": s.groups.iter().enumerate().filter(|(_, g)| !g.name.is_empty()).map(|(gi, g)| json!({
            "Id": format!("ParamGroup{gi}"),
            "GroupId": "",
            "Name": g.name,
        })).collect::<Vec<_>>(),
        "Parts": model.parts.iter().enumerate().map(|(i, p)| json!({
            "Id": p.id,
            "Name": s.part_names.get(i).cloned().unwrap_or_else(|| p.id.clone()),
        })).collect::<Vec<_>>(),
        "CombinedParameters": s.combined.iter().filter_map(|&(a, b)| {
            Some(json!([model.parameters.get(a)?.id, model.parameters.get(b)?.id]))
        }).collect::<Vec<_>>(),
    });
    let cdi_file = format!("{name}.cdi3.json");
    write(&dir.join(&cdi_file), &ayatsuri_import::cubism_json(&cdi), &mut written)?;
    refs.insert("DisplayInfo".into(), json!(cdi_file));

    // expressions go in their own folder, as published models have them, and
    // are listed before the motions in the model3.json, Cubism's order
    if opt.expressions && !s.expressions.is_empty() {
        let edir = dir.join("expressions");
        std::fs::create_dir_all(&edir).map_err(|e| format!("{}: {e}", edir.display()))?;
        let mut list = Vec::new();
        let mut used: Vec<String> = Vec::new();
        for e in &s.expressions {
            // two expressions whose names clean up the same get numbered
            let base = safe(if e.name.is_empty() { "expression" } else { &e.name });
            let mut file = format!("{base}.exp3.json");
            let mut n = 2;
            while used.contains(&file) {
                file = format!("{base}_{n}.exp3.json");
                n += 1;
            }
            used.push(file.clone());
            write(&edir.join(&file), &ayatsuri_import::to_exp3(e), &mut written)?;
            list.push(json!({ "Name": e.name, "File": format!("expressions/{file}") }));
        }
        refs.insert("Expressions".into(), json!(list));
    }

    if opt.motions && (!s.animations.is_empty() || !s.compositions.is_empty()) {
        let mdir = dir.join("motions");
        std::fs::create_dir_all(&mdir).map_err(|e| format!("{}: {e}", mdir.display()))?;
        let mut group = Vec::new();
        for anim in &s.animations {
            let file = format!("{}.motion3.json", safe(&anim.name));
            write(&mdir.join(&file), &ayatsuri_import::to_motion3(anim), &mut written)?;
            group.push(json!({ "File": format!("motions/{file}") }));
        }
        // compositions go out baked: a key every frame, flat like a
        // rendered precomp
        for comp in &s.compositions {
            let baked = s.bake_comp(comp);
            let file = format!("{}.motion3.json", safe(&comp.name));
            write(&mdir.join(&file), &ayatsuri_import::to_motion3(&baked), &mut written)?;
            group.push(json!({ "File": format!("motions/{file}") }));
        }
        refs.insert("Motions".into(), json!({ "Idle": group }));
    }

    let groups: Vec<serde_json::Value> = model_groups(s)
        .into_iter()
        .map(|(target, name, ids)| json!({ "Target": target, "Name": name, "Ids": ids }))
        .collect();
    let model3 = json!({
        "Version": 3,
        "FileReferences": refs,
        "Groups": groups,
    });
    write(
        &dir.join(format!("{name}.model3.json")),
        &ayatsuri_import::cubism_json(&model3),
        &mut written,
    )?;
    Ok(written)
}

/// The model3.json Groups: the ones the model came with, else the two
/// every Cubism export carries, filled from the standard parameter ids
/// when the model has them (apps drive auto blink and lip sync off these).
fn model_groups(s: &Session) -> Vec<(String, String, Vec<String>)> {
    if let Some(g) = &s.model_groups {
        return g.clone();
    }
    let has = |id: &str| s.puppet.model().parameters.iter().any(|p| p.id == id);
    let pick = |ids: &[&str]| ids.iter().filter(|id| has(id)).map(|id| id.to_string()).collect();
    vec![
        ("Parameter".into(), "EyeBlink".into(), pick(&["ParamEyeLOpen", "ParamEyeROpen"])),
        ("Parameter".into(), "LipSync".into(), pick(&["ParamMouthOpenY"])),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(ids: &[&str]) -> Session {
        let mut model = ayatsuri_format::Model::new();
        for id in ids {
            model.parameters.push(ayatsuri_format::model::Parameter {
                id: (*id).into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                repeat: false,
                decimals: 3,
            });
        }
        Session::create(&egui::Context::default(), "t".into(), String::new(), model).unwrap()
    }

    #[test]
    fn standard_groups_fill_from_known_ids() {
        let s = session(&["ParamEyeLOpen", "ParamEyeROpen", "ParamMouthOpenY", "ParamOther"]);
        let g = model_groups(&s);
        assert_eq!(g[0].1, "EyeBlink");
        assert_eq!(g[0].2, vec!["ParamEyeLOpen", "ParamEyeROpen"]);
        assert_eq!(g[1].1, "LipSync");
        assert_eq!(g[1].2, vec!["ParamMouthOpenY"]);
        // a model without them still gets both groups, just empty
        let bare = model_groups(&session(&["ParamOther"]));
        assert!(bare.iter().all(|(_, _, ids)| ids.is_empty()) && bare.len() == 2);
    }

    #[test]
    fn expressions_export_and_read_back() {
        let mut s = session(&["ParamEyeLOpen", "ParamSmile"]);
        let smile = ayatsuri_format::Expression {
            name: "Smile".into(),
            fade_in: Some(0.5),
            fade_out: None,
            params: vec![
                ayatsuri_format::ExpressionParam {
                    id: "ParamSmile".into(),
                    value: 1.0,
                    blend: ayatsuri_format::ExpressionBlend::Add,
                },
                ayatsuri_format::ExpressionParam {
                    id: "ParamEyeLOpen".into(),
                    value: 0.5,
                    blend: ayatsuri_format::ExpressionBlend::Multiply,
                },
            ],
        };
        // a second expression with the same name still gets its own file
        s.expressions = vec![smile.clone(), smile.clone()];
        s.animations = vec![ayatsuri_format::Animation {
            name: "idle".into(),
            fps: 30.0,
            duration: 1.0,
            looped: true,
            ..ayatsuri_format::Animation::default()
        }];
        let dir = std::env::temp_dir().join(format!("ayatsuri-exp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let opt = Options {
            name: "t".into(),
            version: ayatsuri_import::ExportVersion::DEFAULT,
            physics: false,
            motions: true,
            expressions: true,
        };
        export(&s, &dir, &opt).unwrap();
        let model3 = std::fs::read_to_string(dir.join("t.model3.json")).unwrap();
        // listed, and before Motions as Cubism orders them
        let (e, m) = (model3.find("\"Expressions\"").unwrap(), model3.find("\"Motions\"").unwrap());
        assert!(e < m, "{model3}");
        assert!(model3.contains("expressions/Smile.exp3.json"), "{model3}");
        assert!(model3.contains("expressions/Smile_2.exp3.json"), "{model3}");
        let back = ayatsuri_import::from_exp3(
            &std::fs::read(dir.join("expressions/Smile.exp3.json")).unwrap(),
            "Smile",
        )
        .unwrap();
        assert_eq!(back, smile);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn imported_groups_win() {
        let mut s = session(&["ParamEyeLOpen", "ParamMouthOpenY"]);
        let original = vec![
            ("Parameter".to_string(), "EyeBlink".to_string(), vec!["ParamEyeLOpen".to_string()]),
            ("Parameter".to_string(), "LipSync".to_string(), Vec::new()),
        ];
        s.model_groups = Some(original.clone());
        assert_eq!(model_groups(&s), original);
    }
}

#[cfg(test)]
mod overwrite_tests {
    use super::*;

    #[test]
    fn clearing_removes_only_what_the_old_model_listed() {
        let dir = std::env::temp_dir().join(format!("ay-export-overwrite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["ariu.4096", "motions", "expressions"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let files = [
            "ariu.moc3",
            "ariu.physics3.json",
            "ariu.cdi3.json",
            "ariu.4096/texture_00.png",
            "motions/idle.motion3.json",
            "motions/idle.wav",
            "expressions/smile.exp3.json",
            "ariu.vtube.json",
            "items_pinned_to_model.json",
        ];
        for f in files {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        // something outside the folder that a hostile path points at
        let outside = dir.with_extension("outside");
        std::fs::write(&outside, b"keep").unwrap();
        let model = serde_json::json!({
            "Version": 3,
            "FileReferences": {
                "Moc": "ariu.moc3",
                "Textures": ["ariu.4096/texture_00.png"],
                "Physics": "ariu.physics3.json",
                "DisplayInfo": "ariu.cdi3.json",
                "UserData": format!("../{}", outside.file_name().unwrap().to_string_lossy()),
                "Motions": {"Idle": [{"File": "motions/idle.motion3.json", "Sound": "motions/idle.wav"}]},
                "Expressions": [{"Name": "smile", "File": "expressions/smile.exp3.json"}]
            }
        });
        std::fs::write(dir.join("ariu.model3.json"), model.to_string()).unwrap();
        assert_eq!(existing_models(&dir).len(), 1);

        let removed = clear_previous(&dir).unwrap();
        assert_eq!(removed, 7, "6 listed files and the model3.json");
        assert!(existing_models(&dir).is_empty());
        for gone in ["ariu.moc3", "ariu.4096", "expressions", "motions/idle.motion3.json"] {
            assert!(!dir.join(gone).exists(), "{gone} should be gone");
        }
        // not ours: VTube Studio's settings, items, the motion's sound
        for kept in ["ariu.vtube.json", "items_pinned_to_model.json", "motions/idle.wav"] {
            assert!(dir.join(kept).exists(), "{kept} should stay");
        }
        assert!(outside.exists(), "a path out of the folder is never followed");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }
}
