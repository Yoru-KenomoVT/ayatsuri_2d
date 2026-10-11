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

    let model3 = json!({
        "Version": 3,
        "FileReferences": refs,
        "Groups": [],
    });
    write(
        &dir.join(format!("{name}.model3.json")),
        &serde_json::to_vec_pretty(&model3).map_err(|e| e.to_string())?,
        &mut written,
    )?;
    Ok(written)
}
