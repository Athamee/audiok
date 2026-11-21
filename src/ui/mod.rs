use std::collections::HashMap;

use crate::audio::engine::{MAX_DELAY_MS, PAN_MAX, PAN_MIN};
use crate::i18n::tr;

pub mod background;
pub mod master;
pub mod strip;

pub const COLUMN_WIDTH: f32 = 126.0;
pub const BUTTON_WIDTH: f32 = 110.0;
pub const MASTER_PANEL_WIDTH: f32 = COLUMN_WIDTH + 9.0;

pub const MUTE_ACTIVE: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(170, 40, 40);
pub const SOLO_ACTIVE: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(40, 140, 70);
pub const PFL_ACTIVE: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(150, 80, 200);
pub const BYPASS_ACTIVE: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(200, 140, 40);
pub const BYPASS_VEIL: eframe::egui::Color32 = eframe::egui::Color32::from_black_alpha(150);
pub const ACTIVE_VEIL: eframe::egui::Color32 = eframe::egui::Color32::from_black_alpha(10);
pub const CONTROL_VEIL: eframe::egui::Color32 = eframe::egui::Color32::from_black_alpha(60);
pub const KNOB_BACKING: eframe::egui::Color32 = eframe::egui::Color32::from_black_alpha(140);
pub const KNOB_TEXT: eframe::egui::Color32 = eframe::egui::Color32::from_gray(215);
pub const OFFLINE_TINT: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(76, 24, 30);
pub const METER_WARN: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(220, 170, 40);
pub const FADE_SECONDS: f32 = 0.12;

fn mix_color(
    base: eframe::egui::Color32,
    active: eframe::egui::Color32,
    t: f32,
) -> eframe::egui::Color32 {
    base.lerp_to_gamma(active, t.clamp(0.0, 1.0))
}

fn fade_fill(
    ctx: &eframe::egui::Context,
    id: eframe::egui::Id,
    on: bool,
    base: eframe::egui::Color32,
    active: eframe::egui::Color32,
) -> eframe::egui::Color32 {
    let t = ctx.animate_bool_with_time(id, on, FADE_SECONDS);
    mix_color(base, active, t)
}

pub fn active_veil(ctx: &eframe::egui::Context, active: bool) -> eframe::egui::Color32 {
    fade_fill(
        ctx,
        eframe::egui::Id::unique("active_veil"),
        active,
        eframe::egui::Color32::TRANSPARENT,
        ACTIVE_VEIL,
    )
}

pub fn bypass_veil(ctx: &eframe::egui::Context, bypassed: bool) -> eframe::egui::Color32 {
    fade_fill(
        ctx,
        eframe::egui::Id::unique("bypass_veil"),
        bypassed,
        eframe::egui::Color32::TRANSPARENT,
        BYPASS_VEIL,
    )
}

fn meter_color(level: f32) -> eframe::egui::Color32 {
    let warn_from = 10.0_f32.powf(-3.0 / 20.0);
    if level > 1.0 {
        MUTE_ACTIVE
    } else if level >= warn_from {
        METER_WARN
    } else {
        SOLO_ACTIVE
    }
}

pub fn name_block_height(ui: &eframe::egui::Ui) -> f32 {
    ui.text_style_height(&eframe::egui::TextStyle::Body) * 2.0 + 2.0
}

pub fn cue_name_font(ui: &eframe::egui::Ui) -> eframe::egui::FontId {
    let mut font = eframe::egui::TextStyle::Heading.resolve(ui.style());
    font.size -= 2.0;
    font
}

pub fn cue_name_height(ui: &eframe::egui::Ui) -> f32 {
    let font = cue_name_font(ui);
    ui.ctx().fonts_mut(|fonts| fonts.row_height(&font))
}

pub fn combo_text<'a>(ui: &eframe::egui::Ui, text: &'a str, width: f32) -> eframe::egui::Atom<'a> {
    use eframe::egui::AtomExt as _;

    let overhead =
        2.0 * ui.spacing().button_padding.x + ui.spacing().icon_width + ui.spacing().icon_spacing;
    text.atom_max_width((width - overhead).max(0.0))
}

pub fn endpoint_label<'a>(
    aliases: &'a HashMap<String, String>,
    endpoint: &'a crate::audio::devices::AudioEndpoint,
) -> &'a str {
    aliases
        .get(&endpoint.id)
        .map(String::as_str)
        .unwrap_or(&endpoint.name)
}

