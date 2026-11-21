use std::io::Read;
use std::path::Path;

use crate::config::profile::AppearanceProfile;
use eframe::egui;
use image::GenericImageView;

use super::CONTROL_VEIL;

const CONTRAST_THRESHOLD: f32 = 110.0;

#[derive(Clone, PartialEq)]
struct GradientKey {
    from: [u8; 3],
    to: [u8; 3],
    angle: u16,
}

struct LoadedImage {
    texture: egui::TextureHandle,
    size: egui::Vec2,
    sample: SampleGrid,
}

#[derive(Clone)]
struct SampleGrid {
    size: [usize; 2],
    pixels: Vec<[u8; 3]>,
}

#[derive(Default)]
pub struct BackgroundCache {
    gradient: Option<egui::TextureHandle>,
    gradient_key: Option<GradientKey>,
    gradient_size: [u32; 2],
    image: Option<LoadedImage>,
    image_key: Option<String>,
    missing_image: Option<String>,
}

pub struct DecodedImage {
    pub color: egui::ColorImage,
    pub size: egui::Vec2,
    pub extension: String,
}

const MAX_IMAGE_BYTES: usize = 24 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
const SAMPLE_RES: usize = 64;

pub fn read_limited(path: &Path) -> Option<Vec<u8>> {
    read_limited_with_limit(path, MAX_IMAGE_BYTES as u64)
}

fn read_limited_with_limit(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

pub fn decode(bytes: &[u8]) -> Option<DecodedImage> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let extension = image::guess_format(bytes)
        .ok()?
        .extensions_str()
        .first()
        .copied()?
        .to_string();
    let dynamic = image::load_from_memory(bytes).ok()?;
    let (width, height) = dynamic.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return None;
    }
    let rgba = dynamic.to_rgba8();
    let pixels = rgba
        .pixels()
        .map(|pixel| {
            egui::Color32::from_rgba_unmultiplied(pixel.0[0], pixel.0[1], pixel.0[2], pixel.0[3])
        })
        .collect();
    Some(DecodedImage {
        color: egui::ColorImage {
            size: [width as usize, height as usize],
            source_size: egui::vec2(width as f32, height as f32),
            pixels,
        },
        size: egui::vec2(width as f32, height as f32),
        extension,
    })
}

fn sample_grid(color: &egui::ColorImage) -> SampleGrid {
    let source_width = color.size[0].max(1);
    let source_height = color.size[1].max(1);
    let width = source_width.min(SAMPLE_RES);
    let height = source_height.min(SAMPLE_RES);
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        let y0 = y * source_height / height;
        let y1 = ((y + 1) * source_height / height).max(y0 + 1);
        for x in 0..width {
            let x0 = x * source_width / width;
            let x1 = ((x + 1) * source_width / width).max(x0 + 1);
            let mut sum = [0u32; 3];
            let mut count = 0u32;
            for pixel_y in y0..y1 {
                for pixel_x in x0..x1 {
                    let pixel = color.pixels[pixel_y * source_width + pixel_x];
                    sum[0] += u32::from(pixel.r());
                    sum[1] += u32::from(pixel.g());
                    sum[2] += u32::from(pixel.b());
                    count += 1;
                }
            }
            pixels.push(sum.map(|channel| (channel / count) as u8));
        }
    }
    SampleGrid {
        size: [width, height],
        pixels,
    }
}

fn gradient_t(point: egui::Vec2, angle: u16) -> f32 {
    let radians = f32::from(angle % 360) * std::f32::consts::PI / 180.0;
    let direction = egui::vec2(radians.cos(), radians.sin());
    let extent = (direction.x.abs() + direction.y.abs()) * 0.5;
    (((point - egui::vec2(0.5, 0.5)).dot(direction)) / (2.0 * extent) + 0.5).clamp(0.0, 1.0)
}

