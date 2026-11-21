use std::collections::HashMap;

use eframe::egui;

use crate::i18n::tr;

fn mix_label(
    key: &str,
    outputs: &[crate::audio::devices::AudioEndpoint],
    aliases: &HashMap<String, String>,
) -> String {
    outputs
        .iter()
        .find(|output| output.id == key)
        .map(|output| super::endpoint_label(aliases, output).to_string())
        .unwrap_or_else(|| key.to_string())
}

fn cue_mode_text(ui: &egui::Ui, cue_mix_enabled: bool) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let color = ui.visuals().widgets.inactive.text_color();
    let format = |size: f32| egui::text::TextFormat {
        font_id: egui::FontId::new(size, font.family.clone()),
        color,
        valign: egui::Align::Center,
        ..Default::default()
    };
    let active = format(font.size);
    let inactive = format((font.size - 2.0).max(1.0));
    let mut job = egui::text::LayoutJob::default();
    if cue_mix_enabled {
        job.append(tr("Mixe"), 0.0, active);
        job.append(tr("/casque"), 0.0, inactive);
    } else {
        job.append(tr("Casque"), 0.0, active);
        job.append(tr("/mixe"), 0.0, inactive);
    }
    job
}

pub struct Master {
    pub name: String,
    pub cue_volume: f32,
    pub cue_level: f32,
    pub mix_level: f32,
    pub cue_device: Option<String>,
    pub cue_mix: Option<String>,
    pub cue_mix_enabled: bool,
    pub bypass: bool,
    pub mute_all: bool,
    pub top_height: f32,
    pub bottom_height: f32,
}

impl Default for Master {
    fn default() -> Self {
        Self {
            name: "CUE/PFL".to_string(),
            cue_volume: 1.0,
            cue_level: 0.0,
            mix_level: 0.0,
            cue_device: None,
            cue_mix: None,
            cue_mix_enabled: false,
            bypass: false,
            mute_all: false,
            top_height: 0.0,
            bottom_height: 0.0,
        }
    }
}