pub fn endpoint_items(
    ui: &mut eframe::egui::Ui,
    selected: &mut Option<String>,
    endpoints: &[crate::audio::devices::AudioEndpoint],
    aliases: &HashMap<String, String>,
    excluded: &[&str],
) {
    use crate::audio::devices::EndpointKind;

    ui.selectable_value(selected, None, tr("(vide)"));
    for (kind, title) in [
        (EndpointKind::Hardware, tr("Matériel")),
        (EndpointKind::Software, tr("Logiciels")),
    ] {
        let group: Vec<_> = endpoints
            .iter()
            .filter(|e| e.kind == kind && !excluded.contains(&e.id.as_str()))
            .collect();
        if group.is_empty() {
            continue;
        }
        ui.label(eframe::egui::RichText::new(title).small().weak());
        for endpoint in group {
            let label = endpoint_label(aliases, endpoint);
            ui.selectable_value(selected, Some(endpoint.id.clone()), label);
        }
    }
}

pub fn footer_line_y(ui: &eframe::egui::Ui, strips_rect: eframe::egui::Rect) -> f32 {
    strips_rect.bottom() - 10.0 - cue_name_height(ui)
}

pub fn strip_visible_rect(strip_rect: eframe::egui::Rect, line_y: f32) -> eframe::egui::Rect {
    let bottom = (line_y - STRIP_FOOTER_GAP).clamp(strip_rect.top(), strip_rect.bottom());
    eframe::egui::Rect::from_min_max(
        strip_rect.min,
        eframe::egui::pos2(strip_rect.right(), bottom),
    )
}

fn veil_rect(min: eframe::egui::Pos2, max: eframe::egui::Pos2) -> eframe::egui::Rect {
    if min.x < max.x && min.y < max.y {
        eframe::egui::Rect::from_min_max(min, max)
    } else {
        eframe::egui::Rect::NOTHING
    }
}

pub fn veil_rects(full: eframe::egui::Rect, hole: eframe::egui::Rect) -> [eframe::egui::Rect; 4] {
    let hole = hole.intersect(full);
    if hole.width() <= 0.0 || hole.height() <= 0.0 {
        return [
            full,
            eframe::egui::Rect::NOTHING,
            eframe::egui::Rect::NOTHING,
            eframe::egui::Rect::NOTHING,
        ];
    }
    use eframe::egui::pos2;
    [
        veil_rect(full.min, pos2(full.max.x, hole.top())),
        veil_rect(pos2(full.min.x, hole.bottom()), full.max),
        veil_rect(
            pos2(full.min.x, hole.top()),
            pos2(hole.left(), hole.bottom()),
        ),
        veil_rect(
            pos2(hole.right(), hole.top()),
            pos2(full.max.x, hole.bottom()),
        ),
    ]
}

pub fn strips_footer(
    ui: &mut eframe::egui::Ui,
    strips_rect: eframe::egui::Rect,
    line_color: eframe::egui::Color32,
    content: impl FnOnce(&mut eframe::egui::Ui),
) -> bool {
    let band = eframe::egui::Rect::from_min_max(
        eframe::egui::pos2(strips_rect.left(), footer_line_y(ui, strips_rect)),
        eframe::egui::pos2(strips_rect.right(), strips_rect.bottom()),
    );
    let mut help_clicked = false;
    ui.scope_builder(
        eframe::egui::UiBuilder::new()
            .max_rect(band)
            .layout(eframe::egui::Layout::left_to_right(
                eframe::egui::Align::Center,
            )),
        |ui| {
            ui.painter().hline(
                band.x_range(),
                band.top() + 0.5,
                eframe::egui::Stroke::new(1.0, line_color),
            );
            help_clicked |= ui
                .button("?")
                .on_hover_text(tr("Lexique / raccourcis"))
                .clicked();
            content(ui);
        },
    );
    help_clicked
}

pub fn paint_name(
    ui: &eframe::egui::Ui,
    rect: eframe::egui::Rect,
    text: &str,
    font_id: eframe::egui::FontId,
) {
    let color = ui.visuals().text_color();
    let galley = ui
        .painter()
        .layout(text.to_owned(), font_id, color, rect.width());
    let size = galley.size();
    let y = if size.y <= rect.height() {
        rect.center().y - size.y / 2.0
    } else {
        rect.top()
    };
    let pos = eframe::egui::pos2(rect.center().x - size.x / 2.0, y);
    ui.painter().with_clip_rect(rect).galley(pos, galley, color);
}