fn mix_channel(from: u8, to: u8, t: f32) -> u8 {
    (f32::from(from) + (f32::from(to) - f32::from(from)) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn gradient_texture(
    ctx: &egui::Context,
    appearance: &AppearanceProfile,
    size: egui::Vec2,
) -> egui::TextureHandle {
    let width = size.x.ceil().clamp(1.0, 1024.0) as u32;
    let height = size.y.ceil().clamp(1.0, 1024.0) as u32;
    let angle = appearance.angle % 360;
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let point = egui::vec2(
                x as f32 / (width - 1).max(1) as f32,
                y as f32 / (height - 1).max(1) as f32,
            );
            let t = gradient_t(point, angle);
            pixels.push(egui::Color32::from_rgb(
                mix_channel(appearance.gradient_from[0], appearance.gradient_to[0], t),
                mix_channel(appearance.gradient_from[1], appearance.gradient_to[1], t),
                mix_channel(appearance.gradient_from[2], appearance.gradient_to[2], t),
            ));
        }
    }
    ctx.load_texture(
        "audiok_background_gradient",
        egui::ColorImage {
            size: [width as usize, height as usize],
            source_size: egui::vec2(width as f32, height as f32),
            pixels,
        },
        egui::TextureOptions::LINEAR,
    )
}

fn load_texture(ctx: &egui::Context, file: &str) -> Option<LoadedImage> {
    let path = crate::config::background_path(file)?;
    let decoded = decode(&read_limited(&path)?)?;
    let sample = sample_grid(&decoded.color);
    let texture = ctx.load_texture(
        format!("audiok_background:{file}"),
        decoded.color,
        egui::TextureOptions::LINEAR,
    );
    Some(LoadedImage {
        texture,
        size: decoded.size,
        sample,
    })
}

fn ensure_gradient(
    ctx: &egui::Context,
    appearance: &AppearanceProfile,
    size: egui::Vec2,
    cache: &mut BackgroundCache,
) {
    let key = GradientKey {
        from: appearance.gradient_from,
        to: appearance.gradient_to,
        angle: appearance.angle % 360,
    };
    let texture_size = [size.x.ceil() as u32, size.y.ceil() as u32];
    if cache.gradient.is_some()
        && cache.gradient_key.as_ref() == Some(&key)
        && cache.gradient_size == texture_size
    {
        return;
    }
    cache.gradient = Some(gradient_texture(ctx, appearance, size));
    cache.gradient_key = Some(key);
    cache.gradient_size = texture_size;
}

fn ensure_image(ctx: &egui::Context, appearance: &AppearanceProfile, cache: &mut BackgroundCache) {
    let Some(file) = appearance.image.as_deref() else {
        cache.image = None;
        cache.image_key = None;
        cache.missing_image = None;
        return;
    };
    if cache.image_key.as_deref() == Some(file) || cache.missing_image.as_deref() == Some(file) {
        return;
    }
    if let Some(image) = load_texture(ctx, file) {
        cache.image = Some(image);
        cache.image_key = Some(file.to_string());
        cache.missing_image = None;
    } else {
        cache.image = None;
        cache.image_key = None;
        cache.missing_image = Some(file.to_string());
    }
}

pub fn cover_uv(texture_size: egui::Vec2, viewport_size: egui::Vec2) -> egui::Rect {
    if texture_size.x <= 0.0
        || texture_size.y <= 0.0
        || viewport_size.x <= 0.0
        || viewport_size.y <= 0.0
    {
        return egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    }
    let texture_aspect = texture_size.x / texture_size.y;
    let viewport_aspect = viewport_size.x / viewport_size.y;
    if texture_aspect > viewport_aspect {
        let visible = viewport_aspect / texture_aspect;
        let margin = (1.0 - visible) * 0.5;
        egui::Rect::from_min_max(egui::pos2(margin, 0.0), egui::pos2(1.0 - margin, 1.0))
    } else {
        let visible = texture_aspect / viewport_aspect;
        let margin = (1.0 - visible) * 0.5;
        egui::Rect::from_min_max(egui::pos2(0.0, margin), egui::pos2(1.0, 1.0 - margin))
    }
}

fn sample_at(grid: &SampleGrid, uv: egui::Vec2) -> [u8; 3] {
    let x = (uv.x.clamp(0.0, 1.0) * grid.size[0].saturating_sub(1) as f32).round() as usize;
    let y = (uv.y.clamp(0.0, 1.0) * grid.size[1].saturating_sub(1) as f32).round() as usize;
    grid.pixels[y * grid.size[0] + x]
}

