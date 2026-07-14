use std::collections::{HashMap, VecDeque};

use image::RgbImage;

use crate::inpainting::{InpaintMask, InpaintingError};
use crate::rapidocr::{RapidOcrLine, RapidOcrPoint};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundComplexity {
    Low,
    Complex,
}

/// 擦除掩膜与背景复杂度共用同一次逐 span 采样的结果。
#[derive(Debug, Clone)]
pub struct TextMaskAnalysis {
    pub mask: InpaintMask,
    pub complexity: BackgroundComplexity,
}

/// 单次遍历所有 span：每个 span 的多边形内采样 + 主背景色簇只计算一次，
/// 同时派生字形擦除掩膜和整图背景复杂度判定（此前 build_text_mask 与
/// classify_background 各自重复了这份逐像素采样）。
pub fn analyze_text_regions(
    image: &RgbImage,
    spans: &[RapidOcrLine],
) -> Result<TextMaskAnalysis, InpaintingError> {
    let (width, height) = image.dimensions();
    let mut data = vec![0u8; width as usize * height as usize];
    let mut complexity = BackgroundComplexity::Low;
    for span in spans {
        let points = normalized_points(span);
        if points.len() < 3 {
            continue;
        }
        let base_samples = polygon_samples(image, &points, 0.0, 0.0);
        let dominant = dominant_background(&base_samples);
        // A flat UI cell/code badge has one coarse color cluster occupying the
        // majority of the OCR polygon even when bold glyphs are large. Photos,
        // gradients, and genuinely mixed surfaces do not.
        let span_is_low =
            base_samples.len() >= 8 && dominant.is_some_and(|(_, coverage)| coverage > 0.52);
        if !span_is_low {
            complexity = BackgroundComplexity::Complex;
        }
        let radius = adaptive_dilation_radius(span.height);
        let background = (base_samples.len() >= 4)
            .then_some(dominant)
            .flatten()
            .map(|(color, _)| color);
        let foreground = foreground_pixels(image, &points, background);
        if foreground.is_empty() {
            // Low-contrast text can defeat color separation. Keep the polygon
            // fallback for correctness, but normal UI/photo text uses the much
            // tighter glyph-derived mask below.
            rasterize_dilated_polygon(&mut data, width, height, &points, radius);
        } else {
            rasterize_dilated_pixels(&mut data, width, height, &foreground, radius);
        }
    }
    Ok(TextMaskAnalysis {
        mask: InpaintMask::new(width, height, data)?,
        complexity,
    })
}

pub fn build_text_mask(
    image: &RgbImage,
    spans: &[RapidOcrLine],
) -> Result<InpaintMask, InpaintingError> {
    analyze_text_regions(image, spans).map(|analysis| analysis.mask)
}

fn foreground_pixels(
    image: &RgbImage,
    points: &[RapidOcrPoint],
    background: Option<[u8; 3]>,
) -> Vec<(u32, u32)> {
    let bounds_height = points
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max)
        - points
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min);
    // OCR boxes can end a few pixels before punctuation/antialiasing. Sample a
    // small outer band, then let contrast filtering decide what is text.
    let horizontal_margin = (bounds_height * 0.22).clamp(2.0, 14.0);
    let vertical_margin = (bounds_height * 0.05).clamp(1.0, 3.0);
    let Some(background) = background else {
        return Vec::new();
    };
    let samples = polygon_samples(image, points, horizontal_margin, vertical_margin);
    let mut distances: Vec<f64> = samples
        .iter()
        .map(|(_, _, pixel)| color_distance(*pixel, background))
        .collect();
    distances.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Text normally occupies less than a quarter of its OCR polygon. Using the
    // upper contrast quartile keeps gradients/texture out of the mask while
    // still retaining antialiased glyph edges after dilation.
    let strongest = *distances.last().unwrap_or(&0.0);
    let contrast_floor = if strongest >= 64.0 {
        // When true dark/colored glyphs exist, exclude low-contrast UI fills
        // such as inline-code badges that happen to sit inside a long OCR box.
        distances[(distances.len() * 3 / 4).min(distances.len() - 1)].max(32.0)
    } else {
        // Preserve genuinely low-contrast text instead of falling back to the
        // whole OCR polygon.
        distances[(distances.len() * 3 / 4).min(distances.len() - 1)].max(12.0)
    };
    samples
        .into_iter()
        .filter_map(|(x, y, pixel)| {
            (color_distance(pixel, background) >= contrast_floor).then_some((x, y))
        })
        .collect()
}

