use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const DEFAULT_PROFILE: &str = "Défaut";

fn default_gain() -> f32 {
    1.0
}

pub const DEFAULT_GRADIENT_FROM: [u8; 3] = [24, 26, 31];
pub const DEFAULT_GRADIENT_TO: [u8; 3] = [12, 13, 16];
pub const DEFAULT_GRADIENT_ANGLE: u16 = 90;
pub const DEFAULT_SCRIM: u8 = 110;
pub const MAX_VOLUME_GAIN: f32 = 2.0;
pub const MAX_TRIM_GAIN: f32 = 16.0;
pub const MAX_STRING_LEN: usize = 1024;
const MAX_STRIPS: usize = 256;
const MAX_OUTPUTS: usize = 64;
const MAX_MIX_GAINS: usize = 64;
const MAX_CABLES: usize = 64;
const MAX_DELAY_MS: u32 = 5000;

pub(crate) fn safe_gain(gain: f32, max: f32, fallback: f32) -> f32 {
    if gain.is_finite() {
        gain.clamp(0.0, max)
    } else {
        fallback
    }
}

pub(crate) fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_STRING_LEN
        && !value.chars().any(char::is_control)
}

const FALLBACK_APP_PREFIX: &str = "processus ";

pub(crate) fn is_fallback_app_exe(exe: &str) -> bool {
    exe.to_ascii_lowercase().starts_with(FALLBACK_APP_PREFIX)
}

pub(crate) fn is_fallback_string_key(value: &str) -> bool {
    value.strip_prefix("app:").is_some_and(|rest| {
        let exe = rest.split('|').next().unwrap_or(rest);
        is_fallback_app_exe(exe)
    })
}

pub(crate) fn sanitize_owned_text(text: &mut String) {
    if text.chars().any(char::is_control) {
        *text = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
    }
    let end = text
        .char_indices()
        .nth(MAX_STRING_LEN)
        .map_or(text.len(), |(i, _)| i);
    text.truncate(end);
}

pub(crate) fn sanitize_text(value: &mut Option<String>) {
    if let Some(text) = value {
        sanitize_owned_text(text);
    }
}

fn default_gradient_from() -> [u8; 3] {
    DEFAULT_GRADIENT_FROM
}

fn default_gradient_to() -> [u8; 3] {
    DEFAULT_GRADIENT_TO
}

fn default_gradient_angle() -> u16 {
    DEFAULT_GRADIENT_ANGLE
}

fn default_scrim() -> u8 {
    DEFAULT_SCRIM
}