fn sampled_rgb(
    appearance: &AppearanceProfile,
    cache: &BackgroundCache,
    point: egui::Pos2,
    viewport: egui::Rect,
) -> [u8; 3] {
    let x = ((point.x - viewport.min.x) / viewport.width().max(1.0)).clamp(0.0, 1.0);
    let y = ((point.y - viewport.min.y) / viewport.height().max(1.0)).clamp(0.0, 1.0);
    let rgb = if let Some(image) = &cache.image {
        let uv = cover_uv(image.size, viewport.size());
        sample_at(
            &image.sample,
            egui::vec2(uv.min.x + x * uv.width(), uv.min.y + y * uv.height()),
        )
    } else {
        let t = gradient_t(egui::vec2(x, y), appearance.angle % 360);
        [
            mix_channel(appearance.gradient_from[0], appearance.gradient_to[0], t),
            mix_channel(appearance.gradient_from[1], appearance.gradient_to[1], t),
            mix_channel(appearance.gradient_from[2], appearance.gradient_to[2], t),
        ]
    };
    let keep = 1.0 - f32::from(appearance.scrim) / 255.0;
    rgb.map(|channel| (f32::from(channel) * keep).round().clamp(0.0, 255.0) as u8)
}

fn veil_rgb(rgb: [u8; 3], veil: egui::Color32) -> [u8; 3] {
    let alpha = f32::from(veil.a()) / 255.0;
    let [vr, vg, vb, _] = veil.to_array();
    [
        (f32::from(rgb[0]) * (1.0 - alpha) + f32::from(vr) * alpha)
            .round()
            .clamp(0.0, 255.0) as u8,
        (f32::from(rgb[1]) * (1.0 - alpha) + f32::from(vg) * alpha)
            .round()
            .clamp(0.0, 255.0) as u8,
        (f32::from(rgb[2]) * (1.0 - alpha) + f32::from(vb) * alpha)
            .round()
            .clamp(0.0, 255.0) as u8,
    ]
}

fn sampled_content_rgb(
    appearance: &AppearanceProfile,
    cache: &BackgroundCache,
    point: egui::Pos2,
    viewport: egui::Rect,
) -> [u8; 3] {
    veil_rgb(
        sampled_rgb(appearance, cache, point, viewport),
        CONTROL_VEIL,
    )
}

fn luminance(rgb: [u8; 3]) -> f32 {
    0.2126 * f32::from(rgb[0]) + 0.7152 * f32::from(rgb[1]) + 0.0722 * f32::from(rgb[2])
}

struct OutlineColors {
    outer: egui::Color32,
    inner: egui::Color32,
}

fn sampled_luminance_with(
    ctx: &egui::Context,
    appearance: &AppearanceProfile,
    cache: &BackgroundCache,
    rect: egui::Rect,
    sample: impl Fn(&AppearanceProfile, &BackgroundCache, egui::Pos2, egui::Rect) -> [u8; 3],
) -> f32 {
    let viewport = ctx.viewport_rect();
    let rect = rect.expand(1.0);
    let mut sum = [0u32; 3];
    for y in [0.12, 0.5, 0.88] {
        for x in [0.12, 0.5, 0.88] {
            let point = egui::pos2(
                rect.min.x + rect.width() * x,
                rect.min.y + rect.height() * y,
            );
            let rgb = sample(appearance, cache, point, viewport);
            sum[0] += u32::from(rgb[0]);
            sum[1] += u32::from(rgb[1]);
            sum[2] += u32::from(rgb[2]);
        }
    }
    luminance(sum.map(|channel| (channel / 9) as u8))
}

fn sampled_luminance(
    ctx: &egui::Context,
    appearance: &AppearanceProfile,
    cache: &BackgroundCache,
    rect: egui::Rect,
) -> f32 {
    sampled_luminance_with(ctx, appearance, cache, rect, sampled_rgb)
}

pub fn content_luminance(
    ctx: &egui::Context,
    appearance: &AppearanceProfile,
    cache: &BackgroundCache,
    rect: egui::Rect,
) -> f32 {
    sampled_luminance_with(ctx, appearance, cache, rect, sampled_content_rgb)
}

pub fn text_color_for_content(
    ctx: &egui::Context,
    appearance: &AppearanceProfile,
    cache: &BackgroundCache,
    rect: egui::Rect,
) -> egui::Color32 {
    if content_luminance(ctx, appearance, cache, rect) >= CONTRAST_THRESHOLD {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    }
}

