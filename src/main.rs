#![cfg_attr(windows, windows_subsystem = "windows")]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use eframe::egui;

mod audio;
mod config;
mod hotkeys;
mod i18n;
mod logging;
mod ui;

use audio::devices::{endpoint_pid, AudioEndpoint};
use config::profile::{
    is_fallback_string_key, valid_endpoint_key, AppRole, AppearanceProfile, EndpointKey,
    MasterProfile, Profile, StripProfile, DEFAULT_PROFILE,
};
use i18n::tr;

const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const METER_INTERVAL: Duration = Duration::from_millis(33);
const AUTOSAVE_DELAY: Duration = Duration::from_secs(2);
const REDIRECT_RETRY: Duration = Duration::from_secs(2);
const STRIP_GRACE: Duration = Duration::from_secs(7);
const CACHE_CAP: usize = 128;
const TOP_MARGIN: f32 = 8.0;
const LEFT_MARGIN: f32 = 8.0;
const STRIP_SPACING: f32 = 12.0;
const SCREEN_MARGIN: f32 = 32.0;
const WIDTH_EPSILON: f32 = 1.0;
const RESIZE_EPSILON: f32 = 1.0;
const MIN_WINDOW_WIDTH: f32 = 420.0;
const LANGUAGE_COMBO_WIDTH: f32 = 45.0;
const LANGUAGE_COMBO_PADDING: f32 = 3.0;
const TRAY_ARG: &str = "--tray";

struct Rename {
    endpoint: String,
    buffer: String,
    focus: bool,
}

struct NewProfileDialog {
    name: String,
    focus: bool,
}

#[derive(Clone)]
struct ProfileOption {
    id: String,
    label: String,
}

#[derive(Clone)]
enum ProfileTarget {
    Source { saved: StripProfile },
    Output { strip_id: String, index: usize },
}

#[derive(Clone)]
struct ProfileChoice {
    key: EndpointKey,
    target: ProfileTarget,
    options: Vec<ProfileOption>,
    selected: usize,
}

struct AmbiguousOutput {
    index: usize,
    key: EndpointKey,
    ids: Vec<String>,
}

enum ProfileAction {
    Switch(String),
    Save,
    New,
    Delete,
    Reset,
    SetShortcut { profile: String, slot: u8 },
}

#[derive(Clone)]
struct PendingShortcut {
    profile: String,
    slot: u8,
    owner: String,
}

#[derive(Default)]
struct AudiokApp {
    settings: config::Settings,
    last_refresh: Option<Instant>,
    outputs: Vec<AudioEndpoint>,
    strips: Vec<ui::strip::Strip>,
    cache: HashMap<String, ui::strip::Strip>,
    master: ui::master::Master,
    engine: audio::engine::AudioEngine,
    vac: audio::vac::VacRouter,
    app_keys: HashMap<u32, String>,
    captures: HashSet<u32>,
    alias_labels: HashMap<String, String>,
    rename_request: Option<String>,
    reset_request: Option<String>,
    rename: Option<Rename>,
    pending_profile: HashMap<EndpointKey, StripProfile>,
    profile_action: Option<ProfileAction>,
    new_profile: Option<NewProfileDialog>,
    delete_profile_request: Option<String>,
    profile_choices: Vec<ProfileChoice>,
    pending_outputs: HashMap<EndpointKey, Vec<EndpointKey>>,
    redirected: HashMap<u32, [Option<String>; 3]>,
    redirect_retry: HashMap<u32, Instant>,
    diag_redirect_desired: Vec<u32>,
    diag_redirect_actual: Vec<u32>,
    diag_app_routes: Vec<(String, String)>,
    sink_warned: bool,
    last_fingerprint: Option<u64>,
    dirty_since: Option<Instant>,
    hotkeys: Option<hotkeys::Hotkeys>,
    pending_shortcut: Option<PendingShortcut>,
    appearance: AppearanceProfile,
    appearance_cache: ui::background::BackgroundCache,
    appearance_open: bool,
    appearance_missing_image: Option<String>,
    image_alert: bool,
    tray: Option<tray_icon::TrayIcon>,
    tray_start: Option<tray_icon::menu::CheckMenuItem>,
    hidden: bool,
    hide_after_start: bool,
    tray_quit: bool,
    help_open: bool,
}

fn assign_shortcut(profiles: &mut BTreeMap<String, Profile>, active: &str, slot: u8) {
    if slot != 0 {
        for (name, profile) in profiles.iter_mut() {
            if name != active && profile.shortcut == slot {
                profile.shortcut = 0;
            }
        }
    }
    if let Some(profile) = profiles.get_mut(active) {
        profile.shortcut = slot;
    }
}