pub fn gain_meter(ui: &eframe::egui::Ui, rect: eframe::egui::Rect, level: f32, gain: f32) {
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, eframe::egui::Color32::from_gray(25));

    let level = level.max(0.0);
    let position = gain_position(level);
    if position > 0.0 {
        let filled = eframe::egui::Rect::from_min_max(
            eframe::egui::pos2(rect.left(), rect.bottom() - rect.height() * position),
            rect.max,
        );
        let color = meter_color(level);
        painter.rect_filled(filled, 2.0, color);
    }

    let zero_y = rect.bottom() - rect.height() * ZERO_DB_POSITION;
    painter.hline(
        rect.x_range(),
        zero_y,
        eframe::egui::Stroke::new(1.0, BYPASS_ACTIVE),
    );

    let set_y = rect.bottom() - rect.height() * gain_position(gain);
    painter.hline(
        rect.x_range(),
        set_y,
        eframe::egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
    );

    for db in METER_TICKS {
        let y = rect.bottom() - rect.height() * tick_position(db);
        let label = match db {
            Some(db) if db > 0.0 => format!("+{db:.0}"),
            Some(db) => format!("{db:.0}"),
            None => "-∞".to_string(),
        };
        let align = if y <= rect.top() + 1.0 {
            eframe::egui::Align2::LEFT_TOP
        } else if y >= rect.bottom() - 1.0 {
            eframe::egui::Align2::LEFT_BOTTOM
        } else {
            eframe::egui::Align2::LEFT_CENTER
        };
        let color = if db == Some(0.0) {
            BYPASS_ACTIVE
        } else {
            ui.visuals().weak_text_color()
        };
        painter.text(
            eframe::egui::pos2(rect.right() + 3.0, y),
            align,
            label,
            eframe::egui::FontId::proportional(9.0),
            color,
        );
    }
}

pub const MIN_DB: f32 = -60.0;
pub const MAX_DB: f32 = 6.0;
pub const KNEE_DB: f32 = -18.0;
pub const KNEE_POSITION: f32 = 0.05;
pub const ZERO_DB_POSITION: f32 =
    KNEE_POSITION + (0.0 - KNEE_DB) / (MAX_DB - KNEE_DB) * (1.0 - KNEE_POSITION);
pub const WHEEL_STEP: f32 = 0.02;
pub const FADER_PIXELS: f32 = 180.0;
pub const STRIP_FOOTER_GAP: f32 = 8.0;
pub const TRIM_MIN_DB: f32 = -24.0;
pub const TRIM_MAX_DB: f32 = 24.0;
pub const TRIM_STEP_DB: f32 = 0.5;
pub const TRIM_DB_PER_PIXEL: f32 = 0.1;
pub const DELAY_STEP_MS: u32 = 10;
pub const DELAY_MS_PER_PIXEL: f32 = 50.0;
pub const PAN_PER_PIXEL: f32 = 1.0;
pub const METER_TICKS: [Option<f32>; 9] = [
    Some(6.0),
    Some(3.0),
    Some(0.0),
    Some(-3.0),
    Some(-6.0),
    Some(-9.0),
    Some(-12.0),
    Some(-15.0),
    None,
];

fn tick_position(db: Option<f32>) -> f32 {
    db.map_or(0.0, |db| gain_position(10.0_f32.powf(db / 20.0)))
}

pub fn drag_position(position: f32, delta_y: f32) -> f32 {
    (position - delta_y / FADER_PIXELS).clamp(0.0, 1.0)
}

pub fn position_gain(position: f32) -> f32 {
    let position = position.clamp(0.0, 1.0);
    if position <= 0.0 {
        return 0.0;
    }
    let db = if position < KNEE_POSITION {
        MIN_DB + position / KNEE_POSITION * (KNEE_DB - MIN_DB)
    } else {
        KNEE_DB + (position - KNEE_POSITION) / (1.0 - KNEE_POSITION) * (MAX_DB - KNEE_DB)
    };
    10.0_f32.powf(db / 20.0)
}

pub fn gain_position(gain: f32) -> f32 {
    if gain <= 0.0 {
        return 0.0;
    }
    let db = (20.0 * gain.log10()).clamp(MIN_DB, MAX_DB);
    if db < KNEE_DB {
        ((db - MIN_DB) / (KNEE_DB - MIN_DB) * KNEE_POSITION).clamp(0.0, KNEE_POSITION)
    } else {
        (KNEE_POSITION + (db - KNEE_DB) / (MAX_DB - KNEE_DB) * (1.0 - KNEE_POSITION))
            .clamp(KNEE_POSITION, 1.0)
    }
}

pub fn trim_db(gain: f32) -> f32 {
    if gain <= 0.0 {
        return TRIM_MIN_DB;
    }
    (20.0 * gain.log10()).clamp(TRIM_MIN_DB, TRIM_MAX_DB)
}

pub fn trim_gain(db: f32) -> f32 {
    10.0_f32.powf(db.clamp(TRIM_MIN_DB, TRIM_MAX_DB) / 20.0)
}

pub fn trim_position(db: f32) -> f32 {
    (db.clamp(TRIM_MIN_DB, TRIM_MAX_DB) - TRIM_MIN_DB) / (TRIM_MAX_DB - TRIM_MIN_DB)
}

pub fn trim_active(gain: f32) -> bool {
    trim_db(gain).abs() > 0.001
}

