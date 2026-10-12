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
}

/// File names keep to what every filesystem accepts.
fn safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if cleaned.is_empty() { "model".into() } else { cleaned }
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
    write(
        &dir.join(&cdi_file),
        &serde_json::to_vec_pretty(&cdi).map_err(|e| e.to_string())?,
        &mut written,
    )?;
    refs.insert("DisplayInfo".into(), json!(cdi_file));

    if opt.motions && !s.animations.is_empty() {
        let mdir = dir.join("motions");
        std::fs::create_dir_all(&mdir).map_err(|e| format!("{}: {e}", mdir.display()))?;
        let mut group = Vec::new();
        for anim in &s.animations {
            let file = format!("{}.motion3.json", safe(&anim.name));
            write(&mdir.join(&file), &ayatsuri_import::to_motion3(anim), &mut written)?;
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
        &serde_json::to_vec_pretty(&model3).map_err(|e| e.to_string())?,
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