fn color_from_f32(rgb: [f32; 3]) -> [u8; 3] {
    rgb.map(|value| (value * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn appearance_button(ui: &mut egui::Ui, warning: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
    let color = if warning {
        ui::METER_WARN
    } else if response.hovered() || response.has_focus() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let center = rect.center();
    let painter = ui.painter();
    painter.circle_filled(center + egui::vec2(-1.0, 1.0), 6.5, color);
    for offset in [
        egui::vec2(-4.0, -1.0),
        egui::vec2(1.0, -2.0),
        egui::vec2(4.0, 2.0),
    ] {
        painter.circle_filled(center + offset, 1.4, ui.visuals().window_fill());
    }
    if warning {
        painter.circle_filled(
            rect.right_top() + egui::vec2(-1.0, 1.0),
            2.5,
            ui::METER_WARN,
        );
    }
    response.on_hover_text(if warning {
        tr("Apparence — image de fond manquante")
    } else {
        tr("Apparence")
    })
}

fn language_selector_text(language: Option<i18n::Language>) -> String {
    match language {
        Some(language) => language.code().to_string(),
        None => "🌐".to_string(),
    }
}

pub(crate) const MAX_SHORTCUT_SLOT: u8 = 10;

pub(crate) fn shortcut_label(slot: u8) -> String {
    crate::trf!("Ctrl+Maj+{}", "{}" => slot % 10)
}

fn shortcut_lexicon_lines(settings: &config::Settings) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(tr("Globaux").to_string());
    lines.push(tr("  F9 : mute/unmute micros (tranches input)").to_string());
    lines.push(tr("  F10 : mute/unmute global").to_string());
    for slot in 1..=MAX_SHORTCUT_SLOT {
        let owner = profile_for_shortcut(settings, slot);
        let target = owner
            .as_deref()
            .map(|name| crate::trf!("profil « {name} »", "{name}" => name))
            .unwrap_or_else(|| tr("non assigné").to_string());
        lines.push(format!("  {} : {target}", shortcut_label(slot)));
    }
    lines.push(String::new());
    lines.push(tr("Contrôles").to_string());
    lines.push(tr("  Fader / valeur volume : molette ou glisser ; double-clic = 0 dB").to_string());
    lines.push(tr("  Gain / Retard / Pan : molette ou glisser ; double-clic = défaut").to_string());
    lines.push(tr("  Double clic sur sortie : renommer / nom par défaut").to_string());
    lines.push(tr("  Solo / Mute / PFL : boutons de tranche").to_string());
    lines.push(String::new());
    lines.push(tr("Mode d’emploi").to_string());
    lines.push(
        tr("  Route les tranches d'entrée vers les sorties ; le bypass remet Windows en défaut.")
            .to_string(),
    );
    lines.push(
        tr("  MIXE/PFL : écoute post-fader du mixe sélectionné ou PFL pré-fader.").to_string(),
    );
    lines.push(
        tr("  Profils : sauvegardent routages, volumes, apparence et raccourcis.").to_string(),
    );
    lines.push(
        tr("  NR (tranches input) : réduction de bruit RNNoise, ~10 ms à 48 kHz (+ resampling si besoin).").to_string(),
    );
    lines.push(
        tr("  Routing appli→appli : Virtual Audio Cable (VAC) est nécessaire pour router le son d'une application vers une autre.").to_string(),
    );
    lines.push("    VAC : https://vac.muzychenko.net/".to_string());
    lines
}

fn shortcut_owner(profiles: &BTreeMap<String, Profile>, profile: &str, slot: u8) -> Option<String> {
    if slot == 0 {
        return None;
    }
    profiles
        .iter()
        .find(|(name, saved)| name.as_str() != profile && saved.shortcut == slot)
        .map(|(name, _)| name.clone())
}

fn profile_for_shortcut(settings: &config::Settings, slot: u8) -> Option<String> {
    settings
        .profiles
        .iter()
        .find(|(_, profile)| profile.shortcut == slot)
        .map(|(name, _)| name.clone())
}

fn toggle_mic_mutes(strips: &mut [ui::strip::Strip]) {
    let indexes: Vec<usize> = strips
        .iter()
        .enumerate()
        .filter(|(_, strip)| strip.present && strip.id.starts_with("input:"))
        .map(|(index, _)| index)
        .collect();
    if indexes.is_empty() {
        return;
    }
    let mute = indexes.iter().any(|&index| !strips[index].muted);
    for index in indexes {
        strips[index].muted = mute;
    }
}

impl AudiokApp {
    fn with_settings(mut settings: config::Settings, start_in_tray: bool) -> Self {
        if settings.profiles.is_empty() {
            settings.profiles.insert(
                DEFAULT_PROFILE.to_string(),
                Profile {
                    master: MasterProfile {
                        cue_volume: settings.cue_volume,
                        cue_device: settings.cue_device.clone(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
        }
        let active = settings
            .active_profile
            .clone()
            .filter(|name| settings.profiles.contains_key(name))
            .unwrap_or_else(|| DEFAULT_PROFILE.to_string());
        settings.active_profile = Some(active.clone());
        let legacy_master = MasterProfile {
            cue_volume: settings.cue_volume,
            cue_device: settings.cue_device.clone(),
            ..Default::default()
        };
        settings
            .profiles
            .entry(active.clone())
            .or_insert_with(|| Profile {
                master: legacy_master,
                ..Default::default()
            });
        let (master, pending, cable_assignment, appearance) = settings
            .profiles
            .get(&active)
            .map(|profile| {
                (
                    profile.master.clone(),
                    profile
                        .strips
                        .iter()
                        .map(|strip| (strip.source.clone(), strip.clone()))
                        .collect(),
                    profile.cable_assignment.clone(),
                    profile.appearance.clone(),
                )
            })
            .unwrap_or_default();
        let mut app = Self {
            settings,
            pending_profile: pending,
            ..Default::default()
        };
        app.apply_master_profile(&master);
        app.apply_appearance(&appearance);
        app.vac.load_preferences(cable_assignment);
        app.master.bypass = !start_in_tray;
        app.hide_after_start = start_in_tray;
        app
    }

    fn apply_master_profile(&mut self, master: &MasterProfile) {
        self.master.cue_volume = master.cue_volume;
        self.master.cue_device = master.cue_device.clone();
        self.master.cue_mix = master.cue_mix.clone();
        self.master.cue_mix_enabled = master.cue_mix_enabled;
        self.master.mute_all = master.mute_all;
    }

    fn apply_appearance(&mut self, appearance: &AppearanceProfile) {
        self.appearance = appearance.clone();
        self.appearance_cache = ui::background::BackgroundCache::default();
        self.appearance_missing_image = None;
        self.image_alert = false;
    }

    fn sync_appearance(&mut self) {
        let Some(name) = self.settings.active_profile.clone() else {
            return;
        };
        if let Some(profile) = self.settings.profiles.get_mut(&name) {
            profile.appearance = self.appearance.clone();
        }
    }

    fn refresh(&mut self, trim: bool) {
        let trim = trim && self.profile_choices.is_empty();
        let mut sources = visible_inputs(audio::devices::list_input_devices());
        sources.extend(audio::devices::list_audio_sessions());
        let mut outputs = audio::devices::list_output_devices();
        outputs.extend(audio::devices::list_output_apps());
        let outputs = visible_outputs(outputs);
        let known: HashSet<String> = self
            .strips
            .iter()
            .map(|strip| strip.id.clone())
            .chain(self.cache.keys().cloned())
            .collect();
        let previous_present: HashSet<String> = self
            .strips
            .iter()
            .filter(|strip| strip.present)
            .map(|strip| strip.id.clone())
            .collect();
        let previous_grace: HashSet<String> = self
            .strips
            .iter()
            .filter(|strip| !strip.present)
            .map(|strip| strip.id.clone())
            .collect();
        let (strips, cache) = sync_strips(
            &sources,
            std::mem::take(&mut self.strips),
            std::mem::take(&mut self.cache),
            trim,
            Instant::now(),
            STRIP_GRACE,
        );
        self.outputs = outputs;
        self.strips = strips;
        self.cache = cache;
        let captures = audio::devices::capture_pids();
        self.alias_labels = resolved_aliases(&self.settings.aliases, &self.outputs, &captures);
        self.app_keys = app_stable_keys(&sources, &captures);
        let new_apps: Vec<String> = self
            .strips
            .iter()
            .filter(|strip| strip.present && !known.contains(&strip.id))
            .filter(|strip| endpoint_pid(&strip.id).is_some_and(|pid| pid != 0))
            .map(|strip| strip.id.clone())
            .collect();
        self.restore_returning_strips(&new_apps, &captures);
        self.apply_pending_profile(&sources);
        self.retry_pending_outputs(&captures);
        self.vac.refresh();
        self.captures = captures;
        self.log_strip_transitions(&previous_present, &previous_grace);
    }

    fn diag_strip_state(&self, strip: &ui::strip::Strip) -> String {
        let pid = endpoint_pid(&strip.id);
        let destinations = app_selections(&self.strips);
        let dest = pid.is_some_and(|pid| destinations.contains(&format!("app:{pid}").as_str()));
        let redirected = pid.is_some_and(|pid| self.redirected.contains_key(&pid));
        let alive = pid.is_some_and(audio::devices::process_alive);
        format!("dest={dest} redirect={redirected} vivant={alive}")
    }

    fn log_strip_transitions(
        &self,
        previous_present: &HashSet<String>,
        previous_grace: &HashSet<String>,
    ) {
        for id in previous_present {
            match self.strips.iter().find(|strip| &strip.id == id) {
                Some(strip) if !strip.present => logging::log(
                    "diag",
                    &format!(
                        "entrée grâce {} ({}) {}",
                        strip.name,
                        strip.id,
                        self.diag_strip_state(strip)
                    ),
                ),
                None => logging::log("diag", &format!("retrait direct {id}")),
                _ => {}
            }
        }
        for id in previous_grace {
            match self.strips.iter().find(|strip| &strip.id == id) {
                Some(strip) if strip.present => logging::log(
                    "diag",
                    &format!("retour (fin de grâce) {} ({})", strip.name, strip.id),
                ),
                None => logging::log("diag", &format!("retrait après grâce {id}")),
                _ => {}
            }
        }
    }

    fn apply_output_redirects(&mut self, app_pids: &HashSet<u32>, sink: Option<&str>) {
        let desired = if sink.is_some() && !self.master.bypass {
            app_pids.clone()
        } else {
            HashSet::new()
        };
        let now = Instant::now();
        let to_redirect: Vec<u32> = desired
            .iter()
            .copied()
            .filter(|pid| {
                should_redirect(
                    self.redirected.contains_key(pid),
                    self.redirect_retry.get(pid).copied(),
                    now,
                )
            })
            .collect();
        let to_restore: Vec<u32> = self
            .redirected
            .keys()
            .copied()
            .filter(|pid| !desired.contains(pid))
            .filter(|pid| should_restore(self.redirect_retry.get(pid).copied(), now))
            .collect();

        if !app_pids.is_empty() && sink.is_none() && !self.master.bypass {
            if !self.sink_warned {
                logging::log("redirect", "aucun câble VAC puits disponible");
                self.sink_warned = true;
            }
        } else {
            self.sink_warned = false;
        }

        if let Some(sink) = sink {
            let sink = sink.strip_prefix("output:").unwrap_or(sink);
            for pid in to_redirect {
                let first = !self.redirected.contains_key(&pid);
                let previous = if first {
                    let mut previous = audio::policy::get_default_render(pid);
                    for value in &mut previous {
                        if value.as_deref() == Some(sink) {
                            *value = None;
                        }
                    }
                    logging::log(
                        "redirect",
                        &format!(
                            "pid {pid} -> {sink} (avant: {})",
                            format_previous(&previous)
                        ),
                    );
                    previous
                } else {
                    self.redirected.get(&pid).cloned().unwrap_or_default()
                };
                let outcome = match audio::policy::set_default_render(pid, Some(sink)) {
                    Ok(()) => verify_redirect(pid, sink),
                    Err(error) => Err(error),
                };
                self.redirected.insert(pid, previous);
                match outcome {
                    Ok(()) => {
                        self.redirect_retry.remove(&pid);
                        if !first {
                            logging::log("redirect", &format!("pid {pid} -> {sink} confirmé"));
                        }
                    }
                    Err(error) => {
                        self.redirect_retry.insert(pid, now + REDIRECT_RETRY);
                        logging::log(
                            "redirect",
                            &format!("pid {pid} redirection incomplète: {error}"),
                        );
                    }
                }
            }
        }

        for pid in to_restore {
            let Some(previous) = self.redirected.get(&pid).cloned() else {
                continue;
            };
            match audio::policy::restore_default_render(pid, &previous) {
                Ok(()) => {
                    self.redirected.remove(&pid);
                    self.redirect_retry.remove(&pid);
                    logging::log("redirect", &format!("pid {pid} restauré"));
                }
                Err(error) if audio::devices::process_alive(pid) => {
                    self.redirect_retry.insert(pid, now + REDIRECT_RETRY);
                    logging::log(
                        "redirect",
                        &format!("pid {pid} restauration échouée (nouvelle tentative): {error}"),
                    );
                }
                Err(error) => {
                    self.redirected.remove(&pid);
                    self.redirect_retry.remove(&pid);
                    logging::log(
                        "redirect",
                        &format!("pid {pid} restauration abandonnée (processus terminé): {error}"),
                    );
                }
            }
        }
    }

    fn restore_output_redirects(&mut self) {
        self.redirect_retry.clear();
        for (pid, previous) in std::mem::take(&mut self.redirected) {
            match audio::policy::restore_default_render(pid, &previous) {
                Ok(()) => logging::log("redirect", &format!("pid {pid} restauré (sortie)")),
                Err(error) => logging::log(
                    "redirect",
                    &format!("pid {pid} restauration échouée (sortie): {error}"),
                ),
            }
        }
    }

    fn apply_pending_profile(&mut self, sources: &[AudioEndpoint]) {
        if self.pending_profile.is_empty() {
            return;
        }
        let captures = audio::devices::capture_pids();
        let source_map = runtime_ids(sources, &captures);
        let output_map = runtime_ids(&self.outputs, &captures);
        let pending = std::mem::take(&mut self.pending_profile);
        for (key, saved) in pending {
            let Some(ids) = source_map.get(&key) else {
                self.pending_profile.insert(key, saved);
                continue;
            };
            if ids.len() > 1 {
                self.profile_choices.push(ProfileChoice {
                    key,
                    target: ProfileTarget::Source { saved },
                    options: choice_options(ids, sources),
                    selected: 0,
                });
                continue;
            }
            let id = ids[0].clone();
            let (ambiguous, unresolved) = match self.strips.iter_mut().find(|strip| strip.id == id)
            {
                Some(strip) => apply_strip_profile(strip, &saved, &output_map),
                None => {
                    self.pending_profile.insert(key, saved);
                    continue;
                }
            };
            if !unresolved.is_empty() {
                self.pending_outputs.insert(key, unresolved);
            }
            push_output_choices(&mut self.profile_choices, &id, ambiguous, &self.outputs);
        }
    }

    fn restore_returning_strips(&mut self, new_ids: &[String], captures: &HashSet<u32>) {
        if new_ids.is_empty() {
            return;
        }
        let saved = self
            .settings
            .active_profile
            .as_ref()
            .and_then(|name| self.settings.profiles.get(name))
            .map(|profile| profile.strips.clone())
            .unwrap_or_default();
        if saved.is_empty() {
            return;
        }
        let output_map = runtime_ids(&self.outputs, captures);
        for id in new_ids {
            let Some(strip) = self.strips.iter().find(|strip| &strip.id == id) else {
                continue;
            };
            let key = endpoint_key(&strip.id, captures);
            if self.pending_profile.contains_key(&key) {
                continue;
            }
            let Some(slot) = saved.iter().find(|strip| strip.source == key) else {
                continue;
            };
            let (ambiguous, unresolved) = match self.strips.iter_mut().find(|strip| &strip.id == id)
            {
                Some(strip) => apply_strip_profile(strip, slot, &output_map),
                None => continue,
            };
            logging::log(
                "rebind",
                &format!(
                    "{id} profil restauré (ambigus={} non résolus={})",
                    ambiguous.len(),
                    unresolved.len()
                ),
            );
            if !unresolved.is_empty() {
                self.pending_outputs.insert(key, unresolved);
            }
            push_output_choices(&mut self.profile_choices, id, ambiguous, &self.outputs);
        }
    }

    fn retry_pending_outputs(&mut self, captures: &HashSet<u32>) {
        if self.pending_outputs.is_empty() {
            return;
        }
        let output_map = runtime_ids(&self.outputs, captures);
        let pending = std::mem::take(&mut self.pending_outputs);
        let mut still = HashMap::new();
        for (source, keys) in pending {
            let Some(index) = self
                .strips
                .iter()
                .position(|strip| endpoint_key(&strip.id, captures) == source)
            else {
                still.insert(source, keys);
                continue;
            };
            let mut unresolved = Vec::new();
            for key in keys {
                match output_ids(&key, &output_map) {
                    Some(ids) if ids.len() == 1 => {
                        let id = ids[0].clone();
                        let strip = &mut self.strips[index];
                        if !strip
                            .selected_outputs
                            .iter()
                            .flatten()
                            .any(|existing| existing == &id)
                        {
                            match strip
                                .selected_outputs
                                .iter_mut()
                                .find(|slot| slot.is_none())
                            {
                                Some(slot) => *slot = Some(id.clone()),
                                None => strip.selected_outputs.push(Some(id.clone())),
                            }
                            logging::log("rebind", &format!("{} sortie {id} rebindée", strip.id));
                        }
                    }
                    _ => unresolved.push(key),
                }
            }
            if !unresolved.is_empty() {
                still.insert(source, unresolved);
            }
        }
        self.pending_outputs = still;
    }

    fn snapshot_profile(&self, previous: &Profile) -> Profile {
        let captures = audio::devices::capture_pids();
        let mut strips: BTreeMap<EndpointKey, StripProfile> = previous
            .strips
            .iter()
            .map(|strip| (strip.source.clone(), strip.clone()))
            .collect();
        for strip in &self.strips {
            if !strip.present {
                continue;
            }
            let source = endpoint_key(&strip.id, &captures);
            if !valid_endpoint_key(&source) {
                continue;
            }
            let mut outputs: Vec<EndpointKey> = strip
                .selected_outputs
                .iter()
                .flatten()
                .map(|id| endpoint_key(id, &captures))
                .filter(valid_endpoint_key)
                .collect();
            if let Some(unresolved) = self.pending_outputs.get(&source) {
                for key in unresolved {
                    if valid_endpoint_key(key) && !outputs.contains(key) {
                        outputs.push(key.clone());
                    }
                }
            }
            strips.insert(
                source.clone(),
                StripProfile {
                    source,
                    name: (strip.name != strip.auto_name).then(|| strip.name.clone()),
                    outputs,
                    volume: strip.volume,
                    mix_gains: strip.mix_gains.clone(),
                    trim: strip.trim,
                    delay_ms: strip.delay_ms,
                    pan: strip.pan,
                    noise_suppression: strip.noise_suppression,
                    muted: strip.muted,
                    solo: strip.solo,
                    pfl: strip.pfl,
                },
            );
        }
        Profile {
            strips: strips.into_values().collect(),
            master: MasterProfile {
                cue_volume: self.master.cue_volume,
                cue_device: self.master.cue_device.clone(),
                cue_mix: self.master.cue_mix.clone(),
                cue_mix_enabled: self.master.cue_mix_enabled,
                mute_all: self.master.mute_all,
            },
            cable_assignment: self
                .vac
                .preferences()
                .into_iter()
                .filter(|(key, _)| !is_fallback_string_key(key))
                .collect(),
            shortcut: previous.shortcut,
            appearance: previous.appearance.clone(),
        }
    }

    fn save_active_profile(&mut self) {
        let Some(name) = self.settings.active_profile.clone() else {
            return;
        };
        let previous = self
            .settings
            .profiles
            .get(&name)
            .cloned()
            .unwrap_or_default();
        let profile = self.snapshot_profile(&previous);
        self.settings.profiles.insert(name, profile);
    }

    fn persistent_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.master.cue_volume.to_bits().hash(&mut hasher);
        self.master.cue_device.hash(&mut hasher);
        self.master.cue_mix.hash(&mut hasher);
        self.master.cue_mix_enabled.hash(&mut hasher);
        self.master.mute_all.hash(&mut hasher);
        self.appearance.gradient_from.hash(&mut hasher);
        self.appearance.gradient_to.hash(&mut hasher);
        self.appearance.angle.hash(&mut hasher);
        self.appearance.image.hash(&mut hasher);
        self.appearance.scrim.hash(&mut hasher);
        for strip in self.strips.iter().filter(|strip| strip.present) {
            strip.id.hash(&mut hasher);
            strip.name.hash(&mut hasher);
            strip.selected_outputs.hash(&mut hasher);
            strip.volume.to_bits().hash(&mut hasher);
            for (key, gain) in &strip.mix_gains {
                key.hash(&mut hasher);
                gain.to_bits().hash(&mut hasher);
            }
            strip.trim.to_bits().hash(&mut hasher);
            strip.delay_ms.hash(&mut hasher);
            strip.pan.hash(&mut hasher);
            strip.noise_suppression.hash(&mut hasher);
            strip.muted.hash(&mut hasher);
            strip.solo.hash(&mut hasher);
            strip.pfl.hash(&mut hasher);
        }
        for (key, number) in self.vac.preferences() {
            key.hash(&mut hasher);
            number.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn autosave(&mut self, now: Instant) {
        let fingerprint = self.persistent_fingerprint();
        match self.last_fingerprint {
            Some(previous) if previous == fingerprint => {}
            Some(_) => {
                self.last_fingerprint = Some(fingerprint);
                self.dirty_since = Some(now);
            }
            None => self.last_fingerprint = Some(fingerprint),
        }
        if !autosave_due(self.dirty_since, now, !self.profile_choices.is_empty()) {
            return;
        }
        self.save_active_profile();
        config::save_settings(&self.settings);
        self.dirty_since = None;
    }

    fn load_profile(&mut self, name: &str) {
        let profile = self
            .settings
            .profiles
            .get(name)
            .cloned()
            .unwrap_or_default();
        self.settings.active_profile = Some(name.to_string());
        self.profile_choices.clear();
        self.pending_outputs.clear();
        self.pending_profile = profile
            .strips
            .iter()
            .map(|strip| (strip.source.clone(), strip.clone()))
            .collect();
        self.vac.load_preferences(profile.cable_assignment.clone());
        self.apply_master_profile(&profile.master);
        self.apply_appearance(&profile.appearance);
        self.cache.clear();
        for strip in &mut self.strips {
            reset_strip(strip);
        }
        self.refresh(false);
    }

    fn switch_profile(&mut self, name: String) {
        if self.settings.active_profile.as_deref() == Some(name.as_str())
            || !self.settings.profiles.contains_key(&name)
        {
            return;
        }
        self.save_active_profile();
        self.load_profile(&name);
        config::save_settings(&self.settings);
    }

    fn create_profile(&mut self, name: String) {
        if name.is_empty() || self.settings.profiles.contains_key(&name) {
            return;
        }
        self.save_active_profile();
        let profile = self.snapshot_profile(&Profile::default());
        self.settings.profiles.insert(name.clone(), profile.clone());
        self.settings.active_profile = Some(name);
        self.apply_appearance(&profile.appearance);
        self.pending_profile.clear();
        self.profile_choices.clear();
        config::save_settings(&self.settings);
    }

    fn remove_profile(&mut self, name: String) {
        if name == DEFAULT_PROFILE || !self.settings.profiles.contains_key(&name) {
            return;
        }
        let active = self.settings.active_profile.as_deref() == Some(name.as_str());
        self.settings.profiles.remove(&name);
        if active {
            let fallback = DEFAULT_PROFILE.to_string();
            self.settings.profiles.entry(fallback.clone()).or_default();
            self.load_profile(&fallback);
        }
        config::save_settings(&self.settings);
    }

    fn reset_active_profile(&mut self) {
        let Some(name) = self.settings.active_profile.clone() else {
            return;
        };
        self.settings.profiles.insert(name, Profile::default());
        self.pending_profile.clear();
        self.profile_choices.clear();
        self.pending_outputs.clear();
        self.vac.load_preferences(BTreeMap::new());
        self.cache.clear();
        for strip in &mut self.strips {
            reset_strip(strip);
        }
        self.apply_master_profile(&MasterProfile::default());
        self.apply_appearance(&AppearanceProfile::default());
        config::save_settings(&self.settings);
    }

    fn handle_profile_action(&mut self, action: ProfileAction) {
        match action {
            ProfileAction::Switch(name) => self.switch_profile(name),
            ProfileAction::Save => {
                self.save_active_profile();
                config::save_settings(&self.settings);
            }
            ProfileAction::New => {
                self.new_profile = Some(NewProfileDialog {
                    name: String::new(),
                    focus: true,
                });
            }
            ProfileAction::Delete => {
                if let Some(name) = self.settings.active_profile.clone() {
                    self.delete_profile_request = Some(name);
                }
            }
            ProfileAction::Reset => self.reset_active_profile(),
            ProfileAction::SetShortcut { profile, slot } => {
                self.request_profile_shortcut(profile, slot);
            }
        }
    }

    fn request_profile_shortcut(&mut self, profile: String, slot: u8) {
        if let Some(owner) = shortcut_owner(&self.settings.profiles, &profile, slot) {
            self.pending_shortcut = Some(PendingShortcut {
                profile,
                slot,
                owner,
            });
            return;
        }
        assign_shortcut(&mut self.settings.profiles, &profile, slot);
        config::save_settings(&self.settings);
    }

    fn profile_ui(&mut self, ui: &mut egui::Ui) {
        let active = self
            .settings
            .active_profile
            .clone()
            .unwrap_or_else(|| DEFAULT_PROFILE.to_string());
        let mut selected = active.clone();
        egui::ComboBox::from_id_salt("profile_combo")
            .width(110.0)
            .selected_text(&active)
            .show_ui(ui, |ui| {
                for name in self.settings.profiles.keys() {
                    ui.selectable_value(&mut selected, name.clone(), name);
                }
            });
        if selected != active {
            self.profile_action = Some(ProfileAction::Switch(selected));
        }
        let profile_entries: Vec<(String, u8)> = self
            .settings
            .profiles
            .iter()
            .map(|(name, profile)| (name.clone(), profile.shortcut))
            .collect();
        let shortcut_owners: Vec<Option<String>> = (1..=MAX_SHORTCUT_SLOT)
            .map(|slot| profile_for_shortcut(&self.settings, slot))
            .collect();
        ui.menu_button(tr("Actions"), |ui| {
            if ui.button(tr("Enregistrer")).clicked() {
                self.profile_action = Some(ProfileAction::Save);
                ui.close();
            }
            if ui.button(tr("Nouveau…")).clicked() {
                self.profile_action = Some(ProfileAction::New);
                ui.close();
            }
            let can_delete = active != DEFAULT_PROFILE;
            if ui
                .add_enabled(can_delete, egui::Button::new(tr("Supprimer…")))
                .clicked()
            {
                self.profile_action = Some(ProfileAction::Delete);
                ui.close();
            }
            if ui.button(tr("Réinitialiser")).clicked() {
                self.profile_action = Some(ProfileAction::Reset);
                ui.close();
            }
            ui.separator();
            ui.menu_button(tr("Raccourcis"), |ui| {
                for (name, current) in &profile_entries {
                    ui.menu_button(name, |ui| {
                        if ui.selectable_label(*current == 0, tr("Aucun")).clicked() {
                            self.profile_action = Some(ProfileAction::SetShortcut {
                                profile: name.clone(),
                                slot: 0,
                            });
                            ui.close();
                        }
                        for slot in 1..=MAX_SHORTCUT_SLOT {
                            let owner = shortcut_owners[usize::from(slot) - 1]
                                .as_deref()
                                .filter(|owner| owner != name);
                            let mut label = shortcut_label(slot);
                            if owner.is_some() {
                                label.push_str(" •");
                            }
                            let response = ui.selectable_label(*current == slot, label);
                            let response = match owner {
                                Some(owner) => response.on_hover_text(
                                    crate::trf!("Déjà pris par « {owner} »", "{owner}" => owner),
                                ),
                                None => response,
                            };
                            if response.clicked() {
                                self.profile_action = Some(ProfileAction::SetShortcut {
                                    profile: name.clone(),
                                    slot,
                                });
                                ui.close();
                            }
                        }
                    });
                }
            });
        });
        ui.add_space((ui.available_width() - LANGUAGE_COMBO_WIDTH - 22.0).max(0.0));
        if appearance_button(ui, self.appearance_missing_image.is_some()).clicked() {
            self.appearance_open = true;
        }
        let detected = i18n::system_language();
        let auto_label = format!("🌐 AUTO {} {}", detected.code(), detected.name());
        let label = language_selector_text(self.settings.language);
        let mut selected = self.settings.language;
        ui.scope(|ui| {
            ui.spacing_mut().button_padding.x = LANGUAGE_COMBO_PADDING;
            let _ = egui::ComboBox::from_id_salt("language_combo")
                .width(LANGUAGE_COMBO_WIDTH)
                .truncate()
                .selected_text(ui::combo_text(ui, &label, LANGUAGE_COMBO_WIDTH))
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut selected, None, auto_label.as_str())
                        .clicked()
                    {
                        ui.close();
                    }
                    for language in i18n::Language::all() {
                        if ui
                            .selectable_value(&mut selected, Some(*language), language.label())
                            .clicked()
                        {
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text(tr("Langue"));
        });
        if selected != self.settings.language {
            i18n::set(selected.unwrap_or(detected));
            self.settings.language = selected;
            config::save_settings(&self.settings);
            self.rebuild_tray();
        }
    }

    fn appearance_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.appearance_open;
        let mut close = false;
        egui::Window::new(tr("Apparence"))
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                let mut changed = false;
                let missing_image = self.appearance_missing_image.clone();
                let mut from = self
                    .appearance
                    .gradient_from
                    .map(|value| f32::from(value) / 255.0);
                let mut to = self
                    .appearance
                    .gradient_to
                    .map(|value| f32::from(value) / 255.0);
                ui.label(tr("Dégradé"));
                changed |= egui::color_picker::color_edit_button_rgb(ui, &mut from).changed();
                changed |= egui::color_picker::color_edit_button_rgb(ui, &mut to).changed();
                ui.label(tr("Angle"));
                changed |= ui
                    .add(egui::Slider::new(&mut self.appearance.angle, 0..=359).suffix("°"))
                    .changed();
                ui.separator();
                ui.label(tr("Image de fond"));
                ui.horizontal(|ui| {
                    if ui.button(tr("Parcourir…")).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title(tr("Choisir une image de fond"))
                            .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                            .pick_file()
                        {
                            self.import_background(&path);
                        }
                    }
                    if let Some(image) = self.appearance.image.as_deref() {
                        ui.label(image);
                    }
                });
                if self.appearance.image.is_some() && ui.button(tr("Retirer l’image")).clicked() {
                    self.appearance.image = None;
                    self.appearance_cache = ui::background::BackgroundCache::default();
                    self.appearance_missing_image = None;
                    self.image_alert = false;
                    changed = true;
                }
                if let Some(file) = missing_image {
                    ui.colored_label(
                        ui::METER_WARN,
                        crate::trf!(
                            "Image introuvable : {file} — dégradé affiché à la place",
                            "{file}" => file
                        ),
                    );
                }
                ui.separator();
                changed |= ui
                    .add(egui::Slider::new(&mut self.appearance.scrim, 0..=255).text(tr("Voile")))
                    .changed();
                if changed {
                    self.appearance.gradient_from = color_from_f32(from);
                    self.appearance.gradient_to = color_from_f32(to);
                    self.sync_appearance();
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(tr("Réinitialiser")).clicked() {
                        self.apply_appearance(&AppearanceProfile::default());
                        self.sync_appearance();
                        config::save_settings(&self.settings);
                    }
                    if ui.button(tr("Fermer")).clicked() {
                        close = true;
                    }
                });
            });
        self.appearance_open = open && !close;
    }

    fn image_alert_ui(&mut self, ctx: &egui::Context) {
        if !self.image_alert {
            return;
        }
        let Some(file) = self.appearance_missing_image.clone() else {
            self.image_alert = false;
            return;
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new(tr("Image de fond introuvable"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(crate::trf!("Image introuvable : {file}", "{file}" => file));
                ui.label(tr("Dégradé affiché à la place."));
                if ui.button("OK").clicked() {
                    close = true;
                }
            });
        if !open || close {
            self.image_alert = false;
        }
    }

    fn set_start_with_windows(&mut self, enabled: bool) {
        if self.settings.start_with_windows == enabled {
            return;
        }
        let exe = match std::env::current_exe() {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(error) => {
                logging::log(
                    "démarrage",
                    &format!("Chemin de l’exécutable introuvable : {error}"),
                );
                return;
            }
        };
        let launcher = auto_launch::AutoLaunch::new(
            "audiok",
            &exe,
            auto_launch::WindowsEnableMode::CurrentUser,
            &[TRAY_ARG],
        );
        let result = if enabled {
            launcher.enable()
        } else {
            launcher.disable()
        };
        match result {
            Ok(()) => {
                self.settings.start_with_windows = enabled;
                if let Some(item) = &self.tray_start {
                    item.set_checked(enabled);
                }
                config::save_settings(&self.settings);
                logging::log(
                    "démarrage",
                    &format!(
                        "Démarrage avec Windows : {}",
                        if enabled { "activé" } else { "désactivé" }
                    ),
                );
            }
            Err(error) => {
                logging::log(
                    "démarrage",
                    &format!("Démarrage avec Windows impossible : {error}"),
                );
            }
        }
    }

    fn persist(&mut self) {
        self.save_active_profile();
        self.settings.cue_volume = self.master.cue_volume;
        self.settings.bypass = self.master.bypass;
        self.settings.cue_device = self.master.cue_device.clone();
        config::save_settings(&self.settings);
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
            let show = matches!(
                event,
                tray_icon::TrayIconEvent::DoubleClick {
                    button: tray_icon::MouseButton::Left,
                    ..
                } | tray_icon::TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                }
            );
            if show {
                let minimized = ctx.input(|state| state.viewport().minimized.unwrap_or(false));
                if !self.hidden && !minimized {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    self.hidden = true;
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.hidden = false;
                }
            }
        }
        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            match event.id.as_ref() {
                "tray-quit" => {
                    self.tray_quit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                "tray-start" => {
                    if let Some(item) = self.tray_start.clone() {
                        self.set_start_with_windows(item.is_checked());
                        item.set_checked(self.settings.start_with_windows);
                    }
                }
                _ => {}
            }
        }
    }

    fn help_ui(&mut self, ctx: &egui::Context) {
        if !self.help_open {
            return;
        }
        let mut open = true;
        let screen = ctx.input(|i| i.content_rect());
        let default_size = egui::vec2(
            (screen.width() * 0.9).clamp(520.0, 980.0),
            (screen.height() * 0.85).clamp(420.0, 760.0),
        );
        egui::Window::new(tr("Lexique / raccourcis"))
            .collapsible(false)
            .resizable(true)
            .default_size(default_size)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut start = self.settings.start_with_windows;
                    if ui
                        .checkbox(&mut start, tr("Démarrer avec Windows"))
                        .changed()
                    {
                        self.set_start_with_windows(start);
                    }
                    ui.separator();
                    ui.add_space(4.0);
                    for line in shortcut_lexicon_lines(&self.settings) {
                        if line.is_empty() {
                            ui.add_space(6.0);
                        } else if line.ends_with(':')
                            || line == tr("Globaux")
                            || line == tr("Contrôles")
                            || line == tr("Mode d’emploi")
                        {
                            ui.label(egui::RichText::new(line).strong());
                        } else {
                            ui.label(line);
                        }
                    }
                });
            });
        if !open {
            self.help_open = false;
        }
    }

    fn import_background(&mut self, source: &std::path::Path) {
        let Some(bytes) = ui::background::read_limited(source) else {
            logging::log(
                "apparence",
                "Lecture du fichier impossible ou trop volumineuse",
            );
            return;
        };
        let Some(decoded) = ui::background::decode(&bytes) else {
            logging::log("apparence", "Image invalide ou trop volumineuse");
            return;
        };
        if !matches!(decoded.extension.as_str(), "png" | "jpg" | "jpeg" | "webp") {
            logging::log("apparence", "Format d’image non pris en charge");
            return;
        }
        let name = background_file_name(&bytes, &decoded.extension);
        let directory = config::background_dir();
        if let Err(error) = std::fs::create_dir_all(&directory) {
            logging::log(
                "apparence",
                &format!("Création du dossier impossible : {error}"),
            );
            return;
        }
        let path = directory.join(&name);
        if !path.exists() {
            if let Err(error) = std::fs::write(&path, bytes) {
                logging::log("apparence", &format!("Copie impossible : {error}"));
                return;
            }
        }
        self.appearance.image = Some(name);
        self.appearance_cache = ui::background::BackgroundCache::default();
        self.appearance_missing_image = None;
        self.image_alert = false;
        self.sync_appearance();
        config::save_settings(&self.settings);
    }

    fn shortcut_confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_shortcut.clone() else {
            return;
        };
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(tr("Raccourci déjà utilisé"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(crate::trf!(
                    "{slot} est déjà lié à « {owner} ».",
                    "{slot}" => shortcut_label(pending.slot),
                    "{owner}" => pending.owner
                ));
                ui.label(crate::trf!(
                    "Le réassigner à « {profile} » ?",
                    "{profile}" => pending.profile
                ));
                ui.label(tr("Êtes-vous sûr ?"));
                ui.horizontal(|ui| {
                    confirm |= ui.button(tr("Oui")).clicked();
                    cancel |= ui.button(tr("Non")).clicked();
                });
            });
        if confirm {
            assign_shortcut(&mut self.settings.profiles, &pending.profile, pending.slot);
            config::save_settings(&self.settings);
            self.pending_shortcut = None;
        } else if cancel || !open {
            self.pending_shortcut = None;
        }
    }

    fn new_profile_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.new_profile.take() else {
            return;
        };
        let mut open = true;
        let mut create = false;
        let mut cancel = false;
        egui::Window::new(tr("Nouveau profil"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let response =
                    ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(220.0));
                if dialog.focus {
                    response.request_focus();
                    dialog.focus = false;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    create = true;
                }
                let name = dialog.name.trim();
                let valid = !name.is_empty() && !self.settings.profiles.contains_key(name);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(valid, egui::Button::new(tr("Créer")))
                        .clicked()
                    {
                        create = true;
                    }
                    cancel |= ui.button(tr("Annuler")).clicked();
                });
                if !name.is_empty() && !valid {
                    ui.label(egui::RichText::new(tr("Nom déjà utilisé")).small().weak());
                }
            });
        if create {
            let name = dialog.name.trim().to_string();
            let valid = !name.is_empty() && !self.settings.profiles.contains_key(&name);
            if valid {
                self.create_profile(name);
                return;
            }
        }
        if !cancel && open {
            self.new_profile = Some(dialog);
        }
    }

    fn delete_profile_ui(&mut self, ctx: &egui::Context) {
        let Some(name) = self.delete_profile_request.clone() else {
            return;
        };
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(tr("Supprimer le profil"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(crate::trf!("Supprimer « {name} » ?", "{name}" => name));
                ui.horizontal(|ui| {
                    confirm |= ui.button(tr("Supprimer")).clicked();
                    cancel |= ui.button(tr("Annuler")).clicked();
                });
            });
        if confirm {
            self.delete_profile_request = None;
            self.remove_profile(name);
        } else if cancel || !open {
            self.delete_profile_request = None;
        }
    }

    fn profile_choice_ui(&mut self, ctx: &egui::Context) {
        let Some(mut choice) = self.profile_choices.first().cloned() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut ignore = false;
        egui::Window::new(tr("Profil — session ambiguë"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let target = match &choice.target {
                    ProfileTarget::Source { .. } => tr("Plusieurs sessions correspondent à"),
                    ProfileTarget::Output { .. } => {
                        tr("Plusieurs sessions correspondent à la sortie")
                    }
                };
                ui.label(format!("{target} « {} ».", profile_key_label(&choice.key)));
                ui.label(tr("Choisissez celle à configurer :"));
                for (index, option) in choice.options.iter().enumerate() {
                    ui.radio_value(&mut choice.selected, index, &option.label);
                }
                ui.horizontal(|ui| {
                    apply |= ui.button(tr("Appliquer")).clicked();
                    ignore |= ui.button(tr("Ignorer")).clicked();
                });
            });
        let closed = apply || ignore || !open;
        if closed {
            self.profile_choices.remove(0);
        }
        if apply {
            self.apply_profile_choice(&choice);
        } else if !closed {
            if let Some(current) = self.profile_choices.first_mut() {
                current.selected = choice.selected;
            }
        }
    }

    fn apply_profile_choice(&mut self, choice: &ProfileChoice) {
        let Some(option) = choice.options.get(choice.selected) else {
            return;
        };
        match &choice.target {
            ProfileTarget::Source { saved } => {
                let captures = audio::devices::capture_pids();
                let output_map = runtime_ids(&self.outputs, &captures);
                let (ambiguous, unresolved) =
                    match self.strips.iter_mut().find(|strip| strip.id == option.id) {
                        Some(strip) => apply_strip_profile(strip, saved, &output_map),
                        None => {
                            self.pending_profile
                                .insert(choice.key.clone(), saved.clone());
                            return;
                        }
                    };
                if !unresolved.is_empty() {
                    self.pending_outputs.insert(choice.key.clone(), unresolved);
                }
                push_output_choices(
                    &mut self.profile_choices,
                    &option.id,
                    ambiguous,
                    &self.outputs,
                );
            }
            ProfileTarget::Output { strip_id, index } => {
                if let Some(strip) = self.strips.iter_mut().find(|strip| strip.id == *strip_id) {
                    if let Some(slot) = strip.selected_outputs.get_mut(*index) {
                        *slot = Some(option.id.clone());
                    }
                }
            }
        }
    }

    fn open_rename(&mut self) {
        let Some(endpoint) = self.rename_request.take() else {
            return;
        };
        let Some(found) = self.outputs.iter().find(|output| output.id == endpoint) else {
            return;
        };
        let captures = audio::devices::capture_pids();
        let key = alias_key(&endpoint_key(&endpoint, &captures));
        let buffer = self
            .settings
            .aliases
            .get(&key)
            .cloned()
            .unwrap_or_else(|| found.name.clone());
        self.rename = Some(Rename {
            endpoint,
            buffer,
            focus: true,
        });
    }

    fn rename_ui(&mut self, ctx: &egui::Context) {
        let Some(rename) = self.rename.as_mut() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let mut reset_default = false;
        egui::Window::new(tr("Renommer la sortie"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let response =
                    ui.add(egui::TextEdit::singleline(&mut rename.buffer).desired_width(240.0));
                if rename.focus {
                    response.request_focus();
                    rename.focus = false;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    apply = true;
                }
                ui.label(tr("Laisser vide pour rétablir le nom par défaut."));
                ui.horizontal(|ui| {
                    apply |= ui.button("OK").clicked();
                    reset_default |= ui.button(tr("Nom par défaut")).clicked();
                    cancel |= ui.button(tr("Annuler")).clicked();
                });
            });
        let endpoint = rename.endpoint.clone();
        let buffer = rename.buffer.clone();
        if reset_default {
            self.reset_alias(&endpoint);
        } else if apply {
            let name = buffer.trim();
            let captures = audio::devices::capture_pids();
            let key = alias_key(&endpoint_key(&endpoint, &captures));
            if name.is_empty() {
                self.settings.aliases.remove(&key);
                self.alias_labels.remove(&endpoint);
            } else {
                self.settings.aliases.insert(key, name.to_string());
                self.alias_labels.insert(endpoint.clone(), name.to_string());
            }
        }
        if apply || reset_default || cancel || !open {
            self.rename = None;
        }
    }

    fn reset_alias(&mut self, endpoint: &str) {
        let captures = audio::devices::capture_pids();
        let key = alias_key(&endpoint_key(endpoint, &captures));
        self.settings.aliases.remove(&key);
        self.alias_labels.remove(endpoint);
    }

    fn fit_window(&mut self, ctx: &egui::Context, item_spacing: f32) {
        let Some(inner_rect) = ctx.input(|i| i.viewport().inner_rect) else {
            return;
        };
        if inner_rect.height() <= 1.0 {
            return;
        }
        let monitor_size = ctx.input(|i| i.viewport().monitor_size);

        let content_width =
            LEFT_MARGIN + strips_content_width(self.strips.len(), item_spacing) + WIDTH_EPSILON;
        let max_width = match monitor_size {
            Some(size) => content_width
                .min((size.x - SCREEN_MARGIN).max(MIN_WINDOW_WIDTH))
                .max(MIN_WINDOW_WIDTH),
            None => content_width.max(MIN_WINDOW_WIDTH),
        };

        let measured = self.master.bottom_height > 0.0
            || self.strips.iter().any(|strip| strip.bottom_height > 0.0);
        let max_height = if measured {
            let strip_height = self
                .strips
                .iter()
                .map(|strip| strip.top_height + strip.bottom_height)
                .fold(0.0, f32::max);
            let master_height = self.master.top_height + self.master.bottom_height;
            let content_height = TOP_MARGIN + strip_height.max(master_height);
            match monitor_size {
                Some(size) => content_height.min((size.y - SCREEN_MARGIN).max(TOP_MARGIN)),
                None => content_height,
            }
        } else {
            inner_rect.height()
        };

        if (inner_rect.width() - max_width).abs() > RESIZE_EPSILON
            || (inner_rect.height() - max_height).abs() > RESIZE_EPSILON
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                max_width, max_height,
            )));
        }
    }

    fn route(&mut self, log_bypass: bool) -> HashMap<String, String> {
        let now = Instant::now();
        let apps = destination_apps(&self.strips, self.master.cue_device.as_deref());
        let pairs = app_pairs(&apps, &self.app_keys);
        let policy_apps = input_policy_apps(
            &apps,
            &self.strips,
            &self.redirected,
            now,
            STRIP_GRACE,
            audio::devices::process_alive,
        );
        let policy_pairs = app_pairs(&policy_apps, &self.app_keys);
        if self.master.bypass {
            self.vac.restore_captures();
        } else {
            self.vac.apply_policies(&policy_pairs);
        }
        let app_routes = self.vac.resolve(&pairs);
        let mut mapping: Vec<(String, String)> = app_routes
            .iter()
            .map(|(app, render)| (app.clone(), render.clone()))
            .collect();
        mapping.sort();
        if mapping != self.diag_app_routes {
            logging::log("diag", &format!("vac apps -> render: {mapping:?}"));
            self.diag_app_routes = mapping;
        }
        let routes =
            desired_routes_with_captures(&self.strips, &app_routes, &self.master, &self.captures);
        let sink = self.vac.sink().map(|endpoint| endpoint.id.clone());
        let app_pids = protected_redirect_pids(
            &self.strips,
            &self.redirected,
            now,
            STRIP_GRACE,
            audio::devices::process_alive,
        );
        self.apply_output_redirects(&app_pids, sink.as_deref());
        let mut desired: Vec<u32> = app_pids.iter().copied().collect();
        desired.sort_unstable();
        let mut actual: Vec<u32> = self.redirected.keys().copied().collect();
        actual.sort_unstable();
        if desired != self.diag_redirect_desired || actual != self.diag_redirect_actual {
            logging::log(
                "diag",
                &format!("redirection voulue={desired:?} redirigés={actual:?}"),
            );
            self.diag_redirect_desired = desired;
            self.diag_redirect_actual = actual;
        }
        if log_bypass {
            logging::log(
                "bypass",
                &format!(
                    "sink={} applis={} redirigées={} routes={}",
                    sink.as_deref().unwrap_or("-"),
                    app_pids.len(),
                    self.redirected.len(),
                    routes.len()
                ),
            );
            for route in &routes {
                logging::log(
                    "bypass",
                    &format!(
                        "plan {} -> {}",
                        route.input,
                        audio::engine::describe_outputs(&route.outputs)
                    ),
                );
            }
        }
        self.engine.reconcile(routes);
        app_routes
    }
}