pub fn contrast_color(
    ctx: &egui::Context,
    appearance: &AppearanceProfile,
    cache: &BackgroundCache,
    rect: egui::Rect,
) -> egui::Color32 {
    if sampled_luminance(ctx, appearance, cache, rect) >= CONTRAST_THRESHOLD {
        egui::Color32::from_black_alpha(190)
    } else {
        egui::Color32::from_white_alpha(190)
    }
}

fn outline_colors(
    ctx: &egui::Context,
    appearance: &AppearanceProfile,
    cache: &BackgroundCache,
    rect: egui::Rect,
) -> OutlineColors {
    if sampled_luminance(ctx, appearance, cache, rect) >= CONTRAST_THRESHOLD {
        OutlineColors {
            outer: egui::Color32::from_black_alpha(190),
            inner: egui::Color32::from_white_alpha(110),
        }
    } else {
        OutlineColors {
            outer: egui::Color32::from_white_alpha(190),
            inner: egui::Color32::from_black_alpha(110),
        }
    }
}

pub fn paint_outline(
    ctx: &egui::Context,
    painter: &egui::Painter,
    appearance: &AppearanceProfile,
    cache: &BackgroundCache,
    rect: egui::Rect,
) {
    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        return;
    }
    let colors = outline_colors(ctx, appearance, cache, rect);
    painter.rect_stroke(
        rect.expand(2.0),
        5.0,
        egui::Stroke::new(1.0, colors.outer),
        egui::StrokeKind::Outside,
    );
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, colors.inner),
        egui::StrokeKind::Inside,
    );
}