pub fn trim_db_dragged(db: f32, delta_y: f32) -> f32 {
    (db - delta_y * TRIM_DB_PER_PIXEL).clamp(TRIM_MIN_DB, TRIM_MAX_DB)
}

pub fn trim_db_scrolled(db: f32, steps: f32) -> f32 {
    (db + steps * TRIM_STEP_DB).clamp(TRIM_MIN_DB, TRIM_MAX_DB)
}

pub fn delay_ms_dragged(delay_ms: u32, delta_y: f32) -> u32 {
    let value = delay_ms as f32 - delta_y * DELAY_MS_PER_PIXEL;
    value.round().clamp(0.0, MAX_DELAY_MS as f32) as u32
}

pub fn delay_ms_scrolled(delay_ms: u32, steps: f32) -> u32 {
    let value = delay_ms as i64 + (steps * DELAY_STEP_MS as f32).round() as i64;
    value.clamp(0, MAX_DELAY_MS as i64) as u32
}

pub fn pan_dragged(pan: i8, delta_x: f32) -> i8 {
    let value = pan as f32 + delta_x * PAN_PER_PIXEL;
    value.round().clamp(PAN_MIN as f32, PAN_MAX as f32) as i8
}

pub fn pan_scrolled(pan: i8, steps: f32) -> i8 {
    let value = pan as i32 + steps.round() as i32;
    value.clamp(PAN_MIN as i32, PAN_MAX as i32) as i8
}

pub fn pan_position(pan: i8) -> f32 {
    (pan.clamp(PAN_MIN, PAN_MAX) as f32 - PAN_MIN as f32) / (PAN_MAX as i32 - PAN_MIN as i32) as f32
}

pub fn format_pan(pan: i8) -> String {
    match pan.cmp(&0) {
        std::cmp::Ordering::Less => crate::trf!("G {}", "{}" => -pan),
        std::cmp::Ordering::Equal => tr("Centre").to_string(),
        std::cmp::Ordering::Greater => crate::trf!("D {}", "{}" => pan),
    }
}

pub fn format_db(gain: f32) -> String {
    if gain <= 0.0 {
        return "-∞ dB".to_string();
    }
    let db = 20.0 * gain.log10();
    if db > 0.0 {
        format!("+{db:.1} dB")
    } else {
        format!("{db:.1} dB")
    }
}

pub fn format_delay(delay_ms: u32) -> String {
    if delay_ms == 0 {
        "0 ms".to_string()
    } else {
        format!("-{delay_ms} ms")
    }
}

pub fn volume_value(
    ui: &mut eframe::egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    gain: &mut f32,
    width: f32,
    value_height: f32,
    enabled: bool,
) {
    use eframe::egui;

    let (value_row, _) =
        ui.allocate_exact_size(egui::vec2(width, value_height), egui::Sense::hover());
    let rect = egui::Rect::from_center_size(
        egui::pos2(value_row.left() + width / 2.0, value_row.center().y),
        egui::vec2(60.0, value_height),
    );
    let response = ui.interact(
        rect,
        ui.scope_id().with(id),
        if enabled {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::hover()
        },
    );
    if enabled && response.dragged() {
        let delta = ui.input(|input| input.pointer.delta().y);
        *gain = position_gain(drag_position(gain_position(*gain), delta));
    }
    if enabled && response.double_clicked() {
        *gain = 1.0;
    }
    if enabled {
        response
            .on_hover_text(tr("Glisser : régler · Double-clic : 0 dB"))
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
    }

    let painter = ui.painter();
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format_db(*gain),
        egui::FontId::proportional(12.0),
        if enabled {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        },
    );
}

struct KnobView<'a> {
    label: &'a str,
    value: String,
    angle: f32,
    needle: bool,
    active: bool,
    accent: eframe::egui::Color32,
}