fn should_show_image_alert(previous: &Option<String>, current: &Option<String>) -> bool {
    current.is_some() && current != previous
}

impl AudiokApp {
    fn rebuild_tray(&mut self) {
        self.tray = None;
        self.tray_start = None;
        if let Some((tray, start)) = build_tray_icon(self.settings.start_with_windows) {
            self.tray = Some(tray);
            self.tray_start = Some(start);
        }
    }
}

fn build_tray_icon(
    start_with_windows: bool,
) -> Option<(tray_icon::TrayIcon, tray_icon::menu::CheckMenuItem)> {
    use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};

    let image =
        eframe::icon_data::from_png_bytes(include_bytes!("../images/AudioK_imageApp_512.png"))
            .ok()?;
    let icon = tray_icon::Icon::from_rgba(image.rgba, image.width, image.height).ok()?;
    let menu = Menu::new();
    let start = CheckMenuItem::with_id(
        MenuId::new("tray-start"),
        tr("Démarrer avec Windows"),
        true,
        start_with_windows,
        None,
    );
    let quit = MenuItem::with_id(MenuId::new("tray-quit"), tr("Quitter"), true, None);
    menu.append(&start).ok()?;
    menu.append(&PredefinedMenuItem::separator()).ok()?;
    menu.append(&quit).ok()?;
    let tray = tray_icon::TrayIconBuilder::new()
        .with_tooltip("audiok")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .ok()?;
    Some((tray, start))
}