fn polygon_samples(
    image: &RgbImage,
    points: &[RapidOcrPoint],
    horizontal_margin: f32,
    vertical_margin: f32,
) -> Vec<(u32, u32, [u8; 3])> {
    let polygon_min_x = points
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let polygon_min_y = points
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let polygon_max_x = points
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let polygon_max_y = points
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_x = (polygon_min_x - horizontal_margin).floor().max(0.0) as u32;
    let min_y = (polygon_min_y - vertical_margin).floor().max(0.0) as u32;
    let max_x = (polygon_max_x + horizontal_margin)
        .ceil()
        .min(image.width() as f32) as u32;
    let max_y = (polygon_max_y + vertical_margin)
        .ceil()
        .min(image.height() as f32) as u32;

    let mut samples = Vec::new();
    for y in min_y..max_y {
        for x in min_x..max_x {
            let sample_x = x as f32 + 0.5;
            let sample_y = y as f32 + 0.5;
            let in_horizontal_end_band = sample_y >= polygon_min_y - vertical_margin
                && sample_y <= polygon_max_y + vertical_margin
                && ((sample_x < polygon_min_x && sample_x >= polygon_min_x - horizontal_margin)
                    || (sample_x > polygon_max_x && sample_x <= polygon_max_x + horizontal_margin));
            let near_polygon = distance_to_polygon(sample_x, sample_y, points) <= vertical_margin;
            if !point_in_polygon(sample_x, sample_y, points)
                && !in_horizontal_end_band
                && !near_polygon
            {
                continue;
            }
            let pixel = image.get_pixel(x, y).0;
            samples.push((x, y, pixel));
        }
    }
    samples
}

fn dominant_background(samples: &[(u32, u32, [u8; 3])]) -> Option<([u8; 3], f64)> {
    if samples.is_empty() {
        return None;
    }
    let key_for = |pixel: [u8; 3]| {
        ((pixel[0] as u16 >> 5) << 6) | ((pixel[1] as u16 >> 5) << 3) | (pixel[2] as u16 >> 5)
    };
    let mut counts = HashMap::<u16, usize>::new();
    for (_, _, pixel) in samples {
        *counts.entry(key_for(*pixel)).or_default() += 1;
    }
    // HashMap iteration order is randomized. Resolve equal-size clusters
    // deterministically so the same image cannot alternate between treating a
    // dark glyph cluster and the surrounding background as dominant.
    let (&dominant_key, &dominant_count) =
        counts.iter().max_by_key(|(key, count)| (**count, **key))?;
    let mut channels = [
        Vec::with_capacity(dominant_count),
        Vec::with_capacity(dominant_count),
        Vec::with_capacity(dominant_count),
    ];
    for (_, _, pixel) in samples {
        if key_for(*pixel) != dominant_key {
            continue;
        }
        for channel in 0..3 {
            channels[channel].push(pixel[channel]);
        }
    }
    Some((
        [
            median_u8(&mut channels[0]),
            median_u8(&mut channels[1]),
            median_u8(&mut channels[2]),
        ],
        dominant_count as f64 / samples.len() as f64,
    ))
}

fn rasterize_dilated_pixels(
    data: &mut [u8],
    width: u32,
    height: u32,
    pixels: &[(u32, u32)],
    radius: f32,
) {
    let extent = radius.ceil() as i32;
    let radius_sq = radius * radius;
    for &(x, y) in pixels {
        for dy in -extent..=extent {
            for dx in -extent..=extent {
                if (dx * dx + dy * dy) as f32 > radius_sq {
                    continue;
                }
                let next_x = x as i32 + dx;
                let next_y = y as i32 + dy;
                if next_x < 0 || next_y < 0 || next_x >= width as i32 || next_y >= height as i32 {
                    continue;
                }
                data[next_y as usize * width as usize + next_x as usize] = 255;
            }
        }
    }
}

pub fn classify_background(image: &RgbImage, spans: &[RapidOcrLine]) -> BackgroundComplexity {
    analyze_text_regions(image, spans)
        .map(|analysis| analysis.complexity)
        .unwrap_or(BackgroundComplexity::Complex)
}