fn knob_row(
    ui: &mut eframe::egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
    height: f32,
) -> (eframe::egui::Response, eframe::egui::Rect) {
    use eframe::egui;

    let (row, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let response = ui.interact(row, ui.scope_id().with(id), egui::Sense::click_and_drag());
    (response, row)
}

fn gain_angle(db: f32) -> f32 {
    const START: f32 = 2.0 * std::f32::consts::PI / 3.0;
    const SWEEP: f32 = 5.0 * std::f32::consts::PI / 3.0;
    START + trim_position(db) * SWEEP
}

fn gain_arc_range(angle: f32) -> Option<(f32, f32)> {
    const NOON: f32 = 3.0 * std::f32::consts::PI / 2.0;
    if (angle - NOON).abs() < 1e-6 {
        None
    } else if angle > NOON {
        Some((NOON, angle))
    } else {
        Some((angle, NOON))
    }
}

fn paint_knob(ui: &mut eframe::egui::Ui, row: eframe::egui::Rect, view: KnobView<'_>) {
    use eframe::egui;

    let center = egui::pos2(row.center().x, row.top() + 30.0);
    let color = if view.active { view.accent } else { KNOB_TEXT };
    let painter = ui.painter();
    painter.rect_filled(row, 4.0, KNOB_BACKING);
    painter.text(
        egui::pos2(row.center().x, row.top()),
        egui::Align2::CENTER_TOP,
        view.label,
        egui::FontId::proportional(9.0),
        KNOB_TEXT,
    );
    painter.circle_stroke(center, 20.0, egui::Stroke::new(1.0, KNOB_TEXT));
    if view.needle {
        if let Some((from, to)) = gain_arc_range(view.angle) {
            let points = (0..=24)
                .map(|step| {
                    let angle = from + (to - from) * (step as f32 / 24.0);
                    center + egui::vec2(angle.cos(), angle.sin()) * 17.0
                })
                .collect();
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(3.0, view.accent),
            ));
        }
        let tip = center + egui::vec2(view.angle.cos(), view.angle.sin()) * 14.0;
        painter.line_segment([center, tip], egui::Stroke::new(2.0, color));
        painter.circle_filled(center, 3.0, color);
    }
    painter.text(
        egui::pos2(row.center().x, row.bottom()),
        egui::Align2::CENTER_BOTTOM,
        view.value,
        egui::FontId::proportional(9.0),
        KNOB_TEXT,
    );
}

pub fn gain_knob(
    ui: &mut eframe::egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    gain: &mut f32,
    width: f32,
    height: f32,
) {
    use eframe::egui;

    let (response, row) = knob_row(ui, id, width, height);
    let mut db = trim_db(*gain);
    if response.dragged() {
        db = trim_db_dragged(db, ui.input(|input| input.pointer.delta().y));
    }
    let steps = scroll_steps(ui, response.hovered());
    if steps != 0.0 {
        db = trim_db_scrolled(db, steps);
    }
    if response.double_clicked() {
        db = 0.0;
    }
    *gain = trim_gain(db);
    response
        .on_hover_text(crate::trf!(
            "Gain {}\nMolette / glisser : régler · Double-clic : 0 dB",
            "{}" => format_db(*gain)
        ))
        .on_hover_cursor(egui::CursorIcon::ResizeVertical);

    let accent = if db >= 0.0 { METER_WARN } else { PFL_ACTIVE };
    paint_knob(
        ui,
        row,
        KnobView {
            label: "Gain",
            value: format_db(*gain),
            angle: gain_angle(db),
            needle: true,
            active: trim_active(*gain),
            accent,
        },
    );
}

pub fn delay_knob(
    ui: &mut eframe::egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    delay_ms: &mut u32,
    width: f32,
    height: f32,
) {
    use eframe::egui;

    let (response, row) = knob_row(ui, id, width, height);
    if response.dragged() {
        *delay_ms = delay_ms_dragged(*delay_ms, ui.input(|input| input.pointer.delta().y));
    }
    let steps = scroll_steps(ui, response.hovered());
    if steps != 0.0 {
        *delay_ms = delay_ms_scrolled(*delay_ms, steps);
    }
    if response.double_clicked() {
        *delay_ms = 0;
    }
    response
        .on_hover_text(crate::trf!(
            "Retard {}\nMolette / glisser : régler · Double-clic : 0 ms",
            "{}" => format_delay(*delay_ms)
        ))
        .on_hover_cursor(egui::CursorIcon::ResizeVertical);

    paint_knob(
        ui,
        row,
        KnobView {
            label: tr("Retard"),
            value: format_delay(*delay_ms),
            angle: 0.0,
            needle: false,
            active: *delay_ms > 0,
            accent: PFL_ACTIVE,
        },
    );
}

