// Parameter templates: a named set of parameters (ids, names, ranges) and
// the folders they file into, applied to a model in one step. The built-in
// ones are Live2D's published Standard Parameter List with its standard
// parameter groups; templates of your own are saved from a model's
// parameters as JSON in the config folder's `templates`. (Cubism's Model
// Template also carries parts, deformers and keyforms over; this is the
// parameter half of it.)

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::session::{ParamGroup, Session};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct Template {
    pub name: String,
    pub description: String,
    pub parameters: Vec<TemplateParam>,
    pub groups: Vec<TemplateGroup>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct TemplateParam {
    pub id: String,
    pub name: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
    pub blend_shape: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct TemplateGroup {
    pub name: String,
    /// The standard group id (ParamGroupFace...), empty for none.
    pub id: String,
    pub parameters: Vec<String>,
}

/// The published list: (group, id, name, min, default, max, optional).
/// Optional ones are those the list marks "input if necessary".
const STANDARD: &[(&str, &str, &str, f32, f32, f32, bool)] = &[
    ("Face", "ParamAngleX", "Angle X", -30.0, 0.0, 30.0, false),
    ("Face", "ParamAngleY", "Angle Y", -30.0, 0.0, 30.0, false),
    ("Face", "ParamAngleZ", "Angle Z", -30.0, 0.0, 30.0, false),
    ("Face", "ParamCheek", "Cheek", 0.0, 0.0, 1.0, false),
    ("Eyes", "ParamEyeLOpen", "Left eye Open/Close", 0.0, 1.0, 1.0, false),
    ("Eyes", "ParamEyeLSmile", "Left eye Smiling", 0.0, 0.0, 1.0, false),
    ("Eyes", "ParamEyeROpen", "Right eye Open/Close", 0.0, 1.0, 1.0, false),
    ("Eyes", "ParamEyeRSmile", "Right eye Smiling", 0.0, 0.0, 1.0, false),
    ("Eyeballs", "ParamEyeBallX", "Eyeball X", -1.0, 0.0, 1.0, false),
    ("Eyeballs", "ParamEyeBallY", "Eyeball Y", -1.0, 0.0, 1.0, false),
    ("Eyeballs", "ParamEyeBallForm", "Eyeball scaling", -1.0, 0.0, 1.0, true),
    ("Eyebrows", "ParamBrowLY", "Left eyebrow Up/Down", -1.0, 0.0, 1.0, false),
    ("Eyebrows", "ParamBrowRY", "Right eyebrow Up/Down", -1.0, 0.0, 1.0, false),
    ("Eyebrows", "ParamBrowLX", "Left eyebrow Left/Right", -1.0, 0.0, 1.0, false),
    ("Eyebrows", "ParamBrowRX", "Right eyebrow Left/Right", -1.0, 0.0, 1.0, false),
    ("Eyebrows", "ParamBrowLAngle", "Left eyebrow Angle", -1.0, 0.0, 1.0, false),
    ("Eyebrows", "ParamBrowRAngle", "Right eyebrow Angle", -1.0, 0.0, 1.0, false),
    ("Eyebrows", "ParamBrowLForm", "Left eyebrow Deformation", -1.0, 0.0, 1.0, false),
    ("Eyebrows", "ParamBrowRForm", "Right eyebrow Deformation", -1.0, 0.0, 1.0, false),
    ("Mouth", "ParamMouthForm", "Mouth Deformation", -1.0, 0.0, 1.0, false),
    ("Mouth", "ParamMouthOpenY", "Mouth Open/Close", 0.0, 0.0, 1.0, false),
    ("Body", "ParamBodyAngleX", "Body rotation X", -10.0, 0.0, 10.0, false),
    ("Body", "ParamBodyAngleY", "Body rotation Y", -10.0, 0.0, 10.0, false),
    ("Body", "ParamBodyAngleZ", "Body rotation Z", -10.0, 0.0, 10.0, false),
    ("Body", "ParamBreath", "Breath", 0.0, 0.0, 1.0, false),
    ("Body", "ParamShoulderY", "Shrug", -10.0, 0.0, 10.0, true),
    ("Body", "ParamBustX", "Breast physics X", -1.0, 0.0, 1.0, true),
    ("Body", "ParamBustY", "Breast physics Y", -1.0, 0.0, 1.0, true),
    ("Arms", "ParamArmLA", "Left arm A", -30.0, 0.0, 30.0, true),
    ("Arms", "ParamArmRA", "Right arm A", -30.0, 0.0, 30.0, true),
    ("Arms", "ParamArmLB", "Left arm B", -30.0, 0.0, 30.0, true),
    ("Arms", "ParamArmRB", "Right arm B", -30.0, 0.0, 30.0, true),
    ("Hands", "ParamHandL", "Left hand", -10.0, 0.0, 10.0, true),
    ("Hands", "ParamHandR", "Right hand", -10.0, 0.0, 10.0, true),
    ("Hair", "ParamHairFront", "Hair sway Front", -1.0, 0.0, 1.0, false),
    ("Hair", "ParamHairSide", "Hair sway Sideways", -1.0, 0.0, 1.0, false),
    ("Hair", "ParamHairBack", "Hair sway Back", -1.0, 0.0, 1.0, false),
    ("Hair", "ParamHairFluffy", "Hair sway Fluffy", -1.0, 0.0, 1.0, true),
    ("All", "ParamBaseX", "Overall Left/right", -10.0, 0.0, 10.0, true),
    ("All", "ParamBaseY", "Overall Up/down", -10.0, 0.0, 10.0, true),
];

/// The standard parameter group ids, by the names the list gives them.
const GROUP_IDS: &[(&str, &str)] = &[
    ("Face", "ParamGroupFace"),
    ("Eyes", "ParamGroupEyes"),
    ("Eyeballs", "ParamGroupEyeballs"),
    ("Eyebrows", "ParamGroupBrows"),
    ("Mouth", "ParamGroupMouth"),
    ("Body", "ParamGroupBody"),
    ("Arms", "ParamGroupArms"),
    ("Hands", "ParamGroupHands"),
    ("Hair", "ParamGroupHair"),
    ("All", "ParamGroupOverall"),
];

/// The built-in templates: the standard list without the optional
/// parameters, and all of it.
pub fn built_in() -> Vec<Template> {
    let make = |name: &str, description: &str, optional: bool| {
        let mut t =
            Template { name: name.into(), description: description.into(), ..Template::default() };
        for &(group, id, pname, min, default, max, opt) in STANDARD {
            if opt && !optional {
                continue;
            }
            t.parameters.push(TemplateParam {
                id: id.into(),
                name: pname.into(),
                min,
                default,
                max,
                blend_shape: false,
            });
            match t.groups.iter_mut().find(|g| g.name == group) {
                Some(g) => g.parameters.push(id.into()),
                None => t.groups.push(TemplateGroup {
                    name: group.into(),
                    id: GROUP_IDS
                        .iter()
                        .find(|(n, _)| *n == group)
                        .map(|(_, i)| (*i).to_string())
                        .unwrap_or_default(),
                    parameters: vec![id.into()],
                }),
            }
        }
        t
    };
    vec![
        make(
            "Standard parameters",
            "Live2D's Standard Parameter List: face, eyes, brows, mouth, body, breath and hair \
             sway, filed into the standard groups. The ids face trackers and VTube Studio expect.",
            false,
        ),
        make(
            "Standard parameters (all)",
            "The whole Standard Parameter List, the optional ones too: eyeball scaling, \
             shoulders, arms, hands, fluffy hair, breast physics and overall movement.",
            true,
        ),
    ]
}

/// Where templates of your own are kept.
pub fn dir() -> Option<PathBuf> {
    Some(ayatsuri_update::config_dir()?.join("templates"))
}

/// The templates in the folder, sorted by name; a file that won't read is
/// logged and skipped.
pub fn saved() -> Vec<(PathBuf, Template)> {
    let Some(dir) = dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<(PathBuf, Template)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".params.json"))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            match serde_json::from_str::<Template>(&text) {
                Ok(t) => Some((p, t)),
                Err(e) => {
                    log::warn!("template {}: {e}", p.display());
                    None
                }
            }
        })
        .collect();
    out.sort_by_key(|a| a.1.name.to_lowercase());
    out
}