fn background_file_name(bytes: &[u8], extension: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("bg-{:016x}.{}", hasher.finish(), extension)
}

fn strips_content_width(strip_count: usize, item_spacing: f32) -> f32 {
    strip_count as f32 * (ui::COLUMN_WIDTH + item_spacing + STRIP_SPACING) + ui::MASTER_PANEL_WIDTH
}

fn profile_key_label(key: &EndpointKey) -> String {
    match key {
        EndpointKey::Hardware(id) => id.clone(),
        EndpointKey::App { exe, role } => {
            let file = exe.rsplit('\\').next().unwrap_or(exe);
            let role = match role {
                AppRole::Call => "appel",
                AppRole::Other => "autre",
            };
            format!("{file} ({role})")
        }
    }
}

fn endpoint_key(id: &str, captures: &HashSet<u32>) -> EndpointKey {
    let Some(pid) = endpoint_pid(id) else {
        return EndpointKey::Hardware(id.to_string());
    };
    let exe = audio::devices::process_path(pid)
        .unwrap_or_else(|| audio::devices::process_name(pid))
        .to_lowercase();
    let role = if captures.contains(&pid) {
        AppRole::Call
    } else {
        AppRole::Other
    };
    EndpointKey::App { exe, role }
}

fn alias_key(key: &EndpointKey) -> String {
    match key {
        EndpointKey::Hardware(id) => id.clone(),
        EndpointKey::App { exe, role } => {
            let role = match role {
                AppRole::Call => "call",
                AppRole::Other => "other",
            };
            format!("app:{exe}|{role}")
        }
    }
}

fn app_stable_keys(endpoints: &[AudioEndpoint], captures: &HashSet<u32>) -> HashMap<u32, String> {
    endpoints
        .iter()
        .filter_map(|endpoint| {
            let pid = endpoint_pid(&endpoint.id)?;
            if pid == 0 {
                return None;
            }
            match endpoint_key(&endpoint.id, captures) {
                EndpointKey::App { exe, .. } => Some((pid, format!("app:{exe}"))),
                EndpointKey::Hardware(_) => None,
            }
        })
        .collect()
}

fn app_pairs<'a>(apps: &[&'a str], keys: &'a HashMap<u32, String>) -> Vec<(&'a str, &'a str)> {
    apps.iter()
        .map(|&app| {
            let key = endpoint_pid(app)
                .and_then(|pid| keys.get(&pid))
                .map(String::as_str)
                .unwrap_or(app);
            (app, key)
        })
        .collect()
}

fn resolved_aliases(
    aliases: &HashMap<String, String>,
    endpoints: &[AudioEndpoint],
    captures: &HashSet<u32>,
) -> HashMap<String, String> {
    endpoints
        .iter()
        .filter_map(|endpoint| {
            let key = alias_key(&endpoint_key(&endpoint.id, captures));
            aliases
                .get(&key)
                .map(|name| (endpoint.id.clone(), name.clone()))
        })
        .collect()
}

fn runtime_ids(
    endpoints: &[AudioEndpoint],
    captures: &HashSet<u32>,
) -> HashMap<EndpointKey, Vec<String>> {
    let mut grouped: HashMap<EndpointKey, Vec<(u32, String)>> = HashMap::new();
    for endpoint in endpoints {
        let key = endpoint_key(&endpoint.id, captures);
        let pid = endpoint_pid(&endpoint.id).unwrap_or(0);
        grouped
            .entry(key)
            .or_default()
            .push((pid, endpoint.id.clone()));
    }
    grouped
        .into_iter()
        .map(|(key, mut items)| {
            items.sort();
            (key, items.into_iter().map(|(_, id)| id).collect())
        })
        .collect()
}

fn apply_strip_profile(
    strip: &mut ui::strip::Strip,
    saved: &StripProfile,
    output_map: &HashMap<EndpointKey, Vec<String>>,
) -> (Vec<AmbiguousOutput>, Vec<EndpointKey>) {
    strip.name = saved
        .name
        .clone()
        .unwrap_or_else(|| strip.auto_name.clone());
    strip.volume = saved.volume;
    strip.mix_gains = saved.mix_gains.clone();
    strip.trim = saved.trim;
    strip.delay_ms = saved.delay_ms;
    strip.pan = saved.pan;
    strip.noise_suppression = saved.noise_suppression;
    strip.muted = saved.muted;
    strip.solo = saved.solo;
    strip.pfl = saved.pfl;
    let (outputs, ambiguous, unresolved) = resolve_outputs(&saved.outputs, output_map);
    strip.selected_outputs = outputs;
    (ambiguous, unresolved)
}

fn output_ids(
    key: &EndpointKey,
    output_map: &HashMap<EndpointKey, Vec<String>>,
) -> Option<Vec<String>> {
    if let Some(ids) = output_map.get(key) {
        return Some(ids.clone());
    }
    let EndpointKey::App { exe, .. } = key else {
        return None;
    };
    let mut ids = Vec::new();
    for (candidate, values) in output_map {
        if let EndpointKey::App { exe: other, .. } = candidate {
            if other == exe {
                for id in values {
                    if !ids.contains(id) {
                        ids.push(id.clone());
                    }
                }
            }
        }
    }
    (!ids.is_empty()).then_some(ids)
}

fn resolve_outputs(
    saved: &[EndpointKey],
    output_map: &HashMap<EndpointKey, Vec<String>>,
) -> (Vec<Option<String>>, Vec<AmbiguousOutput>, Vec<EndpointKey>) {
    let mut outputs: Vec<Option<String>> = Vec::new();
    let mut ambiguous = Vec::new();
    let mut unresolved = Vec::new();
    for key in saved {
        match output_ids(key, output_map) {
            Some(ids) if ids.len() > 1 => {
                ambiguous.push(AmbiguousOutput {
                    index: outputs.len(),
                    key: key.clone(),
                    ids,
                });
                outputs.push(None);
            }
            Some(ids) => {
                let id = &ids[0];
                if !outputs.iter().flatten().any(|existing| existing == id) {
                    outputs.push(Some(id.clone()));
                }
            }
            None => unresolved.push(key.clone()),
        }
    }
    if outputs.is_empty() {
        outputs.push(None);
    }
    (outputs, ambiguous, unresolved)
}

fn choice_options(ids: &[String], endpoints: &[AudioEndpoint]) -> Vec<ProfileOption> {
    ids.iter()
        .map(|id| {
            let name = endpoints
                .iter()
                .find(|endpoint| &endpoint.id == id)
                .map(|endpoint| endpoint.name.as_str())
                .unwrap_or(id);
            ProfileOption {
                id: id.clone(),
                label: format!("{name} — PID {}", endpoint_pid(id).unwrap_or(0)),
            }
        })
        .collect()
}

fn push_output_choices(
    choices: &mut Vec<ProfileChoice>,
    strip_id: &str,
    ambiguous: Vec<AmbiguousOutput>,
    endpoints: &[AudioEndpoint],
) {
    for item in ambiguous {
        choices.push(ProfileChoice {
            key: item.key,
            target: ProfileTarget::Output {
                strip_id: strip_id.to_string(),
                index: item.index,
            },
            options: choice_options(&item.ids, endpoints),
            selected: 0,
        });
    }
}

fn autosave_due(dirty_since: Option<Instant>, now: Instant, pending: bool) -> bool {
    !pending && dirty_since.is_some_and(|since| now.duration_since(since) >= AUTOSAVE_DELAY)
}

fn reset_strip(strip: &mut ui::strip::Strip) {
    strip.name = strip.auto_name.clone();
    strip.selected_outputs = vec![None];
    strip.volume = 1.0;
    strip.mix_gains.clear();
    strip.trim = 1.0;
    strip.delay_ms = 0;
    strip.pan = 0;
    strip.noise_suppression = false;
    strip.muted = false;
    strip.solo = false;
    strip.pfl = false;
}

fn visible_inputs(inputs: Vec<AudioEndpoint>) -> Vec<AudioEndpoint> {
    inputs
        .into_iter()
        .filter(|input| {
            input.kind != audio::devices::EndpointKind::Hardware || !audio::vac::is_vac(&input.name)
        })
        .collect()
}

fn visible_outputs(outputs: Vec<AudioEndpoint>) -> Vec<AudioEndpoint> {
    outputs
        .into_iter()
        .filter(|output| {
            output.kind != audio::devices::EndpointKind::Hardware
                || !audio::vac::is_vac(&output.name)
        })
        .collect()
}

fn app_selections(strips: &[ui::strip::Strip]) -> Vec<&str> {
    let mut apps = Vec::new();
    for strip in strips {
        for output in strip.selected_outputs.iter().flatten() {
            if output.starts_with("app:") {
                apps.push(output.as_str());
            }
        }
    }
    apps
}

fn format_previous(values: &[Option<String>; 3]) -> String {
    values
        .iter()
        .map(|value| value.as_deref().unwrap_or("-"))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn should_redirect(redirected: bool, retry_at: Option<Instant>, now: Instant) -> bool {
    !redirected || retry_at.is_some_and(|retry| now >= retry)
}

fn should_restore(retry_at: Option<Instant>, now: Instant) -> bool {
    retry_at.is_none_or(|retry| now >= retry)
}

fn mismatched_roles(current: &[Option<String>; 3], sink: &str) -> Vec<String> {
    current
        .iter()
        .enumerate()
        .filter(|(_, value)| value.as_deref() != Some(sink))
        .map(|(role, value)| format!("rôle {role}: {}", value.as_deref().unwrap_or("-")))
        .collect()
}

fn verify_redirect(pid: u32, sink: &str) -> Result<(), String> {
    let mismatched = mismatched_roles(&audio::policy::get_default_render(pid), sink);
    if mismatched.is_empty() {
        Ok(())
    } else {
        Err(mismatched.join(", "))
    }
}

fn destination_apps<'a>(strips: &'a [ui::strip::Strip], cue: Option<&'a str>) -> Vec<&'a str> {
    let mut apps = app_selections(strips);
    if let Some(cue) = cue {
        if cue.starts_with("app:") && !apps.contains(&cue) {
            apps.push(cue);
        }
    }
    apps
}

fn input_policy_apps<'a>(
    apps: &'a [&'a str],
    strips: &[ui::strip::Strip],
    redirected: &HashMap<u32, [Option<String>; 3]>,
    now: Instant,
    grace: Duration,
    is_alive: impl Fn(u32) -> bool,
) -> Vec<&'a str> {
    apps.iter()
        .copied()
        .filter(|app| {
            let Some(pid) = endpoint_pid(app) else {
                return true;
            };
            if !is_alive(pid) {
                return false;
            }
            let Some(strip) = strips
                .iter()
                .find(|strip| endpoint_pid(&strip.id) == Some(pid))
            else {
                return true;
            };
            strip.present
                || (strip
                    .last_seen
                    .is_some_and(|seen| now.duration_since(seen) < grace)
                    && redirected.contains_key(&pid))
        })
        .collect()
}

fn resolve_route_output(output: &str, app_routes: &HashMap<String, String>) -> Option<String> {
    if output.starts_with("app:") {
        app_routes.get(output).cloned()
    } else {
        Some(output.to_string())
    }
}

fn mix_choices(
    strips: &[ui::strip::Strip],
    cue: Option<&str>,
    outputs: &[AudioEndpoint],
) -> Vec<String> {
    let mut mixes: Vec<String> = Vec::new();
    for strip in strips
        .iter()
        .filter(|strip| strip.present && strip.id != "session:0")
    {
        for output in strip.selected_outputs.iter().flatten() {
            if Some(output.as_str()) != cue
                && outputs.iter().any(|endpoint| &endpoint.id == output)
                && !mixes.contains(output)
            {
                mixes.push(output.clone());
            }
        }
    }
    mixes
}

#[cfg(test)]
fn desired_routes(
    strips: &[ui::strip::Strip],
    app_routes: &HashMap<String, String>,
    master: &ui::master::Master,
) -> Vec<audio::engine::RouteSpec> {
    desired_routes_with_captures(strips, app_routes, master, &HashSet::new())
}

fn output_gain(strip: &ui::strip::Strip, output: &str, captures: &HashSet<u32>) -> f32 {
    let key = endpoint_key(output, captures);
    if let Some(gain) = strip.mix_gains.get(&key) {
        return *gain;
    }
    if let EndpointKey::App { exe, .. } = &key {
        if let Some(gain) = strip.mix_gains.iter().find_map(|(saved, gain)| {
            matches!(saved, EndpointKey::App { exe: other, .. } if other == exe).then_some(*gain)
        }) {
            return gain;
        }
    }
    strip.volume
}