pub fn pan_trim(
    ui: &mut eframe::egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    pan: &mut i8,
    width: f32,
    height: f32,
) {
    use eframe::egui;

    let (row, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let response = ui.interact(row, ui.scope_id().with(id), egui::Sense::click_and_drag());
    if response.dragged() {
        *pan = pan_dragged(*pan, ui.input(|input| input.pointer.delta().x));
    }
    let steps = scroll_steps(ui, response.hovered());
    if steps != 0.0 {
        *pan = pan_scrolled(*pan, steps);
    }
    if response.double_clicked() {
        *pan = 0;
    }
    response
        .on_hover_text(crate::trf!(
            "{}\nMolette / glisser : G/D · Double-clic : Centre",
            "{}" => format_pan(*pan)
        ))
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);

    let track_y = row.top() + 4.0;
    let left = row.left() + 12.0;
    let right = row.right() - 12.0;
    let center_x = (left + right) / 2.0;
    let knob_x = left + (right - left) * pan_position(*pan);
    let weak = ui.visuals().weak_text_color();
    let accent = match (*pan).cmp(&0) {
        std::cmp::Ordering::Less => PFL_ACTIVE,
        std::cmp::Ordering::Equal => weak,
        std::cmp::Ordering::Greater => METER_WARN,
    };
    let active = *pan != 0;
    let painter = ui.painter();
    painter.line_segment(
        [egui::pos2(left, track_y), egui::pos2(right, track_y)],
        egui::Stroke::new(1.0, weak),
    );
    if active {
        painter.line_segment(
            [egui::pos2(center_x, track_y), egui::pos2(knob_x, track_y)],
            egui::Stroke::new(3.0, accent),
        );
    }
    painter.circle_filled(
        egui::pos2(knob_x, track_y),
        4.0,
        if active { accent } else { weak },
    );
    let font = egui::FontId::proportional(9.0);
    painter.text(
        egui::pos2(left, row.bottom()),
        egui::Align2::LEFT_BOTTOM,
        tr("G"),
        font.clone(),
        weak,
    );
    painter.text(
        egui::pos2(center_x, row.bottom()),
        egui::Align2::CENTER_BOTTOM,
        "C",
        font.clone(),
        weak,
    );
    painter.text(
        egui::pos2(right, row.bottom()),
        egui::Align2::RIGHT_BOTTOM,
        tr("D"),
        font,
        weak,
    );
}

fn wheel_steps<'a>(events: impl Iterator<Item = &'a eframe::egui::Event>) -> f32 {
    use eframe::egui::{Event, MouseWheelUnit};
    events
        .filter_map(|event| match event {
            Event::MouseWheel { unit, delta, .. } => Some(match unit {
                MouseWheelUnit::Point => delta.y / 50.0,
                MouseWheelUnit::Line | MouseWheelUnit::Page => delta.y,
            }),
            _ => None,
        })
        .sum()
}