/// Fast path for flat UI backgrounds. Each glyph pixel samples the nearest
/// unmasked background ring independently, preserving row shading, code badge
/// fills, and gentle gradients instead of flattening an entire OCR span.
pub fn deterministic_fill(
    image: &RgbImage,
    _spans: &[RapidOcrLine],
    mask: &InpaintMask,
) -> RgbImage {
    let mut output = image.clone();
    let width = image.width() as i32;
    let height = image.height() as i32;
    let mut resolved: Vec<bool> = mask.data.iter().map(|value| *value == 0).collect();
    let mut queued = vec![false; mask.data.len()];
    let mut queue = VecDeque::new();

    // First resolve glyph pixels directly from the nearest original background
    // in eight directions. This preserves inline-code badge fills at their
    // boundary instead of letting the surrounding page white bleed inward.
    for y in 0..height {
        for x in 0..width {
            let index = y as usize * width as usize + x as usize;
            if resolved[index] {
                continue;
            }
            let samples = directional_background_samples(image, mask, x, y, 32);
            if samples.is_empty() {
                continue;
            }
            output.put_pixel(x as u32, y as u32, image::Rgb(median_pixels(&samples)));
            resolved[index] = true;
        }
    }

    for y in 0..height {
        for x in 0..width {
            let index = y as usize * width as usize + x as usize;
            if resolved[index] || !has_resolved_neighbor(&resolved, width, height, x, y) {
                continue;
            }
            queued[index] = true;
            queue.push_back((x, y));
        }
    }

    while let Some((x, y)) = queue.pop_front() {
        let index = y as usize * width as usize + x as usize;
        if resolved[index] {
            continue;
        }
        let mut samples = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let next_x = x + dx;
                let next_y = y + dy;
                if next_x < 0 || next_y < 0 || next_x >= width || next_y >= height {
                    continue;
                }
                let neighbor_index = next_y as usize * width as usize + next_x as usize;
                if resolved[neighbor_index] {
                    samples.push(output.get_pixel(next_x as u32, next_y as u32).0);
                }
            }
        }
        if samples.is_empty() {
            continue;
        }
        output.put_pixel(x as u32, y as u32, image::Rgb(median_pixels(&samples)));
        resolved[index] = true;

        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let next_x = x + dx;
                let next_y = y + dy;
                if next_x < 0 || next_y < 0 || next_x >= width || next_y >= height {
                    continue;
                }
                let neighbor_index = next_y as usize * width as usize + next_x as usize;
                if !resolved[neighbor_index] && !queued[neighbor_index] {
                    queued[neighbor_index] = true;
                    queue.push_back((next_x, next_y));
                }
            }
        }
    }
    output
}

fn directional_background_samples(
    image: &RgbImage,
    mask: &InpaintMask,
    x: i32,
    y: i32,
    max_distance: i32,
) -> Vec<[u8; 3]> {
    const DIRECTIONS: [(i32, i32); 8] = [
        (0, -1),
        (0, 1),
        (-1, 0),
        (1, 0),
        (-1, -1),
        (1, -1),
        (-1, 1),
        (1, 1),
    ];
    let width = image.width() as i32;
    let height = image.height() as i32;
    let mut samples = Vec::with_capacity(DIRECTIONS.len());
    for (dx, dy) in DIRECTIONS {
        for distance in 1..=max_distance {
            let sample_x = x + dx * distance;
            let sample_y = y + dy * distance;
            if sample_x < 0 || sample_y < 0 || sample_x >= width || sample_y >= height {
                break;
            }
            let index = sample_y as usize * width as usize + sample_x as usize;
            if mask.data[index] == 0 {
                samples.push(image.get_pixel(sample_x as u32, sample_y as u32).0);
                break;
            }
        }
    }
    samples
}

fn has_resolved_neighbor(resolved: &[bool], width: i32, height: i32, x: i32, y: i32) -> bool {
    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let next_x = x + dx;
            let next_y = y + dy;
            if next_x < 0 || next_y < 0 || next_x >= width || next_y >= height {
                continue;
            }
            if resolved[next_y as usize * width as usize + next_x as usize] {
                return true;
            }
        }
    }
    false
}

fn median_pixels(samples: &[[u8; 3]]) -> [u8; 3] {
    let mut channels = [
        Vec::with_capacity(samples.len()),
        Vec::with_capacity(samples.len()),
        Vec::with_capacity(samples.len()),
    ];
    for pixel in samples {
        for channel in 0..3 {
            channels[channel].push(pixel[channel]);
        }
    }
    [
        median_u8(&mut channels[0]),
        median_u8(&mut channels[1]),
        median_u8(&mut channels[2]),
    ]
}

