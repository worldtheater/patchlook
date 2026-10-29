use gpui_kit::component;
use patchlook::{git::Area, preferences::Language};
use std::{collections::HashMap, sync::OnceLock};

const RESOURCES: [(&str, &str); 9] = [
    ("en", include_str!("../locales/en.json")),
    ("zh-CN", include_str!("../locales/zh-CN.json")),
    ("zh-TW", include_str!("../locales/zh-TW.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("ko", include_str!("../locales/ko.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("de", include_str!("../locales/de.json")),
    ("es", include_str!("../locales/es.json")),
    ("pt-BR", include_str!("../locales/pt-BR.json")),
];

fn catalog(locale: &str) -> &'static HashMap<String, String> {
    static STRINGS: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();
    STRINGS
        .get_or_init(|| {
            RESOURCES
                .into_iter()
                .map(|(code, json)| {
                    (
                        code,
                        serde_json::from_str(json).expect("The bundled locale must be valid JSON"),
                    )
                })
                .collect()
        })
        .get(locale)
        .unwrap_or_else(|| panic!("Unsupported locale: {locale}"))
}

pub fn tr(key: &str) -> &'static str {
    catalog(&component::locale())
        .get(key)
        .unwrap_or_else(|| panic!("Missing locale key: {key}"))
        .as_str()
}

pub fn language_name(language: Language) -> &'static str {
    match language.code() {
        Some(code) => &catalog(code)["language_name"],
        None => tr("system"),
    }
}

pub fn area_label(area: Area) -> &'static str {
    tr(match area {
        Area::Staged => "staged",
        Area::Partial => "partially_staged",
        Area::Unstaged => "unstaged",
        Area::Untracked => "untracked",
        Area::Conflict => "conflict",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{')
            .skip(1)
            .map(|part| part.split_once('}').expect("Unclosed placeholder").0)
            .collect()
    }

    #[test]
    fn every_language_has_complete_copy_and_matching_placeholders() {
        let english = catalog("en");
        let keys: BTreeSet<_> = english.keys().collect();
        let supported: BTreeSet<_> = Language::ALL
            .into_iter()
            .filter_map(Language::code)
            .collect();
        assert_eq!(supported, RESOURCES.iter().map(|(code, _)| *code).collect());
        for code in supported {
            let translated = catalog(code);
            assert_eq!(translated.keys().collect::<BTreeSet<_>>(), keys, "{code}");
            for (key, value) in translated {
                assert!(!value.trim().is_empty(), "{code}.{key}");
                assert_eq!(
                    placeholders(value),
                    placeholders(&english[key]),
                    "{code}.{key}"
                );
            }
        }
    }
}