pub fn paint(
    ctx: &egui::Context,
    appearance: &AppearanceProfile,
    cache: &mut BackgroundCache,
) -> Option<String> {
    let rect = ctx.viewport_rect();
    let size = rect.size();
    ensure_gradient(ctx, appearance, size, cache);
    ensure_image(ctx, appearance, cache);
    let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), rect);
    let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if let Some(gradient) = &cache.gradient {
        painter.image(gradient.id(), rect, full, egui::Color32::WHITE);
    }
    if let Some(image) = &cache.image {
        painter.image(
            image.texture.id(),
            rect,
            cover_uv(image.size, size),
            egui::Color32::WHITE,
        );
    }
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(appearance.scrim));
    cache.missing_image.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_limited_refuses_files_above_limit() {
        let path = std::env::temp_dir().join(format!("audiok-bg-limit-{}.bin", std::process::id()));
        std::fs::write(&path, b"123456").unwrap();
        assert!(read_limited_with_limit(&path, 4).is_none());
        assert_eq!(
            read_limited_with_limit(&path, 6).as_deref(),
            Some(&b"123456"[..])
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn gradient_starts_at_low_edge() {
        assert!(gradient_t(egui::vec2(0.0, 0.0), 0).abs() < 0.0001);
        assert!((gradient_t(egui::vec2(1.0, 0.0), 0) - 1.0).abs() < 0.0001);
    }

    #[test]
    fn gradient_angle_zero_is_vertical() {
        assert!(gradient_t(egui::vec2(0.0, 0.0), 90).abs() < 0.0001);
        assert!((gradient_t(egui::vec2(0.0, 1.0), 90) - 1.0).abs() < 0.0001);
    }

    #[test]
    fn cover_uv_crops_wide_image_horizontally() {
        let uv = cover_uv(egui::vec2(200.0, 100.0), egui::vec2(100.0, 100.0));
        assert!((uv.min.x - 0.25).abs() < 0.0001);
        assert!((uv.max.x - 0.75).abs() < 0.0001);
        assert_eq!(uv.min.y, 0.0);
        assert_eq!(uv.max.y, 1.0);
    }

    #[test]
    fn cover_uv_crops_tall_image_vertically() {
        let uv = cover_uv(egui::vec2(100.0, 200.0), egui::vec2(100.0, 100.0));
        assert_eq!(uv.min.x, 0.0);
        assert_eq!(uv.max.x, 1.0);
        assert!((uv.min.y - 0.25).abs() < 0.0001);
        assert!((uv.max.y - 0.75).abs() < 0.0001);
    }

    #[test]
    fn missing_image_falls_back_to_gradient_and_is_reported() {
        let ctx = egui::Context::default();
        let appearance = AppearanceProfile {
            image: Some("missing-background-test.png".into()),
            ..Default::default()
        };
        let mut cache = BackgroundCache::default();
        let missing = paint(&ctx, &appearance, &mut cache);
        assert_eq!(missing.as_deref(), Some("missing-background-test.png"));
        assert!(cache.image.is_none());
        assert!(cache.gradient.is_some());
    }

    #[test]
    fn sample_grid_keeps_pixels_and_scrim_is_applied() {
        let image = egui::ColorImage {
            size: [2, 2],
            source_size: egui::vec2(2.0, 2.0),
            pixels: vec![
                egui::Color32::from_rgb(0, 0, 0),
                egui::Color32::from_rgb(255, 255, 255),
                egui::Color32::from_rgb(255, 255, 255),
                egui::Color32::from_rgb(0, 0, 0),
            ],
        };
        let grid = sample_grid(&image);
        assert_eq!(grid.size, [2, 2]);
        assert_eq!(grid.pixels[0], [0, 0, 0]);
        assert_eq!(grid.pixels[1], [255, 255, 255]);
        assert_eq!(sample_at(&grid, egui::vec2(1.0, 0.0)), [255, 255, 255]);

        let appearance = AppearanceProfile {
            gradient_from: [255, 255, 255],
            gradient_to: [255, 255, 255],
            scrim: 128,
            ..Default::default()
        };
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 100.0));
        let rgb = sampled_rgb(
            &appearance,
            &BackgroundCache::default(),
            egui::pos2(50.0, 50.0),
            viewport,
        );
        assert!((i32::from(rgb[0]) - 127).abs() <= 1);
    }

    #[test]
    fn outline_colors_use_dark_stroke_on_light_background() {
        let ctx = egui::Context::default();
        let appearance = AppearanceProfile {
            gradient_from: [255, 255, 255],
            gradient_to: [255, 255, 255],
            ..Default::default()
        };
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(100.0, 40.0));
        let colors = outline_colors(&ctx, &appearance, &BackgroundCache::default(), rect);
        assert_eq!(colors.outer, egui::Color32::from_black_alpha(190));
        assert_eq!(colors.inner, egui::Color32::from_white_alpha(110));
    }

    #[test]
    fn outline_colors_use_light_stroke_on_dark_background() {
        let ctx = egui::Context::default();
        let appearance = AppearanceProfile {
            gradient_from: [0, 0, 0],
            gradient_to: [0, 0, 0],
            ..Default::default()
        };
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(100.0, 40.0));
        let colors = outline_colors(&ctx, &appearance, &BackgroundCache::default(), rect);
        assert_eq!(colors.outer, egui::Color32::from_white_alpha(190));
        assert_eq!(colors.inner, egui::Color32::from_black_alpha(110));
    }

    #[test]
    fn contrast_color_follows_background_luminance() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(20.0, 2.0));
        let light = AppearanceProfile {
            gradient_from: [255, 255, 255],
            gradient_to: [255, 255, 255],
            ..Default::default()
        };
        let dark = AppearanceProfile {
            gradient_from: [0, 0, 0],
            gradient_to: [0, 0, 0],
            ..Default::default()
        };
        assert_eq!(
            contrast_color(&ctx, &light, &BackgroundCache::default(), rect),
            egui::Color32::from_black_alpha(190)
        );
        assert_eq!(
            contrast_color(&ctx, &dark, &BackgroundCache::default(), rect),
            egui::Color32::from_white_alpha(190)
        );
    }

    #[test]
    fn text_color_for_content_accounts_for_control_veil() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(20.0, 2.0));
        let appearance = AppearanceProfile {
            gradient_from: [130, 130, 130],
            gradient_to: [130, 130, 130],
            scrim: 0,
            ..Default::default()
        };
        assert_eq!(
            contrast_color(&ctx, &appearance, &BackgroundCache::default(), rect),
            egui::Color32::from_black_alpha(190)
        );
        assert_eq!(
            text_color_for_content(&ctx, &appearance, &BackgroundCache::default(), rect),
            egui::Color32::WHITE
        );
    }
}