pub fn scroll_steps(ui: &eframe::egui::Ui, hovered: bool) -> f32 {
    if !hovered {
        return 0.0;
    }
    ui.input(|input| wheel_steps(input.events.iter()))
        .clamp(-3.0, 3.0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::audio::devices::{AudioEndpoint, EndpointKind};

    #[test]
    fn alias_overrides_endpoint_name() {
        let endpoint = AudioEndpoint {
            id: "output:1".into(),
            name: "Line 1 (Virtual Audio Cable)".into(),
            kind: EndpointKind::Hardware,
        };
        let aliases = HashMap::from([("output:1".to_string(), "Discord".to_string())]);
        assert_eq!(super::endpoint_label(&aliases, &endpoint), "Discord");
        assert_eq!(
            super::endpoint_label(&HashMap::new(), &endpoint),
            endpoint.name
        );
    }

    #[test]
    fn gain_position_roundtrip_and_db_labels() {
        for gain in [0.0, 0.01, 0.1, 0.5, 1.0, 1.995] {
            let position = super::gain_position(gain);
            let restored = super::position_gain(position);
            assert!(
                (restored - gain).abs() < 1e-4,
                "gain={gain} position={position} restored={restored}"
            );
        }
        assert!((super::gain_position(1.0) - super::ZERO_DB_POSITION).abs() < 1e-5);
        assert!((super::position_gain(1.0) - 1.995_262).abs() < 1e-3);
        assert!((super::position_gain(super::ZERO_DB_POSITION) - 1.0).abs() < 1e-5);
        assert!((super::gain_position(1.995_262) - 1.0).abs() < 1e-4);
        assert_eq!(super::position_gain(0.0), 0.0);
        assert_eq!(super::gain_position(0.0), 0.0);
        assert_eq!(super::gain_position(0.001), 0.0);
        assert_eq!(super::format_db(1.0), "0.0 dB");
        assert_eq!(super::format_db(2.0), "+6.0 dB");
        assert_eq!(super::format_db(0.0), "-∞ dB");
    }

    #[test]
    fn mix_color_interpolates_between_endpoints() {
        let base = eframe::egui::Color32::from_rgb(0, 0, 0);
        let active = eframe::egui::Color32::from_rgb(100, 200, 50);
        assert_eq!(super::mix_color(base, active, 0.0), base);
        assert_eq!(super::mix_color(base, active, 1.0), active);
        assert_eq!(super::mix_color(base, active, 2.0), active);
        let mid = super::mix_color(base, active, 0.5);
        assert!(mid.r() > base.r() && mid.r() < active.r());
    }

    #[test]
    fn meter_ticks_span_the_scale() {
        assert_eq!(super::METER_TICKS.len(), 9);
        assert!((super::tick_position(Some(0.0)) - super::ZERO_DB_POSITION).abs() < 1e-5);
        assert!((super::tick_position(Some(6.0)) - 1.0).abs() < 1e-4);
        assert_eq!(super::tick_position(None), 0.0);
        assert!(super::tick_position(Some(6.0)) > super::tick_position(Some(3.0)));
        assert!(super::tick_position(Some(-15.0)) > super::tick_position(None));
    }

    #[test]
    fn meter_colors_follow_db_thresholds() {
        assert_eq!(super::meter_color(0.5), super::SOLO_ACTIVE);
        assert_eq!(super::meter_color(0.8), super::METER_WARN);
        assert_eq!(
            super::meter_color(10.0_f32.powf(-3.0 / 20.0)),
            super::METER_WARN
        );
        assert_eq!(super::meter_color(1.0), super::METER_WARN);
        assert_eq!(super::meter_color(1.2), super::MUTE_ACTIVE);
    }

    #[test]
    fn meter_scale_is_symmetric_around_zero() {
        let up = super::gain_position(10.0_f32.powf(3.0 / 20.0)) - super::ZERO_DB_POSITION;
        let down = super::ZERO_DB_POSITION - super::gain_position(10.0_f32.powf(-3.0 / 20.0));
        assert!((up - down).abs() < 1e-5, "up={up} down={down}");
        assert!((super::ZERO_DB_POSITION - 0.7625).abs() < 1e-5);
        assert!(
            (super::position_gain(super::KNEE_POSITION) - 10.0_f32.powf(-18.0 / 20.0)).abs() < 1e-3
        );
    }

    #[test]
    fn dragging_up_increases_position() {
        assert!((super::drag_position(0.5, -super::FADER_PIXELS) - 1.0).abs() < 1e-6);
        assert!((super::drag_position(0.5, super::FADER_PIXELS) - 0.0).abs() < 1e-6);
        assert_eq!(super::drag_position(0.5, 0.0), 0.5);
        assert_eq!(super::drag_position(0.1, 10_000.0), 0.0);
    }

    #[test]
    fn wheel_events_become_fader_steps() {
        use eframe::egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, TouchPhase, Vec2};

        fn wheel(unit: MouseWheelUnit, y: f32) -> Event {
            Event::MouseWheel {
                unit,
                delta: Vec2::new(0.0, y),
                phase: TouchPhase::Move,
                source: MouseWheelSource::Unknown,
                modifiers: Modifiers::default(),
            }
        }

        let lines = [wheel(MouseWheelUnit::Line, 2.0)];
        assert_eq!(super::wheel_steps(lines.iter()), 2.0);

        let points = [wheel(MouseWheelUnit::Point, 100.0)];
        assert_eq!(super::wheel_steps(points.iter()), 2.0);
    }

    #[test]
    fn trim_scale_is_centered_clamped_and_active_only_when_set() {
        assert_eq!(super::trim_db(1.0), 0.0);
        assert!((super::trim_position(0.0) - 0.5).abs() < 1e-6);
        assert!((super::trim_gain(24.0) - 15.848_932).abs() < 1e-4);
        assert_eq!(super::trim_gain(30.0), super::trim_gain(24.0));
        assert!(!super::trim_active(1.0));
        assert!(super::trim_active(super::trim_gain(0.5)));
        assert!(super::trim_active(super::trim_gain(-0.5)));
    }

    #[test]
    fn trim_drag_up_boosts_and_down_cuts() {
        assert!((super::trim_db_dragged(0.0, -100.0) - 10.0).abs() < 1e-6);
        assert!((super::trim_db_dragged(0.0, 100.0) + 10.0).abs() < 1e-6);
        assert_eq!(super::trim_db_dragged(0.0, -1_000.0), super::TRIM_MAX_DB);
        assert_eq!(super::trim_db_dragged(0.0, 1_000.0), super::TRIM_MIN_DB);
        assert_eq!(super::trim_db_scrolled(0.0, 1.0), 0.5);
        assert_eq!(super::trim_db_scrolled(0.0, -1.0), -0.5);
    }

    #[test]
    fn delay_and_pan_controls_use_coarse_steps() {
        assert_eq!(super::delay_ms_dragged(500, -10.0), 1000);
        assert_eq!(super::delay_ms_dragged(500, 10.0), 0);
        assert_eq!(super::delay_ms_dragged(0, -100_000.0), super::MAX_DELAY_MS);
        assert_eq!(super::format_delay(0), "0 ms");
        assert_eq!(super::format_delay(100), "-100 ms");
        assert_eq!(super::delay_ms_scrolled(0, 1.0), super::DELAY_STEP_MS);
        assert_eq!(
            super::delay_ms_scrolled(super::MAX_DELAY_MS, 1.0),
            super::MAX_DELAY_MS
        );

        assert_eq!(super::pan_dragged(0, -25.0), -25);
        assert_eq!(super::pan_dragged(0, 25.0), 25);
        assert_eq!(super::pan_dragged(0, -1_000.0), super::PAN_MIN);
        assert_eq!(super::pan_scrolled(0, 1.0), 1);
        assert_eq!(super::pan_scrolled(super::PAN_MAX, 1.0), super::PAN_MAX);
        assert_eq!(super::format_pan(0), "Centre");
        assert_eq!(super::format_pan(-25), "G 25");
        assert_eq!(super::format_pan(25), "D 25");
        assert!((super::pan_position(0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn gain_arc_goes_from_noon_to_each_side() {
        let noon = 3.0 * std::f32::consts::PI / 2.0;
        assert_eq!(super::gain_arc_range(noon), None);
        assert_eq!(
            super::gain_arc_range(super::gain_angle(super::TRIM_MIN_DB)),
            Some((super::gain_angle(super::TRIM_MIN_DB), noon))
        );
        assert_eq!(
            super::gain_arc_range(super::gain_angle(super::TRIM_MAX_DB)),
            Some((noon, super::gain_angle(super::TRIM_MAX_DB)))
        );
    }

    #[test]
    fn gain_angle_is_7h_12h_5h() {
        let low = super::gain_angle(super::TRIM_MIN_DB);
        let center = super::gain_angle(0.0);
        let high = super::gain_angle(super::TRIM_MAX_DB);
        let diagonal = 3.0_f32.sqrt() / 2.0;
        assert!((low.cos() + 0.5).abs() < 1e-6);
        assert!((low.sin() - diagonal).abs() < 1e-6);
        assert!(center.cos().abs() < 1e-6);
        assert!((center.sin() + 1.0).abs() < 1e-6);
        assert!((high.cos() - 0.5).abs() < 1e-6);
        assert!((high.sin() - diagonal).abs() < 1e-6);
    }

    #[test]
    fn long_selected_text_does_not_widen_combo() {
        let ctx = eframe::egui::Context::default();
        let width = 69.0;
        let mut measured = 0.0;

        let output = ctx.run_ui(Default::default(), |ui| {
            let text = "Périphérique de sortie au nom très long";
            measured = eframe::egui::ComboBox::from_id_salt("combo_test")
                .width(width)
                .truncate()
                .selected_text(super::combo_text(ui, text, width))
                .show_ui(ui, |_| {})
                .response
                .rect
                .width();
        });
        output.drop_without_applying_deltas();

        assert!(measured <= width + 0.5, "largeur mesurée: {measured}");
    }

    #[test]
    fn veil_rects_leave_only_the_hole_uncovered() {
        use eframe::egui::{pos2, Rect};

        let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));
        let hole = Rect::from_min_max(pos2(20.0, 30.0), pos2(80.0, 70.0));
        let parts = super::veil_rects(full, hole);

        assert_eq!(
            parts[0],
            Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 30.0))
        );
        assert_eq!(
            parts[1],
            Rect::from_min_max(pos2(0.0, 70.0), pos2(100.0, 100.0))
        );
        assert_eq!(
            parts[2],
            Rect::from_min_max(pos2(0.0, 30.0), pos2(20.0, 70.0))
        );
        assert_eq!(
            parts[3],
            Rect::from_min_max(pos2(80.0, 30.0), pos2(100.0, 70.0))
        );
    }

    #[test]
    fn strip_visible_rect_stops_before_footer() {
        let rect = eframe::egui::Rect::from_min_max(
            eframe::egui::pos2(0.0, 0.0),
            eframe::egui::pos2(100.0, 100.0),
        );
        let visible = super::strip_visible_rect(rect, 92.0);
        assert_eq!(visible.min, rect.min);
        assert_eq!(visible.max.x, rect.right());
        assert_eq!(visible.max.y, 92.0 - super::STRIP_FOOTER_GAP);
        assert_eq!(super::strip_visible_rect(rect, 2.0).max.y, rect.top());
    }

    #[test]
    fn footer_line_y_is_above_strip_bottom() {
        eframe::egui::__run_test_ui(|ui| {
            let rect = eframe::egui::Rect::from_min_max(
                eframe::egui::pos2(0.0, 0.0),
                eframe::egui::pos2(100.0, 100.0),
            );
            let y = super::footer_line_y(ui, rect);
            assert!(y > rect.top() && y < rect.bottom());
        });
    }

    #[test]
    fn layout_uses_fixed_widths() {
        assert_eq!(super::COLUMN_WIDTH, 126.0);
        assert_eq!(super::BUTTON_WIDTH, 110.0);
        assert_eq!(super::MASTER_PANEL_WIDTH, 135.0);
    }
}