impl Master {
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        outputs: &[crate::audio::devices::AudioEndpoint],
        aliases: &HashMap<String, String>,
        mixes: &[String],
        appearance: &crate::config::profile::AppearanceProfile,
        background: &super::background::BackgroundCache,
        rename: &mut Option<String>,
        _reset: &mut Option<String>,
        mix_reset: &mut Option<String>,
    ) -> (bool, egui::Rect, egui::Rect) {
        ui.vertical(|ui| {
            ui.set_min_width(super::COLUMN_WIDTH);
            ui.set_max_width(super::COLUMN_WIDTH);
            let max_rect = ui.max_rect();
            let visible_rect =
                super::strip_visible_rect(max_rect, super::footer_line_y(ui, max_rect));
            ui.painter()
                .rect_filled(visible_rect, 0.0, super::CONTROL_VEIL);
            let start_top = ui.cursor().top();

            let refresh = ui
                .vertical_centered(|ui| {
                    ui.add(
                        egui::Button::new(tr("Rafraîchir"))
                            .min_size([super::BUTTON_WIDTH, 24.0].into()),
                    )
                    .clicked()
                })
                .inner;
            ui.add_space(8.0);

            let mut bypass_rect = egui::Rect::NOTHING;
            let controls = egui::Frame::group(ui.style()).show(ui, |ui| {
                // Bloc master : titre + bypass
                ui.vertical_centered(|ui| {
                    ui.label("Master");
                    let base_fill = ui.visuals().widgets.inactive.weak_bg_fill;
                    let bypass_fill = super::fade_fill(
                        ui.ctx(),
                        egui::Id::unique("master_bypass"),
                        self.bypass,
                        base_fill,
                        super::BYPASS_ACTIVE,
                    );
                    let bypass_button = egui::Button::new("Bypass")
                        .min_size([super::BUTTON_WIDTH, 24.0].into())
                        .fill(bypass_fill);
                    let response = ui.add(bypass_button);
                    bypass_rect = response.rect;
                    if response.clicked() {
                        self.bypass = !self.bypass;
                    }
                    if self.bypass {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(tr("Routage modifiable\npendant le bypass"))
                                    .size(11.0)
                                    .color(super::BYPASS_ACTIVE),
                            )
                            .halign(egui::Align::Center),
                        );
                    }
                });
                ui.add_space(12.0);

                // Bouton mute global
                ui.vertical_centered(|ui| {
                    let base_fill = ui.visuals().widgets.inactive.weak_bg_fill;
                    let mute_fill = super::fade_fill(
                        ui.ctx(),
                        egui::Id::unique("master_mute"),
                        self.mute_all,
                        base_fill,
                        super::MUTE_ACTIVE,
                    );
                    let button = egui::Button::new("MUTE")
                        .min_size([super::BUTTON_WIDTH, 24.0].into())
                        .fill(mute_fill);
                    if ui.add(button).clicked() {
                        self.mute_all = !self.mute_all;
                    }
                });
            });
            let controls_rect = controls.response.rect;
            ui.painter().rect_stroke(
                controls_rect,
                4.0,
                egui::Stroke::new(
                    1.0,
                    super::background::contrast_color(
                        ui.ctx(),
                        appearance,
                        background,
                        controls_rect,
                    ),
                ),
                egui::StrokeKind::Outside,
            );

            // 2. Bas de master ancré en bas, aligné sur les tranches
            self.top_height = ui.cursor().top() - start_top;
            let cue = ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.add_space(16.0);

                // Nom du bus cue (en bas)
                let width = ui.available_width();
                let name_height = super::cue_name_height(ui);
                let (name_rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, name_height), egui::Sense::hover());
                super::paint_name(ui, name_rect, &self.name, super::cue_name_font(ui));
                ui.add_space(8.0);

                // Indicateur de volume cue sous le fader (glisser pour régler)
                super::volume_value(ui, "cue_volume", &mut self.cue_volume, width, 18.0, true);
                ui.add_space(4.0);

                // Fader du cue à 1/3 de la tranche, vumètre à 2/3
                let (level, set_gain) = if self.cue_mix_enabled {
                    (self.mix_level, 1.0)
                } else {
                    (self.cue_level, self.cue_volume)
                };
                let level_text = if self.cue_mix_enabled {
                    match self.cue_mix.as_deref() {
                        None => tr("Aucun mixe -∞ dB").to_string(),
                        Some(key) => format!(
                            "{} {}",
                            mix_label(key, outputs, aliases),
                            super::format_db(level)
                        ),
                    }
                } else {
                    format!("CUE {}", super::format_db(level))
                };
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 180.0),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        ui.spacing_mut().slider_width = 180.0;
                        ui.add_space(width / 3.0 - 10.0);
                        let mut position = super::gain_position(self.cue_volume);
                        let fader = ui
                            .add_sized(
                                [20.0, 180.0],
                                egui::Slider::new(&mut position, 0.0..=1.0)
                                    .vertical()
                                    .show_value(false),
                            )
                            .on_hover_text(tr("Double-clic : 0 dB"));
                        if fader.changed() {
                            self.cue_volume = super::position_gain(position);
                        }
                        let steps = super::scroll_steps(ui, fader.hovered());
                        if steps != 0.0 {
                            self.cue_volume = super::position_gain(
                                (super::gain_position(self.cue_volume) + steps * super::WHEEL_STEP)
                                    .clamp(0.0, 1.0),
                            );
                        }
                        let reset = fader.hovered()
                            && ui.input(|input| {
                                input
                                    .pointer
                                    .button_double_clicked(egui::PointerButton::Primary)
                            });
                        if reset {
                            self.cue_volume = 1.0;
                        }
                        let meter_rect = egui::Rect::from_center_size(
                            egui::pos2(
                                ui.max_rect().left() + width * 2.0 / 3.0,
                                fader.rect.center().y,
                            ),
                            egui::vec2(12.0, 180.0),
                        );
                        super::gain_meter(ui, meter_rect, level, set_gain);
                    },
                );

                ui.allocate_ui_with_layout(
                    egui::vec2(width, 14.0),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        ui.add_space(width * 2.0 / 3.0 - 16.0);
                        ui.add(
                            egui::Label::new(egui::RichText::new(level_text).size(9.0))
                                .truncate()
                                .halign(egui::Align::Center),
                        );
                    },
                );

                ui.add_space(12.0);

                // Sélecteur de sortie du cue
                let cue_label = self
                    .cue_device
                    .as_ref()
                    .and_then(|id| outputs.iter().find(|o| &o.id == id))
                    .map(|o| super::endpoint_label(aliases, o).to_string())
                    .unwrap_or_else(|| tr("(vide)").to_string());
                let combo = egui::ComboBox::from_id_salt("master_cue")
                    .width(super::BUTTON_WIDTH)
                    .truncate()
                    .selected_text(super::combo_text(ui, &cue_label, super::BUTTON_WIDTH))
                    .show_ui(ui, |ui| {
                        super::endpoint_items(ui, &mut self.cue_device, outputs, aliases, &[]);
                    });
                let response = combo
                    .response
                    .on_hover_text(tr("Double-clic ou clic droit : renommer la sortie"));
                if (response.double_clicked() || response.secondary_clicked())
                    && self.cue_device.is_some()
                {
                    *rename = self.cue_device.clone();
                }
                let hint = if self.bypass {
                    tr("Bypass actif :\ncue inactif")
                } else if !self.cue_mix_enabled {
                    tr("Écoute PFL\nsélecteur bas")
                } else if self.cue_mix.is_some() {
                    tr("Réglage du mixe\nsélecteur haut")
                } else {
                    tr("Aucun mixe :\ncue muet")
                };
                ui.add(
                    egui::Label::new(egui::RichText::new(hint).size(10.0))
                        .halign(egui::Align::Center),
                );

                ui.add_space(4.0);
                if self.cue_mix_enabled {
                    ui.allocate_ui_with_layout(
                        egui::vec2(super::BUTTON_WIDTH, 24.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let selected = match self.cue_mix.as_deref() {
                                None => tr("(aucun mixe)").to_string(),
                                Some(key) => mix_label(key, outputs, aliases),
                            };
                            let items: Vec<(String, String)> = mixes
                                .iter()
                                .map(|key| (key.clone(), mix_label(key, outputs, aliases)))
                                .collect();
                            egui::ComboBox::from_id_salt("cue_mix")
                                .width(super::BUTTON_WIDTH)
                                .truncate()
                                .selected_text(super::combo_text(
                                    ui,
                                    &selected,
                                    super::BUTTON_WIDTH,
                                ))
                                .show_ui(ui, |ui| {
                                    if items.is_empty() {
                                        ui.label(tr("(aucun mixe)"));
                                    }
                                    for (key, label) in &items {
                                        let active = self.cue_mix.as_deref() == Some(key.as_str());
                                        if ui
                                            .selectable_label(active, label)
                                            .on_hover_text(tr(
                                                "Écoute post-fader (volume/mute/solo suivis)",
                                            ))
                                            .clicked()
                                        {
                                            self.cue_mix = Some(key.clone());
                                            ui.close();
                                        }
                                    }
                                })
                                .response
                                .on_hover_text(tr("Mixe écouté sur la sortie cue."));
                        },
                    );
                    ui.add_space(4.0);
                }
                ui.allocate_ui_with_layout(
                    egui::vec2(super::BUTTON_WIDTH, 24.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let active = self.cue_mix_enabled && self.cue_mix.is_some();
                        if ui
                            .add_enabled(
                                active,
                                egui::Button::new(tr("Reset mixe"))
                                    .min_size([super::BUTTON_WIDTH, 24.0].into()),
                            )
                            .on_hover_text(tr(
                                "Réinitialiser le mixe sélectionné pour toutes les tranches",
                            ))
                            .clicked()
                        {
                            *mix_reset = self.cue_mix.clone();
                        }
                    },
                );
                ui.add_space(4.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(super::BUTTON_WIDTH, 24.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let text = cue_mode_text(ui, self.cue_mix_enabled);
                        let base_fill = ui.visuals().widgets.inactive.weak_bg_fill;
                        let mixe_fill = super::fade_fill(
                            ui.ctx(),
                            egui::Id::unique("master_mixe"),
                            self.cue_mix_enabled,
                            base_fill,
                            super::PFL_ACTIVE,
                        );
                        let button = egui::Button::new(text)
                            .min_size([super::BUTTON_WIDTH, 24.0].into())
                            .fill(mixe_fill);
                        if ui
                            .add(button)
                            .on_hover_text(tr("Écoute : Casque (PFL) ou Mixe d'une sortie"))
                            .clicked()
                        {
                            self.cue_mix_enabled = !self.cue_mix_enabled;
                            if self.cue_mix_enabled && self.cue_mix.is_none() {
                                self.cue_mix = mixes.first().cloned();
                            }
                        }
                    },
                );

                ui.min_rect().height()
            });
            self.bottom_height = cue.inner;
            (refresh, bypass_rect, visible_rect)
        })
        .inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cue_mode_text_marks_inactive_part_smaller() {
        egui::__run_test_ui(|ui| {
            let casque = cue_mode_text(ui, false);
            assert_eq!(casque.text, "Casque/mixe");
            assert_eq!(casque.sections.len(), 2);
            assert!(
                casque.sections[1].format.font_id.size < casque.sections[0].format.font_id.size
            );

            let mixe = cue_mode_text(ui, true);
            assert_eq!(mixe.text, "Mixe/casque");
            assert_eq!(mixe.sections.len(), 2);
            assert!(
                casque.sections[0].format.font_id.size > casque.sections[1].format.font_id.size
            );
        });
    }

    #[test]
    fn veil_rect_uses_available_area() {
        let outputs = Vec::new();
        let aliases = HashMap::new();
        let mixes = Vec::new();
        let appearance = crate::config::profile::AppearanceProfile::default();
        let background = crate::ui::background::BackgroundCache::default();
        let mut master = Master::default();
        let mut rename = None;
        let mut reset = None;
        let mut mix_reset = None;
        egui::__run_test_ui(|ui| {
            let max_rect = ui.max_rect();
            let expected_bottom =
                crate::ui::strip_visible_rect(max_rect, crate::ui::footer_line_y(ui, max_rect))
                    .bottom();
            let (_, _, rect) = master.ui(
                ui,
                &outputs,
                &aliases,
                &mixes,
                &appearance,
                &background,
                &mut rename,
                &mut reset,
                &mut mix_reset,
            );
            assert!(rect.width() > 0.0);
            assert!(rect.height() > 0.0);
            assert_eq!(rect.bottom(), expected_bottom);
        });
    }
}
