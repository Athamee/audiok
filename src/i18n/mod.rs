use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Deserializer, Serialize};

mod de;
mod en;
mod es;
mod it;
mod ja;
mod pt;
mod zh;

use de::DE;
use en::EN;
use es::ES;
use it::IT;
use ja::JA;
use pt::PT;
use zh::ZH;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Fr,
    En,
    De,
    Es,
    It,
    Pt,
    Zh,
    Ja,
}

impl Language {
    pub fn all() -> &'static [Language] {
        &[
            Self::Fr,
            Self::En,
            Self::De,
            Self::Es,
            Self::It,
            Self::Pt,
            Self::Zh,
            Self::Ja,
        ]
    }

    pub fn index(self) -> u8 {
        match self {
            Self::Fr => 0,
            Self::En => 1,
            Self::De => 2,
            Self::Es => 3,
            Self::It => 4,
            Self::Pt => 5,
            Self::Zh => 6,
            Self::Ja => 7,
        }
    }

    fn from_index(index: u8) -> Self {
        match index {
            1 => Self::En,
            2 => Self::De,
            3 => Self::Es,
            4 => Self::It,
            5 => Self::Pt,
            6 => Self::Zh,
            7 => Self::Ja,
            _ => Self::Fr,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Fr => "FR",
            Self::En => "EN",
            Self::De => "DE",
            Self::Es => "ES",
            Self::It => "IT",
            Self::Pt => "PT",
            Self::Zh => "ZH",
            Self::Ja => "JA",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Fr => "Français",
            Self::En => "English",
            Self::De => "Deutsch",
            Self::Es => "Español",
            Self::It => "Italiano",
            Self::Pt => "Português",
            Self::Zh => "中文",
            Self::Ja => "日本語",
        }
    }

    pub fn flag(self) -> &'static str {
        match self {
            Self::Fr => "🇫🇷",
            Self::En => "🇬🇧",
            Self::De => "🇩🇪",
            Self::Es => "🇪🇸",
            Self::It => "🇮🇹",
            Self::Pt => "🇵🇹",
            Self::Zh => "🇨🇳",
            Self::Ja => "🇯🇵",
        }
    }

    pub fn label(self) -> String {
        format!("{} {} {}", self.flag(), self.code(), self.name())
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(language: Language) {
    CURRENT.store(language.index(), Ordering::Relaxed);
}

pub fn current() -> Language {
    Language::from_index(CURRENT.load(Ordering::Relaxed))
}

fn table(language: Language) -> Option<&'static [(&'static str, &'static str)]> {
    match language {
        Language::Fr => None,
        Language::En => Some(EN),
        Language::De => Some(DE),
        Language::Es => Some(ES),
        Language::It => Some(IT),
        Language::Pt => Some(PT),
        Language::Zh => Some(ZH),
        Language::Ja => Some(JA),
    }
}

pub fn tr(fr: &'static str) -> &'static str {
    table(current())
        .and_then(|entries| entries.iter().find(|(source, _)| *source == fr))
        .map_or(fr, |(_, translated)| *translated)
}

pub fn format(template: &'static str, args: &[(&str, String)]) -> String {
    let mut out = tr(template).to_string();
    for (placeholder, value) in args {
        out = out.replace(placeholder, value);
    }
    out
}

#[macro_export]
macro_rules! trf {
    ($template:literal $(, $placeholder:literal => $value:expr)* $(,)?) => {
        $crate::i18n::format($template, &[$(($placeholder, $value.to_string())),*])
    };
}

fn language_from_tag(tag: &str) -> Option<Language> {
    let primary = tag.split(['-', '_']).next()?.to_ascii_lowercase();
    match primary.as_str() {
        "fr" => Some(Language::Fr),
        "en" => Some(Language::En),
        "de" => Some(Language::De),
        "es" => Some(Language::Es),
        "it" => Some(Language::It),
        "pt" => Some(Language::Pt),
        "zh" => Some(Language::Zh),
        "ja" => Some(Language::Ja),
        _ => None,
    }
}

pub fn detect_from_locales<I, S>(locales: I) -> Language
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    locales
        .into_iter()
        .find_map(|locale| language_from_tag(locale.as_ref()))
        .unwrap_or(Language::Fr)
}