pub fn adaptive_dilation_radius(span_height: f32) -> f32 {
    (span_height * 0.10).round().clamp(1.0, 4.0)
}

fn normalized_points(span: &RapidOcrLine) -> Vec<RapidOcrPoint> {
    if span.points.len() >= 3 {
        span.points.clone()
    } else {
        vec![
            RapidOcrPoint {
                x: span.x,
                y: span.y,
            },
            RapidOcrPoint {
                x: span.x + span.width,
                y: span.y,
            },
            RapidOcrPoint {
                x: span.x + span.width,
                y: span.y + span.height,
            },
            RapidOcrPoint {
                x: span.x,
                y: span.y + span.height,
            },
        ]
    }
}

fn rasterize_dilated_polygon(
    data: &mut [u8],
    width: u32,
    height: u32,
    points: &[RapidOcrPoint],
    radius: f32,
) {
    let min_x = points
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let min_y = points
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_x = points
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = points
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let x0 = (min_x - radius).floor().max(0.0) as u32;
    let y0 = (min_y - radius).floor().max(0.0) as u32;
    let x1 = (max_x + radius).ceil().min(width as f32) as u32;
    let y1 = (max_y + radius).ceil().min(height as f32) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if point_in_polygon(px, py, points) || distance_to_polygon(px, py, points) <= radius {
                data[y as usize * width as usize + x as usize] = 255;
            }
        }
    }
}

fn point_in_polygon(x: f32, y: f32, points: &[RapidOcrPoint]) -> bool {
    let mut inside = false;
    let mut previous = points[points.len() - 1];
    for current in points {
        if ((current.y > y) != (previous.y > y))
            && x < (previous.x - current.x) * (y - current.y) / (previous.y - current.y) + current.x
        {
            inside = !inside;
        }
        previous = *current;
    }
    inside
}

fn distance_to_polygon(x: f32, y: f32, points: &[RapidOcrPoint]) -> f32 {
    let mut distance = f32::INFINITY;
    let mut previous = points[points.len() - 1];
    for current in points {
        distance = distance.min(distance_to_segment(x, y, previous, *current));
        previous = *current;
    }
    distance
}

fn distance_to_segment(x: f32, y: f32, start: RapidOcrPoint, end: RapidOcrPoint) -> f32 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_sq = dx * dx + dy * dy;
    if length_sq <= f32::EPSILON {
        return ((x - start.x).powi(2) + (y - start.y).powi(2)).sqrt();
    }
    let t = (((x - start.x) * dx + (y - start.y) * dy) / length_sq).clamp(0.0, 1.0);
    let projected_x = start.x + t * dx;
    let projected_y = start.y + t * dy;
    ((x - projected_x).powi(2) + (y - projected_y).powi(2)).sqrt()
}

fn color_distance(a: [u8; 3], b: [u8; 3]) -> f64 {
    let dr = a[0] as f64 - b[0] as f64;
    let dg = a[1] as f64 - b[1] as f64;
    let db = a[2] as f64 - b[2] as f64;
    (dr * dr + dg * dg + db * db).sqrt()
}

