use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::i18n::{deserialize_language, Language};

use super::profile::{
    is_fallback_string_key, safe_gain, sanitize_owned_text, sanitize_text, valid_key, Profile,
    MAX_VOLUME_GAIN,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub cue_volume: f32,
    pub bypass: bool,
    pub cue_device: Option<String>,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
    #[serde(default)]
    pub active_profile: Option<String>,
    #[serde(default)]
    pub start_with_windows: bool,
    #[serde(
        default,
        deserialize_with = "deserialize_language",
        skip_serializing_if = "Option::is_none"
    )]
    pub language: Option<Language>,
}

const MAX_PROFILES: usize = 64;
const MAX_ALIASES: usize = 1024;

impl Settings {
    pub(crate) fn sanitize(&mut self) {
        self.cue_volume = safe_gain(self.cue_volume, MAX_VOLUME_GAIN, 1.0);
        sanitize_text(&mut self.cue_device);
        sanitize_text(&mut self.active_profile);
        if self.aliases.len() > MAX_ALIASES {
            self.aliases.clear();
        }
        self.aliases
            .retain(|key, _| valid_key(key) && !is_fallback_string_key(key));
        for value in self.aliases.values_mut() {
            sanitize_owned_text(value);
        }
        while self.profiles.len() > MAX_PROFILES {
            self.profiles.pop_last();
        }
        for profile in self.profiles.values_mut() {
            profile.sanitize();
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cue_volume: 1.0,
            bypass: false,
            cue_device: None,
            aliases: HashMap::new(),
            profiles: BTreeMap::new(),
            active_profile: None,
            start_with_windows: false,
            language: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::Settings;
    use crate::config::profile::{MasterProfile, Profile};
    use crate::i18n::Language;

    #[test]
    fn missing_aliases_default_to_empty() {
        let settings: Settings =
            serde_json::from_str(r#"{"cue_volume":0.5,"bypass":true,"cue_device":null}"#).unwrap();
        assert!(settings.aliases.is_empty());
        assert!(settings.profiles.is_empty());
        assert!(settings.active_profile.is_none());
        assert!(!settings.start_with_windows);
        assert_eq!(settings.language, None);
    }

    #[test]
    fn language_preference_keeps_supported_codes_and_auto() {
        for (code, language) in [
            ("fr", Language::Fr),
            ("en", Language::En),
            ("de", Language::De),
            ("es", Language::Es),
            ("it", Language::It),
            ("pt", Language::Pt),
            ("zh", Language::Zh),
            ("ja", Language::Ja),
        ] {
            let json = format!(
                r#"{{"cue_volume":1.0,"bypass":false,"cue_device":null,"language":"{code}"}}"#
            );
            let settings: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(settings.language, Some(language));
        }

        for value in ["null", "\"xx\""] {
            let json = format!(
                r#"{{"cue_volume":1.0,"bypass":false,"cue_device":null,"language":{value}}}"#
            );
            let settings: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(settings.language, None);
        }
    }

    #[test]
    fn auto_language_is_not_serialized() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(!json.contains("language"));
    }

    #[test]
    fn settings_roundtrip_preserves_aliases_profiles_and_active() {
        let profile = Profile {
            master: MasterProfile {
                cue_volume: 0.6,
                cue_device: Some("output:headphones".into()),
                cue_mix: Some("output:speakers".into()),
                cue_mix_enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let settings = Settings {
            cue_volume: 0.4,
            bypass: true,
            cue_device: Some("output:cue".into()),
            aliases: HashMap::from([("output:a".into(), "Perso".into())]),
            profiles: BTreeMap::from([("Live".to_string(), profile.clone())]),
            active_profile: Some("Live".into()),
            start_with_windows: true,
            language: Some(Language::En),
        };

        let json = serde_json::to_string(&settings).unwrap();
        let decoded: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.cue_volume, 0.4);
        assert!(decoded.bypass);
        assert_eq!(decoded.cue_device.as_deref(), Some("output:cue"));
        assert_eq!(
            decoded.aliases.get("output:a").map(String::as_str),
            Some("Perso")
        );
        assert_eq!(decoded.active_profile.as_deref(), Some("Live"));
        assert!(decoded.start_with_windows);
        assert_eq!(decoded.language, Some(Language::En));
        assert_eq!(decoded.profiles.get("Live"), Some(&profile));
    }

    #[test]
    fn sanitize_replaces_non_finite_values_and_invalid_aliases() {
        let mut settings = Settings {
            cue_volume: f32::NAN,
            aliases: HashMap::from([
                ("".into(), "x".into()),
                ("app:a".into(), "alias\n[hotkey]".into()),
                ("app:Processus 42|other".into(), "x".into()),
            ]),
            ..Default::default()
        };
        settings.sanitize();
        assert_eq!(settings.cue_volume, 1.0);
        assert_eq!(settings.aliases.len(), 1);
        assert!(!settings.aliases.contains_key("app:Processus 42|other"));
        assert_eq!(
            settings.aliases.get("app:a").map(String::as_str),
            Some("alias [hotkey]")
        );
    }
}