static SYSTEM_LANGUAGE: OnceLock<Language> = OnceLock::new();

pub fn system_language() -> Language {
    *SYSTEM_LANGUAGE.get_or_init(|| detect_from_locales(sys_locale::get_locales()))
}

pub fn deserialize_language<'de, D>(deserializer: D) -> Result<Option<Language>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(match value.as_deref() {
        Some("fr") => Some(Language::Fr),
        Some("en") => Some(Language::En),
        Some("de") => Some(Language::De),
        Some("es") => Some(Language::Es),
        Some("it") => Some(Language::It),
        Some("pt") => Some(Language::Pt),
        Some("zh") => Some(Language::Zh),
        Some("ja") => Some(Language::Ja),
        _ => None,
    })
}

#[cfg(test)]
fn tables() -> [&'static [(&'static str, &'static str)]; 7] {
    [EN, DE, ES, IT, PT, ZH, JA]
}

#[cfg(test)]
fn placeholders(text: &str) -> std::collections::BTreeSet<String> {
    let mut result = std::collections::BTreeSet::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '{' {
            let mut placeholder = String::new();
            let mut closed = false;
            for character in characters.by_ref() {
                if character == '}' {
                    closed = true;
                    break;
                }
                placeholder.push(character);
            }
            if closed {
                result.insert(placeholder);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn language_metadata_is_consistent() {
        let mut indexes = BTreeSet::new();
        let mut codes = BTreeSet::new();
        for language in Language::all() {
            assert!(indexes.insert(language.index()));
            assert!(codes.insert(language.code()));
            assert!(!language.flag().is_empty());
            assert!(!language.name().is_empty());
            assert!(language.label().contains(language.code()));
            assert!(language.label().contains(language.name()));
            assert_eq!(Language::from_index(language.index()), *language);
        }
    }

    #[test]
    fn detect_from_locales_maps_supported_tags_and_falls_back() {
        assert_eq!(detect_from_locales(["fr-FR"]), Language::Fr);
        assert_eq!(detect_from_locales(["ru-RU", "en-US"]), Language::En);
        assert_eq!(detect_from_locales(["de-DE"]), Language::De);
        assert_eq!(detect_from_locales(["es_ES"]), Language::Es);
        assert_eq!(detect_from_locales(["it-IT"]), Language::It);
        assert_eq!(detect_from_locales(["pt_PT"]), Language::Pt);
        assert_eq!(detect_from_locales(["zh-Hans-CN"]), Language::Zh);
        assert_eq!(detect_from_locales(["ja-JP"]), Language::Ja);
        assert_eq!(detect_from_locales(["xx-YY"]), Language::Fr);
        assert_eq!(
            detect_from_locales(std::iter::empty::<String>()),
            Language::Fr
        );
    }

    #[test]
    fn translated_tables_have_unique_non_empty_entries() {
        for table in tables() {
            let mut sources = BTreeSet::new();
            for (source, translated) in table {
                assert!(!source.is_empty());
                assert!(!translated.is_empty());
                assert!(sources.insert(*source), "duplicate key: {source}");
            }
        }
    }

    #[test]
    fn translated_tables_cover_all_sources_and_placeholders() {
        let sources: BTreeSet<_> = EN.iter().map(|(source, _)| *source).collect();
        for table in tables() {
            let keys: BTreeSet<_> = table.iter().map(|(source, _)| *source).collect();
            assert_eq!(keys, sources);
            for (source, translated) in table {
                assert_eq!(
                    placeholders(source),
                    placeholders(translated),
                    "key: {source}"
                );
            }
        }
    }
}