fn desired_routes_with_captures(
    strips: &[ui::strip::Strip],
    app_routes: &HashMap<String, String>,
    master: &ui::master::Master,
    captures: &HashSet<u32>,
) -> Vec<audio::engine::RouteSpec> {
    let any_solo = strips
        .iter()
        .any(|strip| strip.present && strip.id != "session:0" && strip.solo);
    let cue = master
        .cue_device
        .as_deref()
        .and_then(|cue| resolve_route_output(cue, app_routes));
    strips
        .iter()
        .filter_map(|strip| {
            if !strip.present || strip.id == "session:0" {
                return None;
            }
            if master.bypass {
                return Some(audio::engine::RouteSpec {
                    input: strip.id.clone(),
                    input_gain: 1.0,
                    delay_ms: 0,
                    pan: 0,
                    noise_suppression: false,
                    outputs: Vec::new(),
                });
            }
            let cut = strip.muted || (any_solo && !strip.solo);
            let own_app = ui::strip::own_app(&strip.id);
            let mut outputs: Vec<audio::engine::OutputSpec> = Vec::new();
            for output in strip.selected_outputs.iter().flatten() {
                if own_app.as_deref() == Some(output.as_str()) {
                    continue;
                }
                let Some(id) = resolve_route_output(output, app_routes) else {
                    continue;
                };
                if !outputs.iter().any(|existing| existing.id == id) {
                    let gain = if cut || master.mute_all {
                        0.0
                    } else {
                        output_gain(strip, output, captures)
                    };
                    outputs.push(audio::engine::OutputSpec { id, gain });
                }
            }
            if let Some(cue) = cue.as_ref() {
                if !outputs.iter().any(|existing| existing.id == *cue) {
                    let cue_gain = if master.mute_all {
                        0.0
                    } else if !master.cue_mix_enabled {
                        if strip.pfl {
                            master.cue_volume
                        } else {
                            0.0
                        }
                    } else if cut {
                        0.0
                    } else {
                        match master.cue_mix.as_deref() {
                            Some(mix)
                                if strip
                                    .selected_outputs
                                    .iter()
                                    .flatten()
                                    .any(|output| output == mix) =>
                            {
                                master.cue_volume * output_gain(strip, mix, captures)
                            }
                            _ => 0.0,
                        }
                    };
                    outputs.push(audio::engine::OutputSpec {
                        id: cue.clone(),
                        gain: cue_gain,
                    });
                }
            }
            Some(audio::engine::RouteSpec {
                input: strip.id.clone(),
                input_gain: strip.trim,
                delay_ms: strip.delay_ms,
                pan: strip.pan,
                noise_suppression: strip.noise_suppression && strip.id.starts_with("input:"),
                outputs,
            })
        })
        .collect()
}

fn strip_app_pids(strips: &[ui::strip::Strip]) -> HashSet<u32> {
    strips
        .iter()
        .filter(|strip| strip.present)
        .filter_map(|strip| endpoint_pid(&strip.id))
        .filter(|pid| *pid != 0)
        .collect()
}

fn protected_redirect_pids(
    strips: &[ui::strip::Strip],
    redirected: &HashMap<u32, [Option<String>; 3]>,
    now: Instant,
    grace: Duration,
    is_alive: impl Fn(u32) -> bool,
) -> HashSet<u32> {
    let mut pids = strip_app_pids(strips);
    for strip in strips {
        if strip.present {
            continue;
        }
        let in_grace = strip
            .last_seen
            .is_some_and(|seen| now.duration_since(seen) < grace);
        if !in_grace {
            continue;
        }
        let Some(pid) = endpoint_pid(&strip.id) else {
            continue;
        };
        if pid != 0 && redirected.contains_key(&pid) && is_alive(pid) {
            pids.insert(pid);
        }
    }
    pids
}

fn sync_strips(
    sources: &[AudioEndpoint],
    current: Vec<ui::strip::Strip>,
    mut cache: HashMap<String, ui::strip::Strip>,
    trim: bool,
    now: Instant,
    grace: Duration,
) -> (Vec<ui::strip::Strip>, HashMap<String, ui::strip::Strip>) {
    for strip in current {
        cache.insert(strip.id.clone(), strip);
    }

    let mut strips = Vec::with_capacity(sources.len());
    for source in sources {
        let mut strip = cache
            .remove(&source.id)
            .unwrap_or_else(|| ui::strip::Strip::new(source.id.clone(), source.name.clone()));
        if strip.name == strip.auto_name {
            strip.name = source.name.clone();
        }
        strip.auto_name = source.name.clone();
        strip.last_seen = Some(now);
        strip.present = true;
        if trim {
            // Drop empty selector slots so "Rafraîchir" cleans up rows that are not
            // assigned. Assigned outputs are kept even when absent so they rebind
            // automatically when the endpoint returns.
            strip.selected_outputs.retain(Option::is_some);
        }
        strips.push(strip);
    }

    if !trim {
        for strip in cache.values() {
            if strip
                .last_seen
                .is_some_and(|seen| now.duration_since(seen) < grace)
            {
                let mut strip = strip.clone();
                strip.present = false;
                strips.push(strip);
            }
        }
    }

    if cache.len() > CACHE_CAP {
        let mut entries: Vec<(Option<Instant>, String)> = cache
            .iter()
            .map(|(id, strip)| (strip.last_seen, id.clone()))
            .collect();
        entries.sort_by_key(|(seen, _)| *seen);
        for (_, id) in entries.into_iter().take(cache.len() - CACHE_CAP) {
            cache.remove(&id);
        }
    }

    strips.sort_by_cached_key(strip_sort_key);
    (strips, cache)
}

fn strip_sort_key(strip: &ui::strip::Strip) -> (u8, u8, String, u8, u32, String) {
    let present = u8::from(!strip.present);
    if !(strip.id.starts_with("session:") || strip.id.starts_with("app:")) {
        return (
            present,
            0,
            strip.name.to_lowercase(),
            0,
            0,
            strip.id.clone(),
        );
    }
    let pid = strip
        .id
        .rsplit_once(':')
        .and_then(|(_, pid)| pid.parse().ok())
        .unwrap_or(0);
    let (base, kind) = match strip.name.strip_suffix(" (appel)") {
        Some(base) => (base, 0),
        None => (strip.name.as_str(), 1),
    };
    (present, 1, base.to_lowercase(), kind, pid, strip.id.clone())
}

impl eframe::App for AudiokApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_tray(ctx);
        if self.hidden && self.hide_after_start {
            let visible = ctx.input(|state| state.viewport().visible());
            if visible == Some(false) {
                self.hide_after_start = false;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        }
        let actions = self
            .hotkeys
            .get_or_insert_with(|| hotkeys::Hotkeys::start(ctx.clone()))
            .poll();
        for action in actions {
            match action {
                hotkeys::Action::Profile(slot) => {
                    if let Some(name) = profile_for_shortcut(&self.settings, slot) {
                        self.profile_action = Some(ProfileAction::Switch(name));
                    }
                }
                hotkeys::Action::MuteMics => toggle_mic_mutes(&mut self.strips),
                hotkeys::Action::MuteAll => self.master.mute_all = !self.master.mute_all,
            }
        }
        if let Some(action) = self.profile_action.take() {
            self.handle_profile_action(action);
        }
        if self.hidden {
            let now = Instant::now();
            if self
                .last_refresh
                .is_none_or(|last| now.duration_since(last) >= REFRESH_INTERVAL)
            {
                self.refresh(false);
                self.last_refresh = Some(now);
            }
            self.route(false);
        }
        ctx.request_repaint_after(METER_INTERVAL);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|state| state.viewport().close_requested()) {
            if self.tray_quit {
                self.tray_quit = false;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.hidden = true;
                self.persist();
            }
        }
        let missing_image =
            ui::background::paint(ui.ctx(), &self.appearance, &mut self.appearance_cache);
        if missing_image != self.appearance_missing_image {
            if let Some(file) = &missing_image {
                logging::log(
                    "apparence",
                    &format!("Image de fond introuvable : {file} — dégradé affiché"),
                );
            }
            if should_show_image_alert(&self.appearance_missing_image, &missing_image) {
                self.image_alert = true;
            }
            self.appearance_missing_image = missing_image;
        }
        let text_color = ui::background::text_color_for_content(
            &ctx,
            &self.appearance,
            &self.appearance_cache,
            ctx.viewport_rect(),
        );
        apply_content_text_visuals(ui, text_color);
        let now = Instant::now();
        if self
            .last_refresh
            .is_none_or(|last| now.duration_since(last) >= REFRESH_INTERVAL)
        {
            self.refresh(false);
            self.last_refresh = Some(now);
        }

        ui.add_space(TOP_MARGIN);
        self.fit_window(ui.ctx(), ui.spacing().item_spacing.x);

        let bypass_before = self.master.bypass;
        let mixes = mix_choices(
            &self.strips,
            self.master.cue_device.as_deref(),
            &self.outputs,
        );
        let (refresh, bypass_rect, master_rect) = egui::Panel::right("master_panel")
            .resizable(false)
            .exact_size(ui::MASTER_PANEL_WIDTH)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                self.master.ui(
                    ui,
                    &self.outputs,
                    &self.alias_labels,
                    &mixes,
                    &self.appearance,
                    &self.appearance_cache,
                    &mut self.rename_request,
                    &mut self.reset_request,
                )
            })
            .inner;
        let bypass = self.master.bypass;
        let active_veil = ui::active_veil(ui.ctx(), !bypass);
        if !bypass {
            ui.painter().rect_filled(master_rect, 4.0, active_veil);
        }
        ui::background::paint_outline(
            ui.ctx(),
            ui.painter(),
            &self.appearance,
            &self.appearance_cache,
            master_rect,
        );

        if bypass != bypass_before {
            logging::log(
                "bypass",
                if bypass {
                    "activé (sorties Windows restaurées)"
                } else {
                    "désactivé (routage actif)"
                },
            );
        }

        let captures = self.captures.clone();
        let cue_mix = self.master.cue_mix.clone();
        egui::ScrollArea::horizontal().show(ui, |ui| {
            let strips_rect = ui
                .horizontal_top(|ui| {
                    ui.add_space(LEFT_MARGIN);
                    let mut rect: Option<egui::Rect> = None;
                    for strip in &mut self.strips {
                        let gain_edit = match cue_mix.as_deref() {
                            None => ui::strip::GainEdit::Volume,
                            Some(mix)
                                if strip
                                    .selected_outputs
                                    .iter()
                                    .flatten()
                                    .any(|output| output.as_str() == mix) =>
                            {
                                ui::strip::GainEdit::Mix(endpoint_key(mix, &captures))
                            }
                            Some(_) => ui::strip::GainEdit::Disabled,
                        };
                        let strip_rect = strip.ui(
                            ui,
                            &self.outputs,
                            &self.alias_labels,
                            &mut self.rename_request,
                            &mut self.reset_request,
                            self.master.cue_device.as_deref(),
                            gain_edit,
                        );
                        let visible_rect =
                            ui::strip_visible_rect(strip_rect, ui::footer_line_y(ui, strip_rect));
                        if !bypass {
                            ui.painter().rect_filled(visible_rect, 4.0, active_veil);
                        }
                        ui::background::paint_outline(
                            ui.ctx(),
                            ui.painter(),
                            &self.appearance,
                            &self.appearance_cache,
                            visible_rect,
                        );
                        rect = Some(rect.map_or(strip_rect, |acc| acc.union(strip_rect)));
                        ui.add_space(STRIP_SPACING);
                    }
                    rect
                })
                .inner;
            if let Some(rect) = strips_rect {
                let line_y = ui::footer_line_y(ui, rect);
                let line_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.left(), line_y),
                    egui::pos2(rect.right(), line_y + 1.0),
                );
                let line_color = ui::background::contrast_color(
                    ui.ctx(),
                    &self.appearance,
                    &self.appearance_cache,
                    line_rect,
                );
                let veil = ui::bypass_veil(ui.ctx(), bypass);
                if veil.a() > 0 {
                    let full = ui.ctx().viewport_rect();
                    let painter = ui.painter().with_clip_rect(full);
                    for rect in ui::veil_rects(full, bypass_rect) {
                        painter.rect_filled(rect, 0.0, veil);
                    }
                }
                if ui::strips_footer(ui, rect, line_color, |ui| self.profile_ui(ui)) {
                    self.help_open = !self.help_open;
                }
            }
        });

        if let Some(endpoint) = self.reset_request.take() {
            self.reset_alias(&endpoint);
        }
        self.open_rename();
        self.rename_ui(ui.ctx());
        self.shortcut_confirm_ui(ui.ctx());
        self.new_profile_ui(ui.ctx());
        self.delete_profile_ui(ui.ctx());
        self.profile_choice_ui(ui.ctx());
        self.appearance_ui(ui.ctx());
        self.image_alert_ui(ui.ctx());
        self.help_ui(ui.ctx());

        if refresh {
            self.refresh(true);
            self.last_refresh = Some(Instant::now());
        }

        let app_routes = self.route(bypass != bypass_before);

        let cue_output = self
            .master
            .cue_device
            .as_deref()
            .and_then(|cue| resolve_route_output(cue, &app_routes));
        let cue_level = cue_output
            .as_deref()
            .map(|id| self.engine.output_level(id))
            .unwrap_or(0.0);
        self.master.cue_level = cue_level.max(self.master.cue_level * 0.9);

        let mix_level = self
            .master
            .cue_mix
            .as_deref()
            .and_then(|key| resolve_route_output(key, &app_routes))
            .map(|id| self.engine.output_level(&id))
            .unwrap_or(0.0);
        self.master.mix_level = mix_level.max(self.master.mix_level * 0.9);

        for strip in &mut self.strips {
            let level = if strip.present {
                self.engine.level(&strip.id)
            } else {
                0.0
            };
            strip.level = level.max(strip.level * 0.9);
        }

        self.autosave(Instant::now());

        ui.ctx().request_repaint_after(METER_INTERVAL);
    }

    fn on_exit(&mut self) {
        self.restore_output_redirects();
        self.vac.restore_captures();
        self.persist();
    }
}

fn acquire_single_instance() -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Threading::CreateMutexW;

    let Ok(handle) = (unsafe {
        CreateMutexW(
            None,
            true,
            windows::core::w!("Local\\audiok-single-instance"),
        )
    }) else {
        return Some(HANDLE::default());
    };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        None
    } else {
        Some(handle)
    }
}

fn warn_already_running() {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};

    let text = windows::core::HSTRING::from(tr("audiok est déjà lancé"));
    unsafe {
        MessageBoxW(
            None,
            &text,
            windows::core::w!("audiok"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn main() -> eframe::Result {
    let settings = config::load_settings();
    i18n::set(settings.language.unwrap_or_else(i18n::system_language));
    let Some(_instance) = acquire_single_instance() else {
        warn_already_running();
        return Ok(());
    };
    logging::init();
    let start_in_tray = std::env::args().any(|arg| arg == TRAY_ARG);
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([640.0, 600.0])
        .with_resizable(false)
        .with_maximize_button(false)
        .with_visible(!start_in_tray);
    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../images/AudioK_imageApp_512.png"));
    let viewport = match icon {
        Ok(icon) => viewport.with_icon(icon),
        Err(_) => viewport,
    };
    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "audiok",
        native_options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut app = AudiokApp::with_settings(settings, start_in_tray);
            app.hidden = start_in_tray;
            app.rebuild_tray();
            Ok(Box::new(app))
        }),
    )
}