fn median_u8(values: &mut [u8]) -> u8 {
    values.sort_unstable();
    values[values.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(points: Vec<RapidOcrPoint>) -> RapidOcrLine {
        RapidOcrLine {
            id: "s0000".into(),
            text: "text".into(),
            points,
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 20.0,
        }
    }

    #[test]
    fn rotated_polygon_does_not_become_axis_aligned_rectangle() {
        let item = span(vec![
            RapidOcrPoint { x: 20.0, y: 10.0 },
            RapidOcrPoint { x: 30.0, y: 20.0 },
            RapidOcrPoint { x: 20.0, y: 30.0 },
            RapidOcrPoint { x: 10.0, y: 20.0 },
        ]);
        let mut image = RgbImage::from_pixel(40, 40, image::Rgb([245, 245, 245]));
        for y in 18..23 {
            for x in 18..23 {
                image.put_pixel(x, y, image::Rgb([20, 20, 20]));
            }
        }
        let mask = build_text_mask(&image, &[item]).expect("build mask");
        assert_eq!(mask.data[20 * 40 + 20], 255);
        assert_eq!(mask.data[11 * 40 + 11], 0);
    }

    #[test]
    fn mask_is_clipped_at_image_boundary() {
        let mut item = span(Vec::new());
        item.x = -5.0;
        item.y = -5.0;
        item.width = 12.0;
        item.height = 12.0;
        let mut image = RgbImage::from_pixel(10, 10, image::Rgb([245, 245, 245]));
        for y in 0..4 {
            for x in 0..4 {
                image.put_pixel(x, y, image::Rgb([20, 20, 20]));
            }
        }
        let mask = build_text_mask(&image, &[item]).expect("build mask");
        assert_eq!(mask.data.len(), 100);
        assert!(mask.data.iter().any(|value| *value != 0));
    }

    #[test]
    fn glyph_mask_does_not_fill_entire_ocr_polygon() {
        let mut image = RgbImage::from_pixel(40, 40, image::Rgb([245, 245, 245]));
        for y in 12..28 {
            for x in 18..22 {
                image.put_pixel(x, y, image::Rgb([20, 20, 20]));
            }
        }
        let item = span(Vec::new());
        let mask = build_text_mask(&image, &[item]).expect("build mask");
        assert_eq!(mask.data[20 * 40 + 20], 255);
        assert_eq!(mask.data[12 * 40 + 12], 0);
    }

    #[test]
    fn outer_ring_sampling_avoids_dark_glyph_pixels() {
        let mut image = RgbImage::from_pixel(40, 30, image::Rgb([245, 245, 245]));
        for y in 10..20 {
            for x in 12..28 {
                image.put_pixel(x, y, image::Rgb([20, 20, 20]));
            }
        }
        let item = span(Vec::new());
        let mask = build_text_mask(&image, std::slice::from_ref(&item)).expect("build mask");
        let output = deterministic_fill(&image, &[item], &mask);
        assert!(output.get_pixel(20, 15).0[0] > 230);
    }

    #[test]
    fn long_ocr_line_does_not_mask_light_inline_code_background() {
        let mut image = RgbImage::from_pixel(120, 30, image::Rgb([255, 255, 255]));
        for y in 7..23 {
            for x in 45..75 {
                image.put_pixel(x, y, image::Rgb([238, 238, 238]));
            }
        }
        for y in 10..20 {
            for x in 12..16 {
                image.put_pixel(x, y, image::Rgb([25, 25, 25]));
            }
            for x in 45..49 {
                image.put_pixel(x, y, image::Rgb([25, 25, 25]));
            }
            for x in 56..60 {
                image.put_pixel(x, y, image::Rgb([25, 25, 25]));
            }
        }
        let item = RapidOcrLine {
            id: "s0000".into(),
            text: "Download .dmg".into(),
            points: vec![
                RapidOcrPoint { x: 5.0, y: 5.0 },
                RapidOcrPoint { x: 110.0, y: 5.0 },
                RapidOcrPoint { x: 110.0, y: 25.0 },
                RapidOcrPoint { x: 5.0, y: 25.0 },
            ],
            x: 5.0,
            y: 5.0,
            width: 105.0,
            height: 20.0,
        };
        let mask = build_text_mask(&image, &[item]).expect("build mask");
        assert_eq!(mask.data[15 * 120 + 14], 255);
        assert_eq!(mask.data[15 * 120 + 47], 255);
        assert_eq!(mask.data[15 * 120 + 58], 255);
        assert_eq!(mask.data[15 * 120 + 52], 0);
        let filled = deterministic_fill(&image, &[], &mask);
        assert!(filled
            .get_pixel(47, 15)
            .0
            .iter()
            .all(|channel| *channel >= 225));
        assert!(filled
            .get_pixel(47, 15)
            .0
            .iter()
            .all(|channel| *channel <= 245));
    }

    #[test]
    fn horizontal_ocr_expansion_catches_end_punctuation_without_erasing_rule_below() {
        let mut image = RgbImage::from_pixel(100, 100, image::Rgb([255, 255, 255]));
        for y in 35..45 {
            for x in 42..48 {
                image.put_pixel(x, y, image::Rgb([20, 20, 20]));
            }
        }
        for x in 5..95 {
            image.put_pixel(x, 88, image::Rgb([180, 180, 180]));
        }
        let item = RapidOcrLine {
            id: "s0000".into(),
            text: "Heading —".into(),
            points: vec![
                RapidOcrPoint { x: 10.0, y: 10.0 },
                RapidOcrPoint { x: 35.0, y: 10.0 },
                RapidOcrPoint { x: 35.0, y: 70.0 },
                RapidOcrPoint { x: 10.0, y: 70.0 },
            ],
            x: 10.0,
            y: 10.0,
            width: 25.0,
            height: 60.0,
        };
        let mask = build_text_mask(&image, &[item]).expect("build mask");
        assert_eq!(mask.data[40 * 100 + 46], 255);
        assert_eq!(mask.data[88 * 100 + 20], 0);
    }

    #[test]
    fn classifies_uniform_and_textured_backgrounds() {
        let item = span(Vec::new());
        let uniform = RgbImage::from_pixel(50, 50, image::Rgb([240, 240, 240]));
        assert_eq!(
            classify_background(&uniform, std::slice::from_ref(&item)),
            BackgroundComplexity::Low
        );
        let textured = RgbImage::from_fn(50, 50, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgb([0, 0, 0])
            } else {
                image::Rgb([255, 255, 255])
            }
        });
        assert_eq!(
            classify_background(&textured, &[item]),
            BackgroundComplexity::Complex
        );
    }

    #[test]
    fn single_analysis_pass_matches_mask_and_complexity_wrappers() {
        let item = span(Vec::new());
        let uniform = RgbImage::from_pixel(50, 50, image::Rgb([240, 240, 240]));
        let analysis =
            analyze_text_regions(&uniform, std::slice::from_ref(&item)).expect("analyze");
        assert_eq!(analysis.complexity, BackgroundComplexity::Low);
        assert_eq!(
            analysis.mask.data,
            build_text_mask(&uniform, std::slice::from_ref(&item))
                .expect("build mask")
                .data
        );
        let textured = RgbImage::from_fn(50, 50, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgb([0, 0, 0])
            } else {
                image::Rgb([255, 255, 255])
            }
        });
        let analysis = analyze_text_regions(&textured, &[item]).expect("analyze");
        assert_eq!(analysis.complexity, BackgroundComplexity::Complex);
    }

    #[test]
    fn sparse_table_rules_do_not_force_photo_inpainting() {
        let item = span(Vec::new());
        let mut table = RgbImage::from_pixel(120, 80, image::Rgb([255, 255, 255]));
        for x in 0..120 {
            table.put_pixel(x, 8, image::Rgb([190, 195, 200]));
            table.put_pixel(x, 40, image::Rgb([190, 195, 200]));
        }
        assert_eq!(
            classify_background(&table, &[item]),
            BackgroundComplexity::Low
        );
    }

    #[test]
    fn mixed_local_surfaces_force_photo_inpainting() {
        let item = span(Vec::new());
        let image = RgbImage::from_fn(50, 50, |x, _| {
            if x < 20 {
                image::Rgb([250, 250, 250])
            } else {
                image::Rgb([215, 220, 225])
            }
        });
        assert_eq!(
            classify_background(&image, &[item]),
            BackgroundComplexity::Complex
        );
    }

    #[test]
    fn deterministic_fill_uses_dominant_ring_color_without_blending() {
        let mut image = RgbImage::from_pixel(40, 30, image::Rgb([245, 245, 245]));
        for x in 8..12 {
            image.put_pixel(x, 8, image::Rgb([120, 120, 120]));
        }
        let item = span(Vec::new());
        let mask = build_text_mask(&image, std::slice::from_ref(&item)).expect("build mask");
        let output = deterministic_fill(&image, &[item], &mask);
        assert_eq!(output.get_pixel(20, 15).0, [245, 245, 245]);
    }

    #[test]
    fn deterministic_fill_tracks_local_gradient_around_glyphs() {
        let mut image = RgbImage::from_fn(40, 30, |x, _| {
            let value = (x * 6).min(240) as u8;
            image::Rgb([value, value, value])
        });
        for y in 10..25 {
            for x in 18..22 {
                image.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        let item = span(Vec::new());
        for _ in 0..32 {
            let mask = build_text_mask(&image, std::slice::from_ref(&item)).expect("build mask");
            let output = deterministic_fill(&image, std::slice::from_ref(&item), &mask);
            let repaired = output.get_pixel(20, 16).0[0];
            assert!((80..=160).contains(&repaired), "{repaired}");
        }
    }

    #[test]
    fn deterministic_fill_never_changes_pixels_outside_mask() {
        let image = RgbImage::from_fn(40, 30, |x, y| image::Rgb([x as u8, y as u8, 200]));
        let item = span(Vec::new());
        let mask = build_text_mask(&image, std::slice::from_ref(&item)).expect("build mask");
        let output = deterministic_fill(&image, &[item], &mask);
        for (index, (source, result)) in image.pixels().zip(output.pixels()).enumerate() {
            if mask.data[index] == 0 {
                assert_eq!(source, result);
            }
        }
    }
}
