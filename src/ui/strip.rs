use std::collections::{BTreeMap, HashMap};

use crate::config::profile::EndpointKey;
use crate::i18n::tr;
use std::time::Instant;

use eframe::egui;

const MAX_SELECTORS: usize = 8;
const SELECTOR_ROW_HEIGHT: f32 = 24.0;
const SELECTOR_ROW_SPACING: f32 = 4.0;
const SELECTOR_ROW_PITCH: f32 = SELECTOR_ROW_HEIGHT + SELECTOR_ROW_SPACING;
const KNOB_ROW_HEIGHT: f32 = 64.0;
const KNOB_WIDTH: f32 = 40.0;
const KNOB_GAP: f32 = 4.0;

#[derive(Clone)]
pub enum GainEdit {
    Volume,
    Mix(EndpointKey),
    Disabled,
}

fn gain_ui(ui: &mut egui::Ui, id: &str, gain: &mut f32, level: f32, width: f32, enabled: bool) {
    super::volume_value(ui, ("volume", id), gain, width, 18.0, enabled);
    ui.add_space(4.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, 180.0),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            if !enabled {
                ui.disable();
            }
            ui.spacing_mut().slider_width = 180.0;
            ui.add_space(width / 3.0 - 10.0);
            let mut position = super::gain_position(*gain);
            let fader = ui
                .add_sized(
                    [20.0, 180.0],
                    egui::Slider::new(&mut position, 0.0..=1.0)
                        .vertical()
                        .show_value(false),
                )
                .on_hover_text(tr("Double-clic : 0 dB"));
            if enabled && fader.changed() {
                *gain = super::position_gain(position);
            }
            let steps = if enabled {
                super::scroll_steps(ui, fader.hovered())
            } else {
                0.0
            };
            if steps != 0.0 {
                *gain = super::position_gain(
                    (super::gain_position(*gain) + steps * super::WHEEL_STEP).clamp(0.0, 1.0),
                );
            }
            if enabled
                && fader.hovered()
                && ui.input(|input| {
                    input
                        .pointer
                        .button_double_clicked(egui::PointerButton::Primary)
                })
            {
                *gain = 1.0;
            }
            let meter_rect = egui::Rect::from_center_size(
                egui::pos2(
                    ui.max_rect().left() + width * 2.0 / 3.0,
                    fader.rect.center().y,
                ),
                egui::vec2(12.0, 180.0),
            );
            super::gain_meter(ui, meter_rect, level, *gain);
        },
    );
}

#[derive(Clone)]
pub struct Strip {
    pub id: String,
    pub name: String,
    pub auto_name: String,
    pub selected_outputs: Vec<Option<String>>,
    pub volume: f32,
    pub mix_gains: BTreeMap<EndpointKey, f32>,
    pub trim: f32,
    pub delay_ms: u32,
    pub pan: i8,
    pub noise_suppression: bool,
    pub level: f32,
    pub muted: bool,
    pub solo: bool,
    pub pfl: bool,
    pub present: bool,
    pub last_seen: Option<Instant>,
    pub top_height: f32,
    pub bottom_height: f32,
    editing_name: bool,
    name_focus: bool,
}

impl Default for Strip {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: tr("Tranche").to_string(),
            auto_name: tr("Tranche").to_string(),
            selected_outputs: vec![None],
            volume: 1.0,
            mix_gains: BTreeMap::new(),
            trim: 1.0,
            delay_ms: 0,
            pan: 0,
            noise_suppression: false,
            level: 0.0,
            muted: false,
            solo: false,
            pfl: false,
            present: true,
            last_seen: None,
            top_height: 0.0,
            bottom_height: 0.0,
            editing_name: false,
            name_focus: false,
        }
    }
}