fn apply_content_text_visuals(ui: &mut egui::Ui, text_color: egui::Color32) {
    let mut visuals = ui.visuals().clone();
    visuals.widgets.noninteractive.fg_stroke.color = text_color;
    visuals.weak_text_color = Some(text_color.gamma_multiply(0.65));
    *ui.visuals_mut() = visuals;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_selector_shows_code_or_globe() {
        assert_eq!(language_selector_text(Some(i18n::Language::Fr)), "FR");
        assert_eq!(language_selector_text(Some(i18n::Language::Ja)), "JA");
        assert_eq!(language_selector_text(None), "🌐");
    }

    #[test]
    fn language_selector_fits_two_letter_code() {
        egui::__run_test_ui(|ui| {
            let overhead =
                2.0 * LANGUAGE_COMBO_PADDING + ui.spacing().icon_width + ui.spacing().icon_spacing;
            let available = LANGUAGE_COMBO_WIDTH - overhead;
            let font = egui::TextStyle::Button.resolve(ui.style());
            let width = ui
                .painter()
                .layout_no_wrap("FR".to_owned(), font, egui::Color32::WHITE)
                .size()
                .x;
            assert!(width <= available, "code {width} px > {available} px");
        });
    }

    #[test]
    fn content_text_visuals_leave_widget_colors_untouched() {
        let text_color = egui::Color32::RED;
        egui::__run_test_ui(|ui| {
            let inactive = ui.visuals().widgets.inactive.text_color();
            apply_content_text_visuals(ui, text_color);
            assert_eq!(ui.visuals().override_text_color, None);
            assert_eq!(ui.visuals().widgets.noninteractive.text_color(), text_color);
            assert_eq!(ui.visuals().widgets.inactive.text_color(), inactive);
        });
    }

    #[test]
    fn appearance_scrim_keeps_full_range() {
        let mut app = AudiokApp::default();
        let appearance = AppearanceProfile {
            scrim: 0,
            ..Default::default()
        };
        app.apply_appearance(&appearance);
        assert_eq!(app.appearance.scrim, 0);
        let appearance = AppearanceProfile {
            scrim: 255,
            ..Default::default()
        };
        app.apply_appearance(&appearance);
        assert_eq!(app.appearance.scrim, 255);
    }

    #[test]
    fn app_icon_decodes() {
        let icon =
            eframe::icon_data::from_png_bytes(include_bytes!("../images/AudioK_imageApp_512.png"))
                .expect("icône PNG invalide");
        assert_eq!((icon.width, icon.height), (512, 512));
    }

    #[test]
    fn image_alert_opens_only_on_new_missing_file() {
        let missing = Some("bg-a.png".to_string());
        assert!(should_show_image_alert(&None, &missing));
        assert!(!should_show_image_alert(&missing, &missing));
        assert!(!should_show_image_alert(&missing, &None));
        assert!(should_show_image_alert(
            &missing,
            &Some("bg-b.png".to_string())
        ));
    }

    #[test]
    fn background_file_name_tracks_content_and_extension() {
        assert_eq!(
            background_file_name(b"image", "png"),
            background_file_name(b"image", "png")
        );
        assert_ne!(
            background_file_name(b"image", "png"),
            background_file_name(b"other", "png")
        );
        assert_ne!(
            background_file_name(b"image", "png"),
            background_file_name(b"image", "jpg")
        );
    }

    #[test]
    fn shortcut_assignment_is_unique() {
        let profile = |shortcut| Profile {
            shortcut,
            ..Default::default()
        };
        let mut profiles =
            BTreeMap::from([("A".to_string(), profile(1)), ("B".to_string(), profile(2))]);
        assign_shortcut(&mut profiles, "B", 1);
        assert_eq!(profiles["A"].shortcut, 0);
        assert_eq!(profiles["B"].shortcut, 1);
        assign_shortcut(&mut profiles, "B", 0);
        assert_eq!(profiles["B"].shortcut, 0);
    }

    #[test]
    fn shortcut_conflict_reports_owner() {
        let profile = |shortcut| Profile {
            shortcut,
            ..Default::default()
        };
        let profiles =
            BTreeMap::from([("A".to_string(), profile(1)), ("B".to_string(), profile(2))]);
        assert_eq!(shortcut_owner(&profiles, "B", 1).as_deref(), Some("A"));
        assert_eq!(shortcut_owner(&profiles, "A", 1), None);
        assert_eq!(shortcut_owner(&profiles, "B", 3), None);
        assert_eq!(shortcut_owner(&profiles, "B", 0), None);
    }

    #[test]
    fn shortcut_labels_include_zero() {
        assert_eq!(shortcut_label(1), "Ctrl+Maj+1");
        assert_eq!(shortcut_label(MAX_SHORTCUT_SLOT), "Ctrl+Maj+0");
    }

    #[test]
    fn shortcut_resolves_profile() {
        let settings = config::Settings {
            profiles: BTreeMap::from([
                (
                    "Live".to_string(),
                    Profile {
                        shortcut: 3,
                        ..Default::default()
                    },
                ),
                ("Studio".to_string(), Profile::default()),
            ]),
            ..Default::default()
        };
        assert_eq!(profile_for_shortcut(&settings, 3).as_deref(), Some("Live"));
        assert_eq!(profile_for_shortcut(&settings, 1), None);
    }

    #[test]
    fn shortcut_lexicon_lists_dynamic_profile_assignments() {
        let settings = config::Settings {
            profiles: BTreeMap::from([(
                "Live".to_string(),
                Profile {
                    shortcut: 3,
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let lines = shortcut_lexicon_lines(&settings);
        assert!(lines.iter().any(|l| l.contains("F9 : mute/unmute micros")));
        assert!(lines.iter().any(|l| l.contains("F10 : mute/unmute global")));
        assert!(lines
            .iter()
            .any(|l| l.contains("Ctrl+Maj+3 : profil « Live »")));
        assert!(lines.iter().any(|l| l.contains("Ctrl+Maj+1 : non assigné")));
        assert!(lines.iter().any(|l| l.contains("double-clic = 0 dB")));
    }

    #[test]
    fn mic_mute_only_touches_hardware_inputs() {
        let mut strips = vec![
            ui::strip::Strip::new("input:mic".into(), "Mic".into()),
            ui::strip::Strip::new("input:line".into(), "Line".into()),
            ui::strip::Strip::new("session:42".into(), "App".into()),
        ];
        toggle_mic_mutes(&mut strips);
        assert!(strips[0].muted && strips[1].muted);
        assert!(!strips[2].muted);
        toggle_mic_mutes(&mut strips);
        assert!(strips.iter().all(|strip| !strip.muted));
    }

    fn endpoint(id: &str) -> AudioEndpoint {
        AudioEndpoint {
            id: id.to_string(),
            name: id.to_string(),
            kind: audio::devices::EndpointKind::Hardware,
        }
    }

    fn ids(strips: &[ui::strip::Strip]) -> Vec<&str> {
        strips.iter().map(|strip| strip.id.as_str()).collect()
    }

    fn out(id: &str, gain: f32) -> audio::engine::OutputSpec {
        audio::engine::OutputSpec {
            id: id.into(),
            gain,
        }
    }

    fn route(input: &str, outputs: Vec<audio::engine::OutputSpec>) -> audio::engine::RouteSpec {
        route_with_gain(input, 1.0, outputs)
    }

    fn route_with_gain(
        input: &str,
        input_gain: f32,
        outputs: Vec<audio::engine::OutputSpec>,
    ) -> audio::engine::RouteSpec {
        route_with_params(input, input_gain, 0, 0, outputs)
    }

    fn route_with_params(
        input: &str,
        input_gain: f32,
        delay_ms: u32,
        pan: i8,
        outputs: Vec<audio::engine::OutputSpec>,
    ) -> audio::engine::RouteSpec {
        route_with_nr(input, input_gain, delay_ms, pan, false, outputs)
    }

    fn route_with_nr(
        input: &str,
        input_gain: f32,
        delay_ms: u32,
        pan: i8,
        noise_suppression: bool,
        outputs: Vec<audio::engine::OutputSpec>,
    ) -> audio::engine::RouteSpec {
        audio::engine::RouteSpec {
            input: input.into(),
            input_gain,
            delay_ms,
            pan,
            noise_suppression,
            outputs,
        }
    }

    #[test]
    fn starts_in_bypass_when_launched_manually() {
        let settings = config::Settings {
            bypass: false,
            ..Default::default()
        };
        assert!(AudiokApp::with_settings(settings, false).master.bypass);
        assert!(!AudiokApp::with_settings(config::Settings::default(), false).hide_after_start);
    }

    #[test]
    fn starts_active_when_launched_with_windows() {
        let settings = config::Settings {
            bypass: true,
            ..Default::default()
        };
        assert!(!AudiokApp::with_settings(settings, true).master.bypass);
        assert!(AudiokApp::with_settings(config::Settings::default(), true).hide_after_start);
    }

    #[test]
    fn vac_hardware_outputs_are_hidden_from_selectors() {
        let outputs = vec![
            AudioEndpoint {
                id: "output:vac".into(),
                name: "Line 1 (Virtual Audio Cable)".into(),
                kind: audio::devices::EndpointKind::Hardware,
            },
            AudioEndpoint {
                id: "output:spk".into(),
                name: "Speakers".into(),
                kind: audio::devices::EndpointKind::Hardware,
            },
            AudioEndpoint {
                id: "app:7".into(),
                name: "Virtual Audio Cable".into(),
                kind: audio::devices::EndpointKind::Software,
            },
        ];
        let visible = visible_outputs(outputs);
        assert_eq!(
            visible.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["output:spk", "app:7"]
        );
    }

    #[test]
    fn vac_hardware_inputs_are_hidden_from_strips() {
        let inputs = vec![
            AudioEndpoint {
                id: "input:vac".into(),
                name: "Line 1 (Virtual Audio Cable)".into(),
                kind: audio::devices::EndpointKind::Hardware,
            },
            AudioEndpoint {
                id: "input:mic".into(),
                name: "Microphone".into(),
                kind: audio::devices::EndpointKind::Hardware,
            },
        ];
        let visible = visible_inputs(inputs);
        assert_eq!(
            visible.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["input:mic"]
        );
    }

    #[test]
    fn strips_are_sorted_by_kind_name_and_call() {
        let sources = vec![
            AudioEndpoint {
                id: "session:16944".into(),
                name: "Discord".into(),
                kind: audio::devices::EndpointKind::Software,
            },
            AudioEndpoint {
                id: "input:mic".into(),
                name: "Microphone".into(),
                kind: audio::devices::EndpointKind::Hardware,
            },
            AudioEndpoint {
                id: "session:12596".into(),
                name: "Discord (appel)".into(),
                kind: audio::devices::EndpointKind::Software,
            },
            AudioEndpoint {
                id: "session:100".into(),
                name: "Firefox".into(),
                kind: audio::devices::EndpointKind::Software,
            },
        ];
        let (strips, _) = sync_strips(
            &sources,
            vec![],
            HashMap::new(),
            false,
            Instant::now(),
            STRIP_GRACE,
        );
        assert_eq!(
            ids(&strips),
            vec!["input:mic", "session:12596", "session:16944", "session:100"]
        );
    }

    #[test]
    fn grace_strips_sink_right_then_return_to_sorted_position() {
        let now = Instant::now();
        let sources = vec![
            AudioEndpoint {
                id: "session:200".into(),
                name: "Discord (appel)".into(),
                kind: audio::devices::EndpointKind::Software,
            },
            AudioEndpoint {
                id: "input:mic".into(),
                name: "Microphone".into(),
                kind: audio::devices::EndpointKind::Hardware,
            },
            AudioEndpoint {
                id: "session:100".into(),
                name: "Firefox".into(),
                kind: audio::devices::EndpointKind::Software,
            },
        ];
        let (strips, cache) =
            sync_strips(&sources, vec![], HashMap::new(), false, now, STRIP_GRACE);
        assert_eq!(
            ids(&strips),
            vec!["input:mic", "session:200", "session:100"]
        );

        let without_discord = vec![
            AudioEndpoint {
                id: "input:mic".into(),
                name: "Microphone".into(),
                kind: audio::devices::EndpointKind::Hardware,
            },
            AudioEndpoint {
                id: "session:100".into(),
                name: "Firefox".into(),
                kind: audio::devices::EndpointKind::Software,
            },
        ];
        let (strips, cache) = sync_strips(
            &without_discord,
            strips,
            cache,
            false,
            now + Duration::from_secs(1),
            STRIP_GRACE,
        );
        assert_eq!(
            ids(&strips),
            vec!["input:mic", "session:100", "session:200"]
        );
        assert!(!strips[2].present);

        let (strips, _) = sync_strips(
            &sources,
            strips,
            cache,
            false,
            now + Duration::from_secs(2),
            STRIP_GRACE,
        );
        assert_eq!(
            ids(&strips),
            vec!["input:mic", "session:200", "session:100"]
        );
        assert!(strips[1].present);
    }

    #[test]
    fn strip_auto_name_tracks_source_label_until_manual_rename() {
        let now = Instant::now();
        let plain = vec![AudioEndpoint {
            id: "session:100".into(),
            name: "Discord".into(),
            kind: audio::devices::EndpointKind::Software,
        }];
        let call = vec![AudioEndpoint {
            id: "session:100".into(),
            name: "Discord (appel)".into(),
            kind: audio::devices::EndpointKind::Software,
        }];

        let (strips, cache) = sync_strips(&plain, vec![], HashMap::new(), false, now, STRIP_GRACE);
        assert_eq!(strips[0].name, "Discord");

        let (strips, cache) = sync_strips(
            &call,
            strips,
            cache,
            false,
            now + Duration::from_secs(1),
            STRIP_GRACE,
        );
        assert_eq!(strips[0].name, "Discord (appel)");

        let mut strips = strips;
        strips[0].name = "Mon Discord".into();
        let (strips, _) = sync_strips(
            &plain,
            strips,
            cache,
            false,
            now + Duration::from_secs(2),
            STRIP_GRACE,
        );
        assert_eq!(strips[0].name, "Mon Discord");
    }

    #[test]
    fn refresh_preserves_state_trims_and_caches() {
        let now = Instant::now();
        let sources = vec![endpoint("input:x")];

        let mut strip = ui::strip::Strip::new("input:x".into(), "X".into());
        strip.volume = 0.5;
        strip.trim = 1.2;
        strip.muted = true;
        strip.selected_outputs = vec![
            Some("output:a".into()),
            None,
            Some("output:b".into()),
            None,
            None,
            None,
        ];

        let (strips, cache) = sync_strips(
            &sources,
            vec![strip],
            HashMap::new(),
            true,
            now,
            STRIP_GRACE,
        );
        assert_eq!(strips.len(), 1);
        assert_eq!(strips[0].volume, 0.5);
        assert_eq!(strips[0].trim, 1.2);
        assert!(strips[0].muted);
        assert_eq!(
            strips[0].selected_outputs,
            vec![Some("output:a".into()), Some("output:b".into())]
        );
        assert!(cache.is_empty());

        let (strips, cache) = sync_strips(&[], strips, cache, true, now, STRIP_GRACE);
        assert!(strips.is_empty());
        assert!(cache.contains_key("input:x"));

        let (strips, cache) = sync_strips(&sources, vec![], cache, true, now, STRIP_GRACE);
        assert_eq!(strips.len(), 1);
        assert_eq!(strips[0].volume, 0.5);
        assert_eq!(strips[0].trim, 1.2);
        assert!(strips[0].muted);
        assert!(cache.is_empty());

        let mut strip = ui::strip::Strip::new("input:y".into(), "Y".into());
        strip.selected_outputs = vec![None, None, None];
        let sources = vec![endpoint("input:y")];
        let (strips, _) = sync_strips(
            &sources,
            vec![strip],
            HashMap::new(),
            false,
            now,
            STRIP_GRACE,
        );
        assert_eq!(strips[0].selected_outputs.len(), 3);

        let mut strip = ui::strip::Strip::new("input:z".into(), "Z".into());
        strip.selected_outputs = vec![Some("output:gone".into()), None, None, None];
        let sources = vec![endpoint("input:z")];
        let (strips, _) = sync_strips(
            &sources,
            vec![strip],
            HashMap::new(),
            true,
            now,
            STRIP_GRACE,
        );
        assert_eq!(strips[0].selected_outputs, vec![Some("output:gone".into())]);
    }

    #[test]
    fn disappeared_strips_are_retained_during_grace_period() {
        let now = Instant::now();
        let sources = vec![endpoint("session:1")];
        let strip = ui::strip::Strip::new("session:1".into(), "App".into());

        let (strips, cache) = sync_strips(
            &sources,
            vec![strip],
            HashMap::new(),
            false,
            now,
            STRIP_GRACE,
        );
        assert_eq!(strips.len(), 1);
        assert!(strips[0].present);

        let (strips, cache) = sync_strips(
            &[],
            strips,
            cache,
            false,
            now + Duration::from_secs(2),
            STRIP_GRACE,
        );
        assert_eq!(strips.len(), 1);
        assert!(!strips[0].present);

        let (strips, cache) = sync_strips(
            &[],
            strips,
            cache,
            false,
            now + Duration::from_secs(8),
            STRIP_GRACE,
        );
        assert!(strips.is_empty());
        assert!(cache.contains_key("session:1"));
    }

    #[test]
    fn cache_is_bounded_and_evicts_oldest_strips() {
        let now = Instant::now();
        let total = CACHE_CAP + 2;
        let mut cache = HashMap::new();
        for i in 0..total {
            let mut strip = ui::strip::Strip::new(format!("session:{i}"), format!("App {i}"));
            strip.last_seen = Some(now - Duration::from_secs(i as u64));
            cache.insert(strip.id.clone(), strip);
        }

        let (_, cache) = sync_strips(&[], vec![], cache, false, now, STRIP_GRACE);

        assert_eq!(cache.len(), CACHE_CAP);
        assert!(!cache.contains_key(&format!("session:{}", total - 1)));
        assert!(!cache.contains_key(&format!("session:{}", total - 2)));
        assert!(cache.contains_key("session:0"));
        assert!(cache.contains_key("session:1"));
    }

    #[test]
    fn desired_routes_includes_present_inputs_and_sessions() {
        let mut session = ui::strip::Strip::new("session:1".into(), "App".into());
        session.selected_outputs = vec![Some("output:a".into())];

        let mut idle = ui::strip::Strip::new("session:2".into(), "Idle".into());
        idle.selected_outputs = vec![None];

        let mut system_sounds = ui::strip::Strip::new("session:0".into(), "Sons du système".into());
        system_sounds.selected_outputs = vec![Some("output:a".into())];

        let mut hidden = ui::strip::Strip::new("input:hidden".into(), "Hidden".into());
        hidden.selected_outputs = vec![Some("output:a".into())];
        hidden.present = false;

        let mut empty = ui::strip::Strip::new("input:empty".into(), "Empty".into());
        empty.selected_outputs = vec![None];

        let mut chosen = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        chosen.selected_outputs = vec![
            None,
            Some("output:b".into()),
            Some("app:7".into()),
            Some("app:8".into()),
            Some("output:a".into()),
            Some("output:b".into()),
        ];

        let mut other = ui::strip::Strip::new("input:line".into(), "Line".into());
        other.selected_outputs = vec![Some("output:a".into())];

        let strips = vec![session, idle, system_sounds, hidden, empty, chosen, other];
        let master = ui::master::Master::default();
        assert_eq!(
            desired_routes(&strips, &HashMap::new(), &master),
            vec![
                route("session:1", vec![out("output:a", 1.0)]),
                route("session:2", Vec::new()),
                route("input:empty", Vec::new()),
                route(
                    "input:mic",
                    vec![out("output:b", 1.0), out("output:a", 1.0)],
                ),
                route("input:line", vec![out("output:a", 1.0)]),
            ]
        );

        let bypassed = ui::master::Master {
            bypass: true,
            ..Default::default()
        };
        assert_eq!(
            desired_routes(&strips, &HashMap::new(), &bypassed),
            vec![
                route("session:1", Vec::new()),
                route("session:2", Vec::new()),
                route("input:empty", Vec::new()),
                route("input:mic", Vec::new()),
                route("input:line", Vec::new()),
            ]
        );

        let app_routes = HashMap::from([("app:7".to_string(), "output:vac1".to_string())]);
        assert_eq!(
            desired_routes(&strips, &app_routes, &master),
            vec![
                route("session:1", vec![out("output:a", 1.0)]),
                route("session:2", Vec::new()),
                route("input:empty", Vec::new()),
                route(
                    "input:mic",
                    vec![
                        out("output:b", 1.0),
                        out("output:vac1", 1.0),
                        out("output:a", 1.0),
                    ],
                ),
                route("input:line", vec![out("output:a", 1.0)]),
            ]
        );
    }

    #[test]
    fn desired_routes_ignores_app_routing_to_itself() {
        let mut strip = ui::strip::Strip::new("session:7".into(), "Self".into());
        strip.selected_outputs = vec![Some("app:7".into()), Some("app:8".into())];
        let app_routes = HashMap::from([
            ("app:7".to_string(), "output:vac7".to_string()),
            ("app:8".to_string(), "output:vac8".to_string()),
        ]);
        assert_eq!(
            desired_routes(&[strip], &app_routes, &ui::master::Master::default()),
            vec![route("session:7", vec![out("output:vac8", 1.0)])]
        );
    }

    #[test]
    fn mix_choices_lists_present_strip_outputs_once_without_cue() {
        let mut first = ui::strip::Strip::new("input:a".into(), "A".into());
        first.selected_outputs = vec![Some("output:x".into()), None, Some("output:y".into())];

        let mut second = ui::strip::Strip::new("input:b".into(), "B".into());
        second.selected_outputs = vec![Some("output:x".into()), Some("output:cue".into())];

        let mut hidden = ui::strip::Strip::new("input:hidden".into(), "Hidden".into());
        hidden.selected_outputs = vec![Some("output:z".into())];
        hidden.present = false;

        let system = ui::strip::Strip::new("session:0".into(), "Sons du système".into());

        let outputs = vec![
            endpoint("output:x"),
            endpoint("output:y"),
            endpoint("output:cue"),
            endpoint("output:z"),
        ];
        assert_eq!(
            mix_choices(
                &[first, second, hidden, system],
                Some("output:cue"),
                &outputs
            ),
            vec!["output:x".to_string(), "output:y".to_string()]
        );
    }

    #[test]
    fn mix_choices_skips_missing_outputs() {
        let mut strip = ui::strip::Strip::new("input:a".into(), "A".into());
        strip.selected_outputs = vec![Some("output:a".into()), Some("app:42".into())];
        let outputs = vec![endpoint("output:a")];
        assert_eq!(
            mix_choices(&[strip], None, &outputs),
            vec!["output:a".to_string()]
        );
    }

    #[test]
    fn strip_app_pids_keeps_present_process_strips() {
        let app = ui::strip::Strip::new("session:10".into(), "App".into());
        let mut grace = ui::strip::Strip::new("session:20".into(), "Grace".into());
        grace.present = false;
        let input = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        let system = ui::strip::Strip::new("session:0".into(), "Sons du système".into());
        assert_eq!(
            strip_app_pids(&[app, grace, input, system]),
            HashSet::from([10])
        );
    }

    #[test]
    fn desired_routes_applies_volume_mute_and_solo() {
        let mut first = ui::strip::Strip::new("input:a".into(), "A".into());
        first.selected_outputs = vec![Some("output:a".into())];
        first.volume = 0.5;

        let mut second = ui::strip::Strip::new("input:b".into(), "B".into());
        second.selected_outputs = vec![Some("output:a".into())];
        second.volume = 0.25;

        let mut third = ui::strip::Strip::new("input:c".into(), "C".into());
        third.selected_outputs = vec![Some("output:a".into())];

        let master = ui::master::Master::default();
        let gains = |strips: &[ui::strip::Strip]| {
            desired_routes(strips, &HashMap::new(), &master)
                .iter()
                .map(|route| route.outputs[0].gain)
                .collect::<Vec<_>>()
        };

        assert_eq!(gains(&[first.clone(), second.clone()]), vec![0.5, 0.25]);

        let mut muted = second.clone();
        muted.muted = true;
        assert_eq!(gains(&[first.clone(), muted]), vec![0.5, 0.0]);

        let mut solo = first.clone();
        solo.solo = true;
        assert_eq!(
            gains(&[solo.clone(), second.clone(), third.clone()]),
            vec![0.5, 0.0, 0.0]
        );

        let mut second_solo = second.clone();
        second_solo.solo = true;
        assert_eq!(gains(&[solo, second_solo, third]), vec![0.5, 0.25, 0.0]);
    }

    #[test]
    fn desired_routes_keeps_trim_as_source_gain() {
        let mut loud = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        loud.selected_outputs = vec![Some("output:a".into())];
        loud.trim = 2.0;
        loud.pfl = true;

        let mut quiet = ui::strip::Strip::new("input:line".into(), "Line".into());
        quiet.selected_outputs = vec![Some("output:a".into())];
        quiet.volume = 0.5;
        quiet.trim = 0.5;

        let strips = vec![loud, quiet];
        let empty = HashMap::new();

        assert_eq!(
            desired_routes(&strips, &empty, &ui::master::Master::default()),
            vec![
                route_with_gain("input:mic", 2.0, vec![out("output:a", 1.0)]),
                route_with_gain("input:line", 0.5, vec![out("output:a", 0.5)]),
            ]
        );

        let pre = ui::master::Master {
            cue_device: Some("output:cue".into()),
            cue_volume: 0.4,
            ..Default::default()
        };
        assert_eq!(
            desired_routes(&strips, &empty, &pre),
            vec![
                route_with_gain(
                    "input:mic",
                    2.0,
                    vec![out("output:a", 1.0), out("output:cue", 0.4)],
                ),
                route_with_gain(
                    "input:line",
                    0.5,
                    vec![out("output:a", 0.5), out("output:cue", 0.0)],
                ),
            ]
        );

        let mix = ui::master::Master {
            cue_mix: Some("output:a".into()),
            cue_mix_enabled: true,
            ..pre
        };
        assert_eq!(
            desired_routes(&strips, &empty, &mix),
            vec![
                route_with_gain(
                    "input:mic",
                    2.0,
                    vec![out("output:a", 1.0), out("output:cue", 0.4)],
                ),
                route_with_gain(
                    "input:line",
                    0.5,
                    vec![out("output:a", 0.5), out("output:cue", 0.2)],
                ),
            ]
        );
    }

    #[test]
    fn solo_cuts_main_mix_but_cue_keeps_pfl_strips() {
        let mut lead = ui::strip::Strip::new("input:lead".into(), "Lead".into());
        lead.selected_outputs = vec![Some("output:a".into())];
        lead.solo = true;

        let mut quiet = ui::strip::Strip::new("input:quiet".into(), "Quiet".into());
        quiet.selected_outputs = vec![Some("output:a".into())];
        quiet.pfl = true;

        let strips = vec![lead, quiet];
        let empty = HashMap::new();

        let pre = ui::master::Master {
            cue_device: Some("output:cue".into()),
            cue_volume: 0.4,
            ..Default::default()
        };
        assert_eq!(
            desired_routes(&strips, &empty, &pre),
            vec![
                route(
                    "input:lead",
                    vec![out("output:a", 1.0), out("output:cue", 0.0)],
                ),
                route(
                    "input:quiet",
                    vec![out("output:a", 0.0), out("output:cue", 0.4)],
                ),
            ]
        );

        let mix = ui::master::Master {
            cue_mix: Some("output:a".into()),
            cue_mix_enabled: true,
            ..pre
        };
        assert_eq!(
            desired_routes(&strips, &empty, &mix),
            vec![
                route(
                    "input:lead",
                    vec![out("output:a", 1.0), out("output:cue", 0.4)],
                ),
                route(
                    "input:quiet",
                    vec![out("output:a", 0.0), out("output:cue", 0.0)],
                ),
            ]
        );
    }

    #[test]
    fn cue_routes_follow_headphone_and_mix_modes() {
        let mut pfl = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        pfl.selected_outputs = vec![Some("output:a".into())];
        pfl.pfl = true;
        pfl.volume = 0.5;

        let mut muted = ui::strip::Strip::new("input:line".into(), "Line".into());
        muted.selected_outputs = vec![Some("output:a".into())];
        muted.pfl = true;
        muted.muted = true;

        let mut plain = ui::strip::Strip::new("input:aux".into(), "Aux".into());
        plain.selected_outputs = vec![Some("output:a".into())];

        let strips = vec![pfl, muted, plain];
        let empty = HashMap::new();

        let master = ui::master::Master::default();
        assert_eq!(
            desired_routes(&strips, &empty, &master),
            vec![
                route("input:mic", vec![out("output:a", 0.5)]),
                route("input:line", vec![out("output:a", 0.0)]),
                route("input:aux", vec![out("output:a", 1.0)]),
            ]
        );

        let pre = ui::master::Master {
            cue_device: Some("output:cue".into()),
            cue_volume: 0.4,
            ..Default::default()
        };
        assert_eq!(
            desired_routes(&strips, &empty, &pre),
            vec![
                route(
                    "input:mic",
                    vec![out("output:a", 0.5), out("output:cue", 0.4)],
                ),
                route(
                    "input:line",
                    vec![out("output:a", 0.0), out("output:cue", 0.4)],
                ),
                route(
                    "input:aux",
                    vec![out("output:a", 1.0), out("output:cue", 0.0)],
                ),
            ]
        );

        let mix = ui::master::Master {
            cue_mix: Some("output:a".into()),
            cue_mix_enabled: true,
            ..pre
        };
        assert_eq!(
            desired_routes(&strips, &empty, &mix),
            vec![
                route(
                    "input:mic",
                    vec![out("output:a", 0.5), out("output:cue", 0.2)],
                ),
                route(
                    "input:line",
                    vec![out("output:a", 0.0), out("output:cue", 0.0)],
                ),
                route(
                    "input:aux",
                    vec![out("output:a", 1.0), out("output:cue", 0.4)],
                ),
            ]
        );

        let mix_without_selection = ui::master::Master {
            cue_device: Some("output:cue".into()),
            cue_volume: 0.4,
            cue_mix_enabled: true,
            ..Default::default()
        };
        assert_eq!(
            desired_routes(&strips, &empty, &mix_without_selection),
            vec![
                route(
                    "input:mic",
                    vec![out("output:a", 0.5), out("output:cue", 0.0)],
                ),
                route(
                    "input:line",
                    vec![out("output:a", 0.0), out("output:cue", 0.0)],
                ),
                route(
                    "input:aux",
                    vec![out("output:a", 1.0), out("output:cue", 0.0)],
                ),
            ]
        );

        let mute_all = ui::master::Master {
            cue_device: Some("output:cue".into()),
            cue_volume: 0.4,
            mute_all: true,
            ..Default::default()
        };
        let mute_all_mix = ui::master::Master {
            cue_device: Some("output:cue".into()),
            cue_volume: 0.4,
            cue_mix: Some("output:a".into()),
            cue_mix_enabled: true,
            mute_all: true,
            ..Default::default()
        };
        for master in [mute_all, mute_all_mix] {
            assert_eq!(
                desired_routes(&strips, &empty, &master),
                vec![
                    route(
                        "input:mic",
                        vec![out("output:a", 0.0), out("output:cue", 0.0)],
                    ),
                    route(
                        "input:line",
                        vec![out("output:a", 0.0), out("output:cue", 0.0)],
                    ),
                    route(
                        "input:aux",
                        vec![out("output:a", 0.0), out("output:cue", 0.0)],
                    ),
                ]
            );
        }

        let mut shared = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        shared.selected_outputs = vec![Some("output:cue".into())];
        shared.pfl = true;
        let shared_master = ui::master::Master {
            cue_device: Some("output:cue".into()),
            ..Default::default()
        };
        assert_eq!(
            desired_routes(&[shared], &empty, &shared_master),
            vec![route("input:mic", vec![out("output:cue", 1.0)])]
        );

        let app_routes = HashMap::from([("app:7".to_string(), "output:vac1".to_string())]);
        let app_master = ui::master::Master {
            cue_device: Some("app:7".into()),
            cue_volume: 0.4,
            ..Default::default()
        };
        let mut pfl = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        pfl.pfl = true;
        assert_eq!(
            desired_routes(&[pfl], &app_routes, &app_master),
            vec![route("input:mic", vec![out("output:vac1", 0.4)])]
        );
    }

    #[test]
    fn desired_routes_applies_per_output_mix_gain() {
        let mut strip = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        strip.selected_outputs = vec![Some("output:a".into()), Some("output:b".into())];
        strip.mix_gains = BTreeMap::from([
            (EndpointKey::Hardware("output:a".into()), 0.25),
            (EndpointKey::Hardware("output:b".into()), 0.75),
        ]);

        assert_eq!(
            desired_routes(&[strip], &HashMap::new(), &ui::master::Master::default()),
            vec![route(
                "input:mic",
                vec![out("output:a", 0.25), out("output:b", 0.75)]
            )]
        );
    }

    #[test]
    fn desired_routes_propagates_delay_and_pan() {
        let mut strip = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        strip.selected_outputs = vec![Some("output:a".into())];
        strip.delay_ms = 250;
        strip.pan = -40;

        assert_eq!(
            desired_routes(&[strip], &HashMap::new(), &ui::master::Master::default()),
            vec![route_with_params(
                "input:mic",
                1.0,
                250,
                -40,
                vec![out("output:a", 1.0)]
            )]
        );
    }

    #[test]
    fn output_gain_falls_back_to_app_exe_regardless_of_role() {
        let pid = std::process::id();
        let id = format!("app:{pid}");
        let captures = HashSet::from([pid]);
        let key = match endpoint_key(&id, &captures) {
            EndpointKey::App { exe, .. } => EndpointKey::App {
                exe,
                role: AppRole::Other,
            },
            other => other,
        };
        let mut strip = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        strip.mix_gains.insert(key, 0.3);

        assert_eq!(output_gain(&strip, &id, &captures), 0.3);
    }

    #[test]
    fn cue_mix_uses_selected_output_gain() {
        let mut strip = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        strip.selected_outputs = vec![Some("output:b".into())];
        strip
            .mix_gains
            .insert(EndpointKey::Hardware("output:b".into()), 0.5);
        let master = ui::master::Master {
            cue_device: Some("output:cue".into()),
            cue_mix_enabled: true,
            cue_mix: Some("output:b".into()),
            cue_volume: 0.8,
            ..Default::default()
        };

        assert_eq!(
            desired_routes(&[strip], &HashMap::new(), &master),
            vec![route(
                "input:mic",
                vec![out("output:b", 0.5), out("output:cue", 0.4)]
            )]
        );
    }

    #[test]
    fn alias_key_is_stable_for_hardware_and_apps() {
        assert_eq!(
            alias_key(&EndpointKey::Hardware("output:a".into())),
            "output:a"
        );
        let key = EndpointKey::App {
            exe: "c:\\app.exe".into(),
            role: AppRole::Call,
        };
        assert_eq!(alias_key(&key), "app:c:\\app.exe|call");
    }

    #[test]
    fn resolved_aliases_maps_stable_keys_to_runtime_ids() {
        let captures = HashSet::new();

        let aliases = HashMap::from([("output:a".to_string(), "HP".to_string())]);
        let labels = resolved_aliases(&aliases, &[endpoint("output:a")], &captures);
        assert_eq!(labels.get("output:a").map(String::as_str), Some("HP"));

        let id = format!("app:{}", std::process::id());
        let key = alias_key(&endpoint_key(&id, &captures));
        let aliases = HashMap::from([(key, "Mon appli".to_string())]);
        let labels = resolved_aliases(&aliases, &[endpoint(&id)], &captures);
        assert_eq!(labels.get(&id).map(String::as_str), Some("Mon appli"));
    }

    #[test]
    fn apply_strip_profile_restores_state_and_resolves_outputs() {
        let mut strip = ui::strip::Strip::new("session:123".into(), "Discord".into());
        let key = EndpointKey::App {
            exe: "c:\\discord.exe".into(),
            role: AppRole::Other,
        };
        let saved = StripProfile {
            source: key.clone(),
            name: Some("Perso".into()),
            outputs: vec![
                EndpointKey::Hardware("output:a".into()),
                EndpointKey::Hardware("output:missing".into()),
            ],
            volume: 0.5,
            trim: 1.2,
            delay_ms: 300,
            pan: 20,
            noise_suppression: true,
            muted: true,
            solo: false,
            pfl: true,
            mix_gains: BTreeMap::from([(EndpointKey::Hardware("output:a".into()), 0.25)]),
        };
        let output_map = HashMap::from([(
            EndpointKey::Hardware("output:a".into()),
            vec!["output:a".to_string()],
        )]);

        let (ambiguous, unresolved) = apply_strip_profile(&mut strip, &saved, &output_map);

        assert!(ambiguous.is_empty());
        assert_eq!(
            unresolved,
            vec![EndpointKey::Hardware("output:missing".into())]
        );
        assert_eq!(strip.name, "Perso");
        assert_eq!(strip.volume, 0.5);
        assert_eq!(
            strip
                .mix_gains
                .get(&EndpointKey::Hardware("output:a".into())),
            Some(&0.25)
        );
        assert!(strip.muted);
        assert!(strip.pfl);
        assert_eq!(strip.selected_outputs, vec![Some("output:a".to_string())]);
        assert_eq!(strip.trim, 1.2);
        assert_eq!(strip.delay_ms, 300);
        assert_eq!(strip.pan, 20);
        assert!(strip.noise_suppression);
    }

    #[test]
    fn resolve_outputs_reports_ambiguous_app_output() {
        let app = EndpointKey::App {
            exe: "c:\\discord.exe".into(),
            role: AppRole::Other,
        };
        let saved = vec![
            EndpointKey::Hardware("output:a".into()),
            app.clone(),
            EndpointKey::Hardware("output:missing".into()),
        ];
        let output_map = HashMap::from([
            (
                EndpointKey::Hardware("output:a".into()),
                vec!["output:a".to_string()],
            ),
            (
                app.clone(),
                vec!["app:10".to_string(), "app:11".to_string()],
            ),
        ]);

        let (outputs, ambiguous, unresolved) = resolve_outputs(&saved, &output_map);

        assert_eq!(outputs, vec![Some("output:a".to_string()), None]);
        assert_eq!(ambiguous.len(), 1);
        assert_eq!(ambiguous[0].index, 1);
        assert_eq!(ambiguous[0].key, app);
        assert_eq!(ambiguous[0].ids, vec!["app:10", "app:11"]);
        assert_eq!(
            unresolved,
            vec![EndpointKey::Hardware("output:missing".into())]
        );
    }

    #[test]
    fn push_output_choices_targets_strip_slot_with_pid_labels() {
        let app = EndpointKey::App {
            exe: "c:\\discord.exe".into(),
            role: AppRole::Other,
        };
        let endpoints = vec![
            AudioEndpoint {
                id: "app:10".into(),
                name: "Discord".into(),
                kind: audio::devices::EndpointKind::Software,
            },
            AudioEndpoint {
                id: "app:11".into(),
                name: "Discord (appel)".into(),
                kind: audio::devices::EndpointKind::Software,
            },
        ];
        let mut choices = Vec::new();

        push_output_choices(
            &mut choices,
            "session:10",
            vec![AmbiguousOutput {
                index: 1,
                key: app,
                ids: vec!["app:10".into(), "app:11".into()],
            }],
            &endpoints,
        );

        assert_eq!(choices.len(), 1);
        assert!(matches!(
            choices[0].target,
            ProfileTarget::Output { ref strip_id, index } if strip_id == "session:10" && index == 1
        ));
        assert_eq!(choices[0].options[0].label, "Discord — PID 10");
        assert_eq!(choices[0].options[1].label, "Discord (appel) — PID 11");
    }

    #[test]
    fn reset_strip_restores_defaults_and_auto_name() {
        let mut strip = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        strip.name = "Perso".into();
        strip.selected_outputs = vec![Some("output:a".into())];
        strip.volume = 0.25;
        strip.trim = 1.5;
        strip.delay_ms = 500;
        strip.pan = -60;
        strip.noise_suppression = true;
        strip.muted = true;
        strip.pfl = true;

        reset_strip(&mut strip);

        assert_eq!(strip.name, "Mic");
        assert_eq!(strip.selected_outputs, vec![None]);
        assert_eq!(strip.volume, 1.0);
        assert_eq!(strip.trim, 1.0);
        assert_eq!(strip.delay_ms, 0);
        assert_eq!(strip.pan, 0);
        assert!(!strip.noise_suppression);
        assert!(!strip.muted);
        assert!(!strip.pfl);
    }

    #[test]
    fn content_width_includes_layout_spacing() {
        let six_strips = strips_content_width(6, 8.0);
        assert_eq!(
            six_strips,
            6.0 * (ui::COLUMN_WIDTH + 8.0 + STRIP_SPACING) + ui::MASTER_PANEL_WIDTH
        );
        assert_eq!(strips_content_width(0, 8.0), ui::MASTER_PANEL_WIDTH);
    }

    #[test]
    fn snapshot_drops_fallback_endpoint_keys() {
        let mut mic = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        mic.selected_outputs = vec![Some("app:999998".into())];
        let ghost = ui::strip::Strip::new("session:999999".into(), "Ghost".into());
        let app = AudiokApp {
            strips: vec![mic, ghost],
            ..Default::default()
        };

        let profile = app.snapshot_profile(&Profile::default());
        assert_eq!(profile.strips.len(), 1);
        assert!(profile.strips[0].outputs.is_empty());
    }

    #[test]
    fn retry_pending_outputs_fills_slot_when_output_appears() {
        let mut app = AudiokApp {
            strips: vec![ui::strip::Strip::new("input:mic".into(), "Mic".into())],
            ..Default::default()
        };
        app.pending_outputs.insert(
            EndpointKey::Hardware("input:mic".into()),
            vec![EndpointKey::Hardware("output:b".into())],
        );

        app.retry_pending_outputs(&HashSet::new());
        assert!(!app.pending_outputs.is_empty());

        app.outputs = vec![endpoint("output:b")];
        app.retry_pending_outputs(&HashSet::new());
        assert_eq!(app.strips[0].selected_outputs[0], Some("output:b".into()));
        assert!(app.pending_outputs.is_empty());
    }

    #[test]
    fn refresh_reassigns_returning_output_without_manual_delete() {
        let now = Instant::now();
        let sources = vec![endpoint("input:mic")];
        let mut strip = ui::strip::Strip::new("input:mic".into(), "Mic".into());
        strip.selected_outputs = vec![Some("output:a".into())];

        let (strips, _) = sync_strips(
            &sources,
            vec![strip],
            HashMap::new(),
            true,
            now,
            STRIP_GRACE,
        );
        assert_eq!(strips[0].selected_outputs, vec![Some("output:a".into())]);

        let (strips, _) = sync_strips(&sources, strips, HashMap::new(), true, now, STRIP_GRACE);
        assert_eq!(strips[0].selected_outputs, vec![Some("output:a".into())]);
    }

    #[test]
    fn returning_strip_restores_saved_state() {
        let mut app = AudiokApp {
            strips: vec![ui::strip::Strip::new("input:mic".into(), "Mic".into())],
            outputs: vec![endpoint("output:a")],
            ..Default::default()
        };
        app.settings.active_profile = Some("Défaut".into());
        app.settings.profiles.insert(
            "Défaut".into(),
            Profile {
                strips: vec![StripProfile {
                    source: EndpointKey::Hardware("input:mic".into()),
                    name: Some("Perso".into()),
                    outputs: vec![EndpointKey::Hardware("output:a".into())],
                    volume: 0.5,
                    trim: 1.2,
                    delay_ms: 150,
                    pan: -40,
                    noise_suppression: true,
                    muted: true,
                    solo: false,
                    pfl: true,
                    mix_gains: BTreeMap::from([(EndpointKey::Hardware("output:a".into()), 0.4)]),
                }],
                ..Default::default()
            },
        );

        app.restore_returning_strips(&["input:mic".to_string()], &HashSet::new());

        let strip = &app.strips[0];
        assert_eq!(strip.name, "Perso");
        assert_eq!(strip.volume, 0.5);
        assert_eq!(
            strip
                .mix_gains
                .get(&EndpointKey::Hardware("output:a".into())),
            Some(&0.4)
        );
        assert_eq!(strip.trim, 1.2);
        assert_eq!(strip.delay_ms, 150);
        assert_eq!(strip.pan, -40);
        assert!(strip.noise_suppression);
        assert!(strip.muted);
        assert!(strip.pfl);
        assert_eq!(strip.selected_outputs, vec![Some("output:a".into())]);
    }

    #[test]
    fn returning_strip_skips_sources_waiting_for_profile_choice() {
        let mut app = AudiokApp {
            strips: vec![ui::strip::Strip::new("input:mic".into(), "Mic".into())],
            ..Default::default()
        };
        app.settings.active_profile = Some("Défaut".into());
        app.settings.profiles.insert(
            "Défaut".into(),
            Profile {
                strips: vec![StripProfile {
                    source: EndpointKey::Hardware("input:mic".into()),
                    volume: 0.25,
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        app.pending_profile.insert(
            EndpointKey::Hardware("input:mic".into()),
            StripProfile::default(),
        );

        app.restore_returning_strips(&["input:mic".to_string()], &HashSet::new());

        assert_eq!(app.strips[0].volume, 1.0);
    }

    #[test]
    fn app_output_resolves_across_role_change() {
        let saved = EndpointKey::App {
            exe: "c:\\discord.exe".into(),
            role: AppRole::Call,
        };
        let runtime = EndpointKey::App {
            exe: "c:\\discord.exe".into(),
            role: AppRole::Other,
        };
        let output_map = HashMap::from([(runtime, vec!["app:42".to_string()])]);

        let (outputs, ambiguous, unresolved) = resolve_outputs(&[saved], &output_map);

        assert!(ambiguous.is_empty());
        assert!(unresolved.is_empty());
        assert_eq!(outputs[0], Some("app:42".into()));
    }

    #[test]
    fn refresh_keeps_missing_app_output() {
        let now = Instant::now();
        let sources = vec![endpoint("input:x")];
        let mut strip = ui::strip::Strip::new("input:x".into(), "X".into());
        strip.selected_outputs = vec![Some("app:42".into()), Some("output:gone".into()), None];

        let (strips, _) = sync_strips(
            &sources,
            vec![strip],
            HashMap::new(),
            true,
            now,
            STRIP_GRACE,
        );
        assert_eq!(
            strips[0].selected_outputs,
            vec![Some("app:42".into()), Some("output:gone".into())]
        );
    }

    #[test]
    fn autosave_due_only_after_delay_without_pending_choices() {
        let now = Instant::now();
        assert!(autosave_due(Some(now - AUTOSAVE_DELAY), now, false));
        assert!(!autosave_due(
            Some(now - Duration::from_millis(500)),
            now,
            false
        ));
        assert!(!autosave_due(Some(now - AUTOSAVE_DELAY), now, true));
        assert!(!autosave_due(None, now, false));
    }

    #[test]
    fn fingerprint_follows_persistent_state() {
        let mut app = AudiokApp::default();
        app.strips
            .push(ui::strip::Strip::new("input:mic".into(), "Mic".into()));
        let initial = app.persistent_fingerprint();
        assert_eq!(initial, app.persistent_fingerprint());

        app.strips[0].volume = 0.5;
        assert_ne!(initial, app.persistent_fingerprint());

        let after_volume = app.persistent_fingerprint();
        app.strips[0].trim = 1.2;
        assert_ne!(after_volume, app.persistent_fingerprint());

        let after_trim = app.persistent_fingerprint();
        app.strips[0]
            .mix_gains
            .insert(EndpointKey::Hardware("output:a".into()), 0.7);
        assert_ne!(after_trim, app.persistent_fingerprint());

        let after_mix = app.persistent_fingerprint();
        app.strips[0].delay_ms = 300;
        assert_ne!(after_mix, app.persistent_fingerprint());

        let after_delay = app.persistent_fingerprint();
        app.strips[0].pan = 25;
        assert_ne!(after_delay, app.persistent_fingerprint());

        let after_pan = app.persistent_fingerprint();
        app.appearance.angle = 135;
        assert_ne!(after_pan, app.persistent_fingerprint());
    }

    #[test]
    fn redirect_retries_only_after_delay() {
        let now = Instant::now();
        assert!(should_redirect(false, None, now));
        assert!(!should_redirect(true, None, now));
        assert!(!should_redirect(true, Some(now + REDIRECT_RETRY), now));
        assert!(should_redirect(true, Some(now - REDIRECT_RETRY), now));
    }

    #[test]
    fn restore_waits_for_retry_delay() {
        let now = Instant::now();
        assert!(should_restore(None, now));
        assert!(!should_restore(Some(now + REDIRECT_RETRY), now));
        assert!(should_restore(Some(now - REDIRECT_RETRY), now));
    }

    #[test]
    fn input_policy_apps_keeps_present_grace_and_destination_only() {
        let now = Instant::now();
        let mut gone = ui::strip::Strip::new("session:42".into(), "App".into());
        gone.present = false;
        gone.last_seen = Some(now - Duration::from_secs(1));
        let present = ui::strip::Strip::new("session:7".into(), "Other".into());
        let apps = ["app:42", "app:7", "app:99"];
        let redirected = HashMap::from([(42, [None, None, None])]);
        let kept = input_policy_apps(
            &apps,
            &[gone, present],
            &redirected,
            now,
            STRIP_GRACE,
            |_| true,
        );
        assert_eq!(kept, vec!["app:42", "app:7", "app:99"]);
    }

    #[test]
    fn input_policy_apps_drops_dead_expired_and_unredirected() {
        let now = Instant::now();
        let mut expired = ui::strip::Strip::new("session:42".into(), "Expired".into());
        expired.present = false;
        expired.last_seen = Some(now - STRIP_GRACE - Duration::from_secs(1));
        let mut unredirected = ui::strip::Strip::new("session:43".into(), "Unredirected".into());
        unredirected.present = false;
        unredirected.last_seen = Some(now - Duration::from_secs(1));
        let dead = ui::strip::Strip::new("session:44".into(), "Dead".into());
        let apps = ["app:42", "app:43", "app:44"];
        let redirected = HashMap::from([(42, [None, None, None])]);
        let kept = input_policy_apps(
            &apps,
            &[expired, unredirected, dead],
            &redirected,
            now,
            STRIP_GRACE,
            |pid| pid == 42 || pid == 43,
        );
        assert!(kept.is_empty());
    }

    #[test]
    fn protected_redirect_pids_keeps_grace_alive_redirected_app() {
        let now = Instant::now();
        let mut gone = ui::strip::Strip::new("session:42".into(), "Telegram".into());
        gone.present = false;
        gone.last_seen = Some(now - Duration::from_secs(1));
        let redirected = HashMap::from([(42, [None, None, None])]);
        let protected = protected_redirect_pids(&[gone], &redirected, now, STRIP_GRACE, |_| true);
        assert!(protected.contains(&42));
    }

    #[test]
    fn protected_redirect_pids_ignores_grace_app_not_redirected() {
        let now = Instant::now();
        let mut gone = ui::strip::Strip::new("session:42".into(), "Telegram".into());
        gone.present = false;
        gone.last_seen = Some(now - Duration::from_secs(1));
        let protected =
            protected_redirect_pids(&[gone], &HashMap::new(), now, STRIP_GRACE, |_| true);
        assert!(!protected.contains(&42));
    }

    #[test]
    fn protected_redirect_pids_drops_expired_or_dead_grace_app() {
        let now = Instant::now();
        let mut expired = ui::strip::Strip::new("session:42".into(), "Telegram".into());
        expired.present = false;
        expired.last_seen = Some(now - STRIP_GRACE - Duration::from_secs(1));
        let mut dead = ui::strip::Strip::new("session:43".into(), "Discord".into());
        dead.present = false;
        dead.last_seen = Some(now - Duration::from_secs(1));
        let redirected = HashMap::from([(42, [None, None, None]), (43, [None, None, None])]);
        let protected =
            protected_redirect_pids(&[expired, dead], &redirected, now, STRIP_GRACE, |_| false);
        assert!(!protected.contains(&42));
        assert!(!protected.contains(&43));
    }

    #[test]
    fn protected_redirect_pids_includes_present_apps() {
        let now = Instant::now();
        let present = ui::strip::Strip::new("session:7".into(), "App".into());
        let protected =
            protected_redirect_pids(&[present], &HashMap::new(), now, STRIP_GRACE, |_| false);
        assert!(protected.contains(&7));
    }

    #[test]
    fn mismatched_roles_lists_non_matching_endpoints() {
        let sink = "{sink}";
        let matching = [
            Some(sink.to_string()),
            Some(sink.to_string()),
            Some(sink.to_string()),
        ];
        assert!(mismatched_roles(&matching, sink).is_empty());

        let partial = [Some(sink.to_string()), None, Some("{other}".to_string())];
        assert_eq!(
            mismatched_roles(&partial, sink),
            vec!["rôle 1: -".to_string(), "rôle 2: {other}".to_string()]
        );
    }
}
