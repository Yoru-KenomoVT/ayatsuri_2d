// UI translations. Every piece of interface text goes through `tr` (or `trf`
// when it carries values), keyed by its English wording: English needs no
// file, and a string a language has not translated yet shows in English.
//
// Built-in languages live in assets/i18n/<code>.json and ship inside the
// app. Dropping another <code>.json into the settings folder's i18n/
// adds a language without rebuilding; a file there with a built-in code
// replaces that language, so a translator can try fixes in place.
//
// A language file is a flat JSON object, English text to translation, plus
// "_name" (the language's own name, shown in Settings) and optionally
// "_note". Placeholders are written {name}, the same in every language.

use std::collections::HashMap;
use std::sync::RwLock;

/// The languages built into the app: code, own name, file.
const BUILT_IN: &[(&str, &str, &str)] = &[
    ("af", "Afrikaans", include_str!("../../../assets/i18n/af.json")),
    ("nl", "Nederlands", include_str!("../../../assets/i18n/nl.json")),
    ("ja", "日本語", include_str!("../../../assets/i18n/ja.json")),
    ("ko", "한국어", include_str!("../../../assets/i18n/ko.json")),
    ("zh-CN", "简体中文", include_str!("../../../assets/i18n/zh-CN.json")),
];

/// The table in use: English text to translation. Leaked once per language
/// change so lookups can hand out `&'static str` like the literals they
/// replace; switching languages is rare enough for that to be free.
static ACTIVE: RwLock<Option<&'static HashMap<String, String>>> = RwLock::new(None);
static ACTIVE_CODE: RwLock<String> = RwLock::new(String::new());

/// A language on offer: its code and its own name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
    pub code: String,
    pub name: String,
}

fn user_dir() -> Option<std::path::PathBuf> {
    ayatsuri_update::config_dir().map(|d| d.join("i18n"))
}

/// English plus every built-in language and every file in the user folder.
pub fn languages() -> Vec<Language> {
    let mut out = vec![Language { code: "en".into(), name: "English".into() }];
    for (code, name, _) in BUILT_IN {
        out.push(Language { code: (*code).into(), name: (*name).into() });
    }
    if let Some(dir) = user_dir()
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for path in entries.flatten().map(|e| e.path()) {
            let Some(code) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".json"))
                .map(str::to_string)
            else {
                continue;
            };
            if out.iter().any(|l| l.code == code) {
                continue;
            }
            let name = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|v| v.get("_name").and_then(|n| n.as_str()).map(str::to_string))
                .unwrap_or_else(|| code.clone());
            out.push(Language { code, name });
        }
    }
    out
}

/// The operating system's language as one of ours: "ja-JP" finds "ja",
/// "zh-Hans-CN" finds "zh-CN"; anything else is English.
pub fn system_language() -> String {
    let have = languages();
    let wants = sys_locale::get_locales().collect::<Vec<_>>();
    for want in wants {
        let want = want.replace('_', "-");
        let lower = want.to_ascii_lowercase();
        if let Some(l) = have.iter().find(|l| l.code.eq_ignore_ascii_case(&want)) {
            return l.code.clone();
        }
        // Chinese: simplified scripts and mainland regions take zh-CN
        if lower.starts_with("zh") {
            if lower.contains("hant") || lower.ends_with("-tw") || lower.ends_with("-hk") {
                if let Some(l) = have.iter().find(|l| l.code.eq_ignore_ascii_case("zh-TW")) {
                    return l.code.clone();
                }
            } else if have.iter().any(|l| l.code == "zh-CN") {
                return "zh-CN".into();
            }
        }
        let base = lower.split('-').next().unwrap_or_default().to_string();
        if let Some(l) = have.iter().find(|l| l.code.to_ascii_lowercase() == base) {
            return l.code.clone();
        }
    }
    "en".into()
}

fn table_for(code: &str) -> Option<HashMap<String, String>> {
    let user = user_dir().map(|d| d.join(format!("{code}.json")));
    let text = user.and_then(|p| std::fs::read_to_string(p).ok()).or_else(|| {
        BUILT_IN.iter().find(|(c, _, _)| *c == code).map(|(_, _, t)| (*t).to_string())
    })?;
    let map: HashMap<String, serde_json::Value> = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("language {code}: {e}");
            return None;
        }
    };
    Some(
        map.into_iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .filter_map(|(k, v)| v.as_str().filter(|s| !s.is_empty()).map(|s| (k, s.to_string())))
            .collect(),
    )
}

/// Switch the interface language; "" or "system" follows the OS.
pub fn set_language(choice: &str) {
    let code = if choice.is_empty() || choice == "system" {
        // tests read English text back, whatever the machine's language
        if cfg!(test) { "en".to_string() } else { system_language() }
    } else {
        choice.to_string()
    };
    let table: Option<&'static HashMap<String, String>> =
        if code == "en" { None } else { table_for(&code).map(|t| &*Box::leak(Box::new(t))) };
    if let Ok(mut a) = ACTIVE.write() {
        *a = table;
    }
    if let Ok(mut c) = ACTIVE_CODE.write() {
        *c = code.clone();
    }
    log::info!("interface language: {code}");
}

/// The language in use, after "system" was worked out.
pub fn current() -> String {
    ACTIVE_CODE
        .read()
        .map(|c| if c.is_empty() { "en".to_string() } else { c.clone() })
        .unwrap_or_default()
}

/// Interface text in the current language.
pub fn tr(en: &'static str) -> &'static str {
    let table = ACTIVE.read().ok().and_then(|a| *a);
    match table.and_then(|t| t.get(en)) {
        Some(s) => s.as_str(),
        None => en,
    }
}

/// Interface text whose English is ambiguous, keyed with its context
/// (`"blend|Add"`): the translation of the whole key, or the English after
/// the bar.
pub fn trc(key: &'static str) -> &'static str {
    let table = ACTIVE.read().ok().and_then(|a| *a);
    match table.and_then(|t| t.get(key)) {
        Some(s) => s.as_str(),
        None => key.split_once('|').map_or(key, |(_, en)| en),
    }
}

/// Interface text with values: {name} placeholders filled in.
pub fn trf(en: &'static str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = tr(en).to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_files_are_valid_json_with_a_name() {
        for (code, name, text) in BUILT_IN {
            let v: serde_json::Value =
                serde_json::from_str(text).unwrap_or_else(|e| panic!("{code}: {e}"));
            assert_eq!(v.get("_name").and_then(|n| n.as_str()), Some(*name), "{code}");
        }
    }

    #[test]
    fn placeholders_fill_and_missing_text_stays_english() {
        set_language("en");
        assert_eq!(tr("Nothing translated here"), "Nothing translated here");
        assert_eq!(trf("{count} meshes selected", &[("count", &3)]), "3 meshes selected");
    }
}