impl Strip {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name: name.clone(),
            auto_name: name,
            ..Default::default()
        }
    }

    fn toggle_solo(&mut self) {
        if self.solo {
            self.solo = false;
        } else {
            self.solo = true;
            self.muted = false;
        }
    }

    fn set_gain(&mut self, key: Option<EndpointKey>, value: f32) {
        match key {
            Some(key) => {
                self.mix_gains.insert(key, value);
            }
            None => self.volume = value,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        outputs: &[crate::audio::devices::AudioEndpoint],
        aliases: &HashMap<String, String>,
        rename: &mut Option<String>,
        _reset: &mut Option<String>,
        cue: Option<&str>,
        gain_edit: GainEdit,
    ) -> egui::Rect {
        ui.vertical(|ui| {
            ui.set_min_width(super::COLUMN_WIDTH);
            ui.set_max_width(super::COLUMN_WIDTH);
            let max_rect = ui.max_rect();
            let visible_rect =
                super::strip_visible_rect(max_rect, super::footer_line_y(ui, max_rect));
            ui.painter()
                .rect_filled(visible_rect, 0.0, super::CONTROL_VEIL);
            let start_top = ui.cursor().top();

            if !self.present {
                ui.painter()
                    .rect_filled(visible_rect, 4.0, super::OFFLINE_TINT);
            }

            // Nom (en haut, renommable au double-clic)
            self.name_ui(ui);
            ui.add_space(8.0);

            // 1. Sélecteurs de sortie (ligne horizontale pour chacun, bouton - sauf le premier)
            let selectors_top = ui.cursor().top();
            let mut remove_index: Option<usize> = None;
            let slots = self.selected_outputs.clone();
            let own_app = own_app(&self.id);
            for (i, sel) in self.selected_outputs.iter_mut().enumerate() {
                let excluded = excluded_ids(&slots, i, cue, own_app.as_deref());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let combo_width = super::COLUMN_WIDTH - 24.0 - ui.spacing().item_spacing.x;
                    let label = sel
                        .as_ref()
                        .and_then(|id| outputs.iter().find(|o| &o.id == id))
                        .map(|o| super::endpoint_label(aliases, o).to_string())
                        .unwrap_or_else(|| tr("(vide)").to_string());
                    let combo = egui::ComboBox::from_id_salt(("out", self.id.as_str(), i))
                        .width(combo_width)
                        .truncate()
                        .selected_text(super::combo_text(ui, &label, combo_width))
                        .show_ui(ui, |ui| {
                            super::endpoint_items(ui, sel, outputs, aliases, &excluded);
                        });
                    let combo_height = combo.response.rect.height();
                    let response = combo
                        .response
                        .on_hover_text(tr("Double-clic ou clic droit : renommer la sortie"));
                    if (response.double_clicked() || response.secondary_clicked())
                        && sel.as_ref().is_some()
                    {
                        *rename = sel.clone();
                    }
                    if i > 0
                        && ui
                            .add(egui::Button::new("-").min_size([24.0, combo_height].into()))
                            .clicked()
                    {
                        remove_index = Some(i);
                    }
                });
                ui.add_space(SELECTOR_ROW_SPACING);
            }

            if let Some(idx) = remove_index {
                if self.selected_outputs.len() > 1 {
                    self.selected_outputs.remove(idx);
                }
            }

            // 2. Bouton + (masqué quand le maximum est atteint)
            if self.selected_outputs.len() < MAX_SELECTORS
                && ui
                    .add(egui::Button::new("+").min_size([24.0, 24.0].into()))
                    .clicked()
            {
                self.selected_outputs.push(None);
            }
            let selector_height = ui.cursor().top() - selectors_top;
            ui.add_space((MAX_SELECTORS as f32 * SELECTOR_ROW_PITCH - selector_height).max(0.0));
            ui.add_space(8.0);
            self.top_height = ui.cursor().top() - start_top;

            // 3. Bas de tranche ancré en bas; la zone des sélecteurs s'étire au-dessus
            self.bottom_height = ui
                .with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.add_space(10.0);

                    // Espace réservé (une ligne) pour aligner les faders sur le nom CUE/PFL du master
                    ui.add_space(super::cue_name_height(ui));
                    ui.add_space(8.0);

                    let width = ui.available_width();
                    let mix_key = match &gain_edit {
                        GainEdit::Mix(key) => Some(key.clone()),
                        _ => None,
                    };
                    let enabled = !matches!(gain_edit, GainEdit::Disabled);
                    let id = self.id.clone();
                    let level = self.level;
                    let original = mix_key
                        .as_ref()
                        .and_then(|key| self.mix_gains.get(key).copied())
                        .unwrap_or(self.volume);
                    let mut value = original;
                    gain_ui(ui, &id, &mut value, level, width, enabled);
                    if value != original {
                        self.set_gain(mix_key, value);
                    }
                    ui.add_space(2.0);
                    super::pan_trim(ui, ("pan", id.as_str()), &mut self.pan, width, 16.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, KNOB_ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let group_width = KNOB_WIDTH * 2.0 + KNOB_GAP;
                            ui.add_space(((width - group_width).max(0.0)) / 2.0);
                            super::gain_knob(
                                ui,
                                ("gain", id.as_str()),
                                &mut self.trim,
                                KNOB_WIDTH,
                                KNOB_ROW_HEIGHT,
                            );
                            ui.add_space(KNOB_GAP);
                            super::delay_knob(
                                ui,
                                ("delay", id.as_str()),
                                &mut self.delay_ms,
                                KNOB_WIDTH,
                                KNOB_ROW_HEIGHT,
                            );
                        },
                    );
                    if self.id.starts_with("input:") {
                        ui.add_space(2.0);
                        let base_fill = ui.visuals().widgets.inactive.weak_bg_fill;
                        let nr_fill = super::fade_fill(
                            ui.ctx(),
                            egui::Id::unique((id.as_str(), "nr")),
                            self.noise_suppression,
                            base_fill,
                            super::PFL_ACTIVE,
                        );
                        let nr_button = egui::Button::new(if self.noise_suppression {
                            "NR on"
                        } else {
                            "NR off"
                        })
                        .min_size([super::BUTTON_WIDTH, 20.0].into())
                        .fill(nr_fill);
                        if ui
                            .add(nr_button)
                            .on_hover_text(tr(
                                "Réduction de bruit RNNoise (48 kHz, ~10 ms ; plus avec resampling).",
                            ))
                            .clicked()
                        {
                            self.noise_suppression = !self.noise_suppression;
                        }
                    }
                    ui.add_space(3.0);

                    // Solo / Mute / PFL empilés (PFL au-dessus)
                    let base_fill = ui.visuals().widgets.inactive.weak_bg_fill;
                    let solo_fill = super::fade_fill(
                        ui.ctx(),
                        egui::Id::unique((id.as_str(), "solo")),
                        self.solo,
                        base_fill,
                        super::SOLO_ACTIVE,
                    );
                    let solo_button = egui::Button::new("Solo")
                        .min_size([super::BUTTON_WIDTH, 24.0].into())
                        .fill(solo_fill);
                    if ui.add(solo_button).clicked() {
                        self.toggle_solo();
                    }
                    ui.add_space(4.0);
                    let mute_label = if self.muted { "Unmute" } else { "Mute" };
                    let mute_fill = super::fade_fill(
                        ui.ctx(),
                        egui::Id::unique((id.as_str(), "mute")),
                        self.muted,
                        base_fill,
                        super::MUTE_ACTIVE,
                    );
                    let mute_button = egui::Button::new(mute_label)
                        .min_size([super::BUTTON_WIDTH, 24.0].into())
                        .fill(mute_fill);
                    if ui.add(mute_button).clicked() {
                        self.muted = !self.muted;
                    }
                    ui.add_space(4.0);
                    let pfl_fill = super::fade_fill(
                        ui.ctx(),
                        egui::Id::unique((id.as_str(), "pfl")),
                        self.pfl,
                        base_fill,
                        super::PFL_ACTIVE,
                    );
                    let pfl_button = egui::Button::new("PFL")
                        .min_size([super::BUTTON_WIDTH, 24.0].into())
                        .fill(pfl_fill);
                    if ui.add(pfl_button).clicked() {
                        self.pfl = !self.pfl;
                    }

                    ui.min_rect().height()
                })
                .inner;
        })
        .response
        .rect
    }

    fn name_ui(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let height = super::name_block_height(ui);

        if self.editing_name {
            let mut finished = false;
            ui.allocate_ui_with_layout(
                egui::vec2(width, height),
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.name)
                            .desired_width(super::BUTTON_WIDTH)
                            .align(egui::Align2::CENTER_CENTER),
                    );
                    if self.name_focus {
                        resp.request_focus();
                        self.name_focus = false;
                    }
                    if resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        finished = true;
                    }
                },
            );
            if finished {
                let trimmed = self.name.trim();
                self.name = if trimmed.is_empty() {
                    self.auto_name.clone()
                } else {
                    trimmed.to_string()
                };
                self.editing_name = false;
            }
        } else {
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
            super::paint_name(
                ui,
                rect,
                &self.name,
                egui::TextStyle::Body.resolve(ui.style()),
            );
            let resp = match pid_tooltip(&self.id) {
                Some(text) => resp.on_hover_text(text),
                None => resp,
            };
            if resp.double_clicked() || resp.secondary_clicked() {
                self.editing_name = true;
                self.name_focus = true;
            }
        }
    }
}