mod mix_gains_serde {
    use super::{BTreeMap, EndpointKey};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(
        gains: &BTreeMap<EndpointKey, f32>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        gains
            .iter()
            .map(|(key, gain)| (key.clone(), *gain))
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<EndpointKey, f32>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<(EndpointKey, f32)>::deserialize(deserializer)?;
        Ok(entries.into_iter().collect())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AppRole {
    Call,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EndpointKey {
    Hardware(String),
    App { exe: String, role: AppRole },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StripProfile {
    pub source: EndpointKey,
    pub name: Option<String>,
    pub outputs: Vec<EndpointKey>,
    pub volume: f32,
    #[serde(default, with = "mix_gains_serde")]
    pub mix_gains: BTreeMap<EndpointKey, f32>,
    #[serde(default = "default_gain")]
    pub trim: f32,
    #[serde(default)]
    pub delay_ms: u32,
    #[serde(default)]
    pub pan: i8,
    #[serde(default)]
    pub noise_suppression: bool,
    pub muted: bool,
    pub solo: bool,
    pub pfl: bool,
}

impl Default for StripProfile {
    fn default() -> Self {
        Self {
            source: EndpointKey::Hardware(String::new()),
            name: None,
            outputs: Vec::new(),
            volume: 1.0,
            mix_gains: BTreeMap::new(),
            trim: 1.0,
            delay_ms: 0,
            pan: 0,
            noise_suppression: false,
            muted: false,
            solo: false,
            pfl: false,
        }
    }
}

impl StripProfile {
    fn sanitize(&mut self) {
        self.volume = safe_gain(self.volume, MAX_VOLUME_GAIN, 1.0);
        self.trim = safe_gain(self.trim, MAX_TRIM_GAIN, 1.0);
        self.delay_ms = self.delay_ms.min(MAX_DELAY_MS);
        sanitize_text(&mut self.name);
        self.outputs.retain(valid_endpoint_key);
        self.outputs.truncate(MAX_OUTPUTS);
        self.mix_gains.retain(|key, _| valid_endpoint_key(key));
        while self.mix_gains.len() > MAX_MIX_GAINS {
            self.mix_gains.pop_last();
        }
        for gain in self.mix_gains.values_mut() {
            *gain = safe_gain(*gain, MAX_VOLUME_GAIN, 1.0);
        }
    }
}

pub(crate) fn valid_endpoint_key(key: &EndpointKey) -> bool {
    match key {
        EndpointKey::Hardware(id) => valid_key(id),
        EndpointKey::App { exe, .. } => valid_key(exe) && !is_fallback_app_exe(exe),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MasterProfile {
    pub cue_volume: f32,
    pub cue_device: Option<String>,
    #[serde(default)]
    pub cue_mix: Option<String>,
    #[serde(default)]
    pub cue_mix_enabled: bool,
    pub mute_all: bool,
}

impl Default for MasterProfile {
    fn default() -> Self {
        Self {
            cue_volume: 1.0,
            cue_device: None,
            cue_mix: None,
            cue_mix_enabled: false,
            mute_all: false,
        }
    }
}

impl MasterProfile {
    fn sanitize(&mut self) {
        self.cue_volume = safe_gain(self.cue_volume, MAX_VOLUME_GAIN, 1.0);
        sanitize_text(&mut self.cue_device);
        sanitize_text(&mut self.cue_mix);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppearanceProfile {
    #[serde(default = "default_gradient_from")]
    pub gradient_from: [u8; 3],
    #[serde(default = "default_gradient_to")]
    pub gradient_to: [u8; 3],
    #[serde(default = "default_gradient_angle")]
    pub angle: u16,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default = "default_scrim")]
    pub scrim: u8,
}

impl Default for AppearanceProfile {
    fn default() -> Self {
        Self {
            gradient_from: DEFAULT_GRADIENT_FROM,
            gradient_to: DEFAULT_GRADIENT_TO,
            angle: DEFAULT_GRADIENT_ANGLE,
            image: None,
            scrim: DEFAULT_SCRIM,
        }
    }
}

impl AppearanceProfile {
    fn sanitize(&mut self) {
        sanitize_text(&mut self.image);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Profile {
    #[serde(default)]
    pub strips: Vec<StripProfile>,
    #[serde(default)]
    pub master: MasterProfile,
    #[serde(default)]
    pub cable_assignment: BTreeMap<String, u32>,
    #[serde(default)]
    pub shortcut: u8,
    #[serde(default)]
    pub appearance: AppearanceProfile,
}

impl Profile {
    pub(crate) fn sanitize(&mut self) {
        self.strips.truncate(MAX_STRIPS);
        for strip in &mut self.strips {
            strip.sanitize();
        }
        self.master.sanitize();
        self.appearance.sanitize();
        self.cable_assignment
            .retain(|key, _| valid_key(key) && !is_fallback_string_key(key));
        while self.cable_assignment.len() > MAX_CABLES {
            self.cable_assignment.pop_last();
        }
        self.shortcut = self.shortcut.min(9);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_roundtrip_preserves_app_keys() {
        let profile = Profile {
            strips: vec![StripProfile {
                source: EndpointKey::App {
                    exe: "c:\\program files\\discord\\discord.exe".into(),
                    role: AppRole::Call,
                },
                name: Some("Discord perso".into()),
                outputs: vec![EndpointKey::Hardware("output:speakers".into())],
                volume: 0.5,
                mix_gains: BTreeMap::from([(EndpointKey::Hardware("output:a".into()), 0.25)]),
                trim: 1.2,
                delay_ms: 250,
                pan: -35,
                noise_suppression: true,
                muted: true,
                solo: false,
                pfl: true,
            }],
            master: MasterProfile {
                cue_volume: 0.8,
                cue_device: Some("output:headphones".into()),
                cue_mix: Some("output:speakers".into()),
                cue_mix_enabled: true,
                mute_all: false,
            },
            cable_assignment: std::collections::BTreeMap::from([(
                "app:c:\\discord\\discord.exe".into(),
                2,
            )]),
            shortcut: 1,
            appearance: AppearanceProfile {
                gradient_from: [10, 20, 30],
                gradient_to: [40, 50, 60],
                angle: 135,
                image: Some("bg.png".to_string()),
                scrim: 80,
            },
        };
        let json = serde_json::to_string(&profile).unwrap();
        let decoded: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, profile);
    }

    #[test]
    fn mix_gains_roundtrip_for_hardware_and_app_keys() {
        let profile = Profile {
            strips: vec![StripProfile {
                mix_gains: BTreeMap::from([
                    (EndpointKey::Hardware("output:a".into()), 0.25),
                    (
                        EndpointKey::App {
                            exe: "c:\\discord.exe".into(),
                            role: AppRole::Call,
                        },
                        0.75,
                    ),
                ]),
                ..Default::default()
            }],
            ..Default::default()
        };
        let json = serde_json::to_string(&profile).unwrap();
        let decoded: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, profile);
    }

    #[test]
    fn missing_profile_fields_use_defaults() {
        let profile: Profile = serde_json::from_str(r#"{"strips":[]}"#).unwrap();
        assert!(profile.strips.is_empty());
        assert!(profile.cable_assignment.is_empty());
        assert_eq!(profile.shortcut, 0);
        assert_eq!(profile.master, MasterProfile::default());
        assert_eq!(profile.appearance, AppearanceProfile::default());
    }

    #[test]
    fn partial_appearance_uses_sensible_defaults() {
        let appearance: AppearanceProfile =
            serde_json::from_str(r#"{"gradient_from":[1,2,3]}"#).unwrap();
        assert_eq!(appearance.gradient_from, [1, 2, 3]);
        assert_eq!(appearance.gradient_to, DEFAULT_GRADIENT_TO);
        assert_eq!(appearance.angle, DEFAULT_GRADIENT_ANGLE);
        assert_eq!(appearance.image, None);
        assert_eq!(appearance.scrim, DEFAULT_SCRIM);
    }

    #[test]
    fn strip_without_trim_defaults_to_unity() {
        let profile: Profile = serde_json::from_str(
            r#"{"strips":[{"source":{"Hardware":"input:mic"},"outputs":[],"volume":0.5,"muted":false,"solo":false,"pfl":false}]}"#,
        )
        .unwrap();
        assert_eq!(profile.strips[0].trim, 1.0);
        assert_eq!(profile.strips[0].delay_ms, 0);
        assert_eq!(profile.strips[0].pan, 0);
        assert!(!profile.strips[0].noise_suppression);
        assert!(profile.strips[0].mix_gains.is_empty());
    }

    #[test]
    fn master_without_cue_mix_defaults_to_headphone() {
        let profile: Profile = serde_json::from_str(
            r#"{"master":{"cue_volume":0.5,"cue_device":null,"cue_pre":false,"mute_inputs":false,"mute_outputs":false,"mute_all":false}}"#,
        )
        .unwrap();
        assert_eq!(profile.master.cue_mix, None);
        assert!(!profile.master.cue_mix_enabled);
    }

    #[test]
    fn master_with_mix_but_without_mode_defaults_to_headphone() {
        let profile: Profile = serde_json::from_str(
            r#"{"master":{"cue_volume":0.5,"cue_device":null,"cue_mix":"output:a","mute_inputs":false,"mute_outputs":false,"mute_all":false}}"#,
        )
        .unwrap();
        assert_eq!(profile.master.cue_mix.as_deref(), Some("output:a"));
        assert!(!profile.master.cue_mix_enabled);
    }

    #[test]
    fn sanitize_bounds_gains_and_rejects_invalid_keys() {
        let mut profile = Profile {
            strips: vec![StripProfile {
                volume: f32::INFINITY,
                trim: f32::NAN,
                delay_ms: u32::MAX,
                mix_gains: BTreeMap::from([
                    (EndpointKey::Hardware(String::new()), 1.0),
                    (EndpointKey::Hardware("output:a".into()), 100.0),
                ]),
                ..Default::default()
            }],
            master: MasterProfile {
                cue_volume: -1.0,
                cue_device: Some("output:a\n[hotkey]".into()),
                ..Default::default()
            },
            appearance: AppearanceProfile {
                image: Some("bg\u{0}.png".into()),
                scrim: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        profile.sanitize();
        let strip = &profile.strips[0];
        assert_eq!(strip.volume, 1.0);
        assert_eq!(strip.trim, 1.0);
        assert_eq!(strip.delay_ms, MAX_DELAY_MS);
        assert_eq!(strip.mix_gains.len(), 1);
        assert_eq!(strip.mix_gains.values().next(), Some(&MAX_VOLUME_GAIN));
        assert_eq!(profile.master.cue_volume, 0.0);
        assert_eq!(
            profile.master.cue_device.as_deref(),
            Some("output:a [hotkey]")
        );
        assert_eq!(profile.appearance.image.as_deref(), Some("bg .png"));
        assert_eq!(profile.appearance.scrim, 0);
    }

    #[test]
    fn sanitize_drops_processus_fallback_keys() {
        let fallback = EndpointKey::App {
            exe: "Processus 7292".into(),
            role: AppRole::Other,
        };
        let stable = EndpointKey::App {
            exe: "telegram.exe".into(),
            role: AppRole::Other,
        };
        let mut profile = Profile {
            strips: vec![StripProfile {
                outputs: vec![fallback.clone(), stable.clone()],
                mix_gains: BTreeMap::from([(fallback.clone(), 1.0), (stable.clone(), 0.5)]),
                ..Default::default()
            }],
            cable_assignment: BTreeMap::from([
                ("app:processus 7292".into(), 2),
                ("app:telegram.exe".into(), 1),
            ]),
            ..Default::default()
        };
        profile.sanitize();
        let strip = &profile.strips[0];
        assert_eq!(strip.outputs, vec![stable.clone()]);
        assert!(!strip.mix_gains.contains_key(&fallback));
        assert_eq!(strip.mix_gains.get(&stable), Some(&0.5));
        assert!(!profile.cable_assignment.contains_key("app:processus 7292"));
        assert_eq!(profile.cable_assignment.get("app:telegram.exe"), Some(&1));
    }

    #[test]
    fn sanitize_keeps_full_trim_range() {
        use crate::ui::{trim_db, trim_gain, TRIM_MAX_DB, TRIM_MIN_DB};

        let mut profile = Profile {
            strips: vec![StripProfile {
                trim: trim_gain(TRIM_MAX_DB),
                ..Default::default()
            }],
            ..Default::default()
        };
        profile.sanitize();
        assert_eq!(profile.strips[0].trim, trim_gain(TRIM_MAX_DB));
        assert_eq!(trim_db(profile.strips[0].trim), TRIM_MAX_DB);
        assert_eq!(trim_db(trim_gain(TRIM_MIN_DB)), TRIM_MIN_DB);
    }

    #[test]
    fn sanitize_caps_collection_sizes() {
        let mut profile = Profile {
            strips: (0..300)
                .map(|_| StripProfile {
                    outputs: (0..100)
                        .map(|i| EndpointKey::Hardware(format!("output:{i}")))
                        .collect(),
                    mix_gains: (0..100)
                        .map(|i| (EndpointKey::Hardware(format!("output:{i}")), 1.0))
                        .collect(),
                    ..Default::default()
                })
                .collect(),
            cable_assignment: (0..100).map(|i| (format!("app:{i}"), i)).collect(),
            shortcut: 200,
            ..Default::default()
        };
        profile.sanitize();
        assert_eq!(profile.strips.len(), 256);
        assert_eq!(profile.strips[0].outputs.len(), 64);
        assert_eq!(profile.strips[0].mix_gains.len(), 64);
        assert_eq!(profile.cable_assignment.len(), 64);
        assert_eq!(profile.shortcut, 9);
    }
}
