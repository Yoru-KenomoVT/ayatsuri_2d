// exp3.json, the published expression format (CubismSpecs): a type tag,
// optional fade times in seconds, and parameters with a value and a blend.
// Read leniently, since real files vary: a missing blend is Add, as the
// spec says, and an unknown one is read as Add with a warning rather than
// losing the whole expression.

use ayatsuri_format::{Expression, ExpressionBlend, ExpressionParam};
use serde::Deserialize;

use crate::ImportError;

#[derive(Deserialize)]
struct File {
    #[serde(rename = "FadeInTime", default)]
    fade_in: Option<f32>,
    #[serde(rename = "FadeOutTime", default)]
    fade_out: Option<f32>,
    #[serde(rename = "Parameters", default)]
    parameters: Vec<Param>,
}

#[derive(Deserialize)]
struct Param {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Value", default)]
    value: f32,
    #[serde(rename = "Blend", default)]
    blend: Option<String>,
}

/// Parse an exp3.json; `name` is what the model3.json calls it, or the
/// file name when nothing does.
pub fn from_exp3(bytes: &[u8], name: &str) -> Result<Expression, ImportError> {
    let text = core::str::from_utf8(bytes).map_err(|e| ImportError::Expression(e.to_string()))?;
    // some tools write a byte order mark
    let text = text.trim_start_matches('\u{feff}');
    let file: File =
        serde_json::from_str(text).map_err(|e| ImportError::Expression(e.to_string()))?;
    let params = file
        .parameters
        .into_iter()
        .filter(|p| !p.id.is_empty() && p.value.is_finite())
        .map(|p| ExpressionParam {
            blend: match p.blend.as_deref() {
                None | Some("Add") => ExpressionBlend::Add,
                Some("Multiply") => ExpressionBlend::Multiply,
                Some("Overwrite") => ExpressionBlend::Overwrite,
                Some(other) => {
                    log::warn!("{name}: unknown blend {other:?} on {}, read as Add", p.id);
                    ExpressionBlend::Add
                }
            },
            id: p.id,
            value: p.value,
        })
        .collect();
    let fade = |f: Option<f32>| f.filter(|v| v.is_finite() && *v >= 0.0);
    Ok(Expression {
        name: name.to_string(),
        fade_in: fade(file.fade_in),
        fade_out: fade(file.fade_out),
        params,
    })
}

/// Write an expression as exp3.json, in the spec's key order.
pub fn to_exp3(e: &Expression) -> Vec<u8> {
    let mut doc = serde_json::Map::new();
    doc.insert("Type".into(), "Live2D Expression".into());
    if let Some(t) = e.fade_in {
        doc.insert("FadeInTime".into(), t.into());
    }
    if let Some(t) = e.fade_out {
        doc.insert("FadeOutTime".into(), t.into());
    }
    let params: Vec<serde_json::Value> = e
        .params
        .iter()
        .map(|p| {
            serde_json::json!({
                "Id": p.id,
                "Value": p.value,
                "Blend": match p.blend {
                    ExpressionBlend::Add => "Add",
                    ExpressionBlend::Multiply => "Multiply",
                    ExpressionBlend::Overwrite => "Overwrite",
                },
            })
        })
        .collect();
    doc.insert("Parameters".into(), params.into());
    crate::cubism_json(&serde_json::Value::Object(doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    // the shape of a real file (021's aozame), with a fade and a Multiply
    const SAMPLE: &str = r#"{
	"Type": "Live2D Expression",
	"FadeInTime": 0.5,
	"Parameters": [
		{ "Id": "ParamAozame", "Value": 1, "Blend": "Add" },
		{ "Id": "ParamEyeLOpen", "Value": 0.6, "Blend": "Multiply" },
		{ "Id": "ParamOh", "Value": 1 }
	]
}"#;

    #[test]
    fn reads_values_blends_and_fades() {
        let e = from_exp3(SAMPLE.as_bytes(), "aozame").unwrap();
        assert_eq!(e.name, "aozame");
        assert_eq!(e.fade_in, Some(0.5));
        assert_eq!(e.fade_out, None);
        assert_eq!(e.params.len(), 3);
        assert_eq!(e.params[1].blend, ExpressionBlend::Multiply);
        // no Blend means Add, per the spec
        assert_eq!(e.params[2].blend, ExpressionBlend::Add);
    }

    #[test]
    fn round_trips_through_our_writer() {
        let e = from_exp3(SAMPLE.as_bytes(), "aozame").unwrap();
        let text = String::from_utf8(to_exp3(&e)).unwrap();
        assert!(text.starts_with("{\n\t\"Type\": \"Live2D Expression\""), "{text}");
        // whole numbers stay integers, like Cubism writes them
        assert!(text.contains("\"Value\": 1,"), "{text}");
        assert!(!text.contains("FadeOutTime"), "{text}");
        assert_eq!(from_exp3(text.as_bytes(), "aozame").unwrap(), e);
    }

    #[test]
    fn hostile_input_is_an_error_not_a_crash() {
        assert!(from_exp3(b"\xff\xfe", "x").is_err());
        assert!(from_exp3(b"[1,2,3]", "x").is_err());
        let odd = from_exp3(
            br#"{"Parameters":[{"Id":"","Value":1},{"Id":"A","Value":2,"Blend":"Screen"}]}"#,
            "x",
        )
        .unwrap();
        // the nameless entry is dropped, the unknown blend reads as Add
        assert_eq!(odd.params.len(), 1);
        assert_eq!(odd.params[0].blend, ExpressionBlend::Add);
    }
}