fn pid_tooltip(id: &str) -> Option<String> {
    crate::audio::devices::endpoint_pid(id).map(|pid| format!("PID {pid}"))
}

pub fn own_app(id: &str) -> Option<String> {
    id.strip_prefix("session:").map(|pid| format!("app:{pid}"))
}

fn excluded_ids<'a>(
    slots: &'a [Option<String>],
    index: usize,
    cue: Option<&'a str>,
    own_app: Option<&'a str>,
) -> Vec<&'a str> {
    slots
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .filter_map(|(_, id)| id.as_deref())
        .chain(cue)
        .chain(own_app)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{GainEdit, Strip};
    use eframe::egui;

    fn input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(400.0, 800.0),
            )),
            ..Default::default()
        }
    }

    #[test]
    fn strip_keeps_selector_area_height_when_adding_outputs() {
        let ctx = egui::Context::default();
        let mut strip = Strip::new("session:1".into(), "App".into());
        let aliases = std::collections::HashMap::new();
        let mut rename = None;
        let mut reset = None;

        let output = ctx.run_ui(input(), |ui| {
            strip.ui(
                ui,
                &[],
                &aliases,
                &mut rename,
                &mut reset,
                None,
                GainEdit::Volume,
            );
        });
        output.drop_without_applying_deltas();
        let one_selector = strip.top_height;
        assert!(one_selector > 0.0, "top_height={one_selector}");
        assert!(
            strip.bottom_height > 0.0 && strip.bottom_height < 430.0,
            "bottom_height={}",
            strip.bottom_height
        );

        while strip.selected_outputs.len() < super::MAX_SELECTORS {
            strip.selected_outputs.push(None);
        }
        let output = ctx.run_ui(input(), |ui| {
            strip.ui(
                ui,
                &[],
                &aliases,
                &mut rename,
                &mut reset,
                None,
                GainEdit::Volume,
            );
        });
        output.drop_without_applying_deltas();
        assert!(
            (strip.top_height - one_selector).abs() < 0.5,
            "top_height={} != {one_selector}",
            strip.top_height
        );
    }

    #[test]
    fn pid_tooltip_is_limited_to_process_endpoints() {
        assert_eq!(super::pid_tooltip("session:42"), Some("PID 42".to_string()));
        assert_eq!(super::pid_tooltip("app:7"), Some("PID 7".to_string()));
        assert_eq!(super::pid_tooltip("input:mic"), None);
    }

    #[test]
    fn excluded_ids_lists_other_slots_and_cue() {
        let slots = vec![
            Some("output:a".to_string()),
            Some("output:b".to_string()),
            Some("output:c".to_string()),
        ];
        assert_eq!(
            super::excluded_ids(&slots, 0, Some("output:cue"), Some("app:7")),
            vec!["output:b", "output:c", "output:cue", "app:7"]
        );
        assert_eq!(
            super::excluded_ids(&slots, 1, None, None),
            vec!["output:a", "output:c"]
        );
    }

    #[test]
    fn own_app_matches_session_pid() {
        assert_eq!(super::own_app("session:42"), Some("app:42".to_string()));
        assert_eq!(super::own_app("app:7"), None);
        assert_eq!(super::own_app("input:mic"), None);
    }

    #[test]
    fn enabling_solo_unmutes_the_strip() {
        let mut strip = Strip::new("input:mic".into(), "Mic".into());
        strip.muted = true;

        strip.toggle_solo();
        assert!(strip.solo);
        assert!(!strip.muted);

        strip.toggle_solo();
        assert!(!strip.solo);
        assert!(!strip.muted);
    }

    #[test]
    fn gain_reset_forces_zero_db_in_mix_mode() {
        use crate::config::profile::EndpointKey;

        let mut strip = Strip::new("session:1".into(), "App".into());
        strip.volume = 0.5;
        let key = EndpointKey::Hardware("output:a".into());
        strip.mix_gains.insert(key.clone(), 0.25);

        strip.set_gain(Some(key.clone()), 1.0);

        assert_eq!(strip.mix_gains.get(&key), Some(&1.0));
        assert_eq!(strip.volume, 0.5);
    }

    #[test]
    fn gain_reset_sets_volume_to_zero_db() {
        let mut strip = Strip::new("session:1".into(), "App".into());
        strip.volume = 0.5;

        strip.set_gain(None, 1.0);

        assert_eq!(strip.volume, 1.0);
    }
}