/// Write a template to the folder as `<name>.params.json`.
pub fn save(t: &Template) -> Result<PathBuf, String> {
    let dir = dir().ok_or("no config folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem: String = t
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let stem = if stem.is_empty() { "template".to_string() } else { stem };
    let path = dir.join(format!("{stem}.params.json"));
    let text = serde_json::to_string_pretty(t).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

/// A template from a model's parameters as they are: names, ranges and the
/// folders they sit in.
pub fn from_session(s: &Session, name: &str, description: &str) -> Template {
    let model = s.puppet.model();
    let parameters = model
        .parameters
        .iter()
        .enumerate()
        .map(|(i, p)| TemplateParam {
            id: p.id.clone(),
            name: s.param_names.get(i).cloned().unwrap_or_else(|| p.id.clone()),
            min: p.min,
            default: p.default,
            max: p.max,
            blend_shape: p.blend_shape,
        })
        .collect();
    let groups = s
        .groups
        .iter()
        .map(|g| TemplateGroup {
            name: g.name.clone(),
            id: String::new(),
            parameters: g
                .params
                .iter()
                .filter_map(|&pi| model.parameters.get(pi).map(|p| p.id.clone()))
                .collect(),
        })
        .filter(|g| !g.parameters.is_empty())
        .collect();
    Template { name: name.into(), description: description.into(), parameters, groups }
}

/// Add the template's parameters the model lacks (by id) as one undo step
/// and file each into its folder, made when missing. Returns how many were
/// added.
pub fn apply(s: &mut Session, t: &Template) -> usize {
    let have: std::collections::HashSet<String> =
        s.puppet.model().parameters.iter().map(|p| p.id.clone()).collect();
    let new: Vec<&TemplateParam> = t.parameters.iter().filter(|p| !have.contains(&p.id)).collect();
    if new.is_empty() {
        return 0;
    }
    let first = s.puppet.model().parameters.len();
    let made = s.edit(|m| {
        let mut any = false;
        for p in &new {
            let (min, max) = (p.min.min(p.max), p.min.max(p.max));
            let made = ayatsuri_core::edit::create_parameter(
                m,
                p.id.clone(),
                min,
                p.default.clamp(min, max),
                max,
            );
            if let Some(q) = made.and_then(|i| m.parameters.get_mut(i)) {
                q.blend_shape = p.blend_shape;
                any = true;
            }
        }
        any
    });
    if !made {
        return 0;
    }
    let model = s.puppet.model();
    let added: Vec<(usize, String)> =
        model.parameters.iter().enumerate().skip(first).map(|(i, p)| (i, p.id.clone())).collect();
    for (_, id) in &added {
        let name = new.iter().find(|p| &p.id == id).map(|p| p.name.clone()).unwrap_or_default();
        s.param_names.push(if name.is_empty() { id.clone() } else { name });
        s.param_colors.push(None);
    }
    for (pi, id) in &added {
        let group = t.groups.iter().find(|g| g.parameters.contains(id));
        let name = group.map(|g| g.name.clone()).unwrap_or_default();
        let slot = match s.groups.iter().position(|g| g.name == name) {
            Some(i) => i,
            None => {
                s.groups.push(ParamGroup { name, params: Vec::new(), open: true, color: None });
                s.groups.len() - 1
            }
        };
        if let Some(g) = s.groups.get_mut(slot) {
            g.params.push(*pi);
        }
    }
    added.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_templates_hold_together() {
        let all = built_in();
        assert_eq!(all.len(), 2);
        for t in &all {
            let ids: std::collections::HashSet<&str> =
                t.parameters.iter().map(|p| p.id.as_str()).collect();
            assert_eq!(ids.len(), t.parameters.len(), "{}: ids repeat", t.name);
            for g in &t.groups {
                assert!(!g.id.is_empty(), "{}: group {} has no standard id", t.name, g.name);
                assert!(g.parameters.iter().all(|p| ids.contains(p.as_str())));
            }
            for p in &t.parameters {
                assert!(p.min <= p.default && p.default <= p.max, "{}", p.id);
            }
        }
        // the optional ones only in the full set
        assert!(all[0].parameters.iter().all(|p| p.id != "ParamArmLA"));
        assert!(all[1].parameters.iter().any(|p| p.id == "ParamArmLA"));
        assert!(all[0].parameters.iter().any(|p| p.id == "ParamBreath"));
    }

    #[test]
    fn a_template_reads_back_as_written() {
        let t = built_in().remove(0);
        let text = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Template>(&text).unwrap(), t);
    }
}
