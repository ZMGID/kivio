//! Raster output is composed once off-thread, so preview, clipboard and export match.
use crate::replace_translation::{layout::*, pipeline::Replacement};
use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};
use std::sync::{Mutex, OnceLock};
struct Placement {
    slot: RenderSlot,
    text: String,
    font: f32,
    height: f32,
    width: f32,
    x: f32,
    y: f32,
}
fn layout(
    fonts: &mut FontSystem,
    text: &str,
    slots: &[&RenderSlot],
    font: f32,
) -> Option<Vec<Placement>> {
    let mut remaining = text.trim();
    let mut placed = Vec::new();
    let line_height = font * 1.18;
    for slot in slots {
        if remaining.is_empty() {
            break;
        }
        let padding = (slot.source_font_px * 0.2).clamp(2., 6.);
        let source_line = matches!(slot.flow, RenderFlow::ExactLine | RenderFlow::ParagraphFlow);
        let x = if slot.align == ReplaceTextAlign::Left {
            slot.anchor.x
        } else {
            slot.bounds.x + padding
        };
        let width = (slot.bounds.x + slot.bounds.width - x - padding).max(1.);
        let y = if source_line || slot.vertical_align == RenderVerticalAlign::Top {
            slot.anchor.y
        } else {
            slot.bounds.y + padding
        };
        let height =
            (slot.bounds.y + slot.bounds.height - y - if source_line { 0. } else { padding })
                .max(1.);
        if height < line_height {
            return None;
        }
        let mut buffer = Buffer::new(fonts, Metrics::new(font, line_height));
        buffer.set_size(Some(width), None);
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_text(
            remaining,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(fonts, false);
        let mut offsets = vec![0];
        offsets.extend(remaining.match_indices('\n').map(|(index, _)| index + 1));
        let mut end = 0usize;
        let mut lines = 0usize;
        for run in buffer.layout_runs() {
            if (source_line && lines == 1) || (lines + 1) as f32 * line_height > height + 0.01 {
                break;
            }
            if run.line_w > width + 0.5 || run.glyphs.iter().any(|glyph| glyph.glyph_id == 0) {
                return None;
            }
            let line_offset = *offsets.get(run.line_i)?;
            let run_end = run
                .glyphs
                .iter()
                .map(|glyph| glyph.end)
                .max()
                .unwrap_or(run.text.len());
            end = line_offset + run_end;
            if run_end == run.text.len() && remaining.as_bytes().get(end) == Some(&b'\n') {
                end += 1;
            }
            lines += 1;
        }
        if end == 0 || !remaining.is_char_boundary(end) {
            return None;
        }
        let part = remaining.get(..end)?.trim_end().to_owned();
        let content_height = lines as f32 * line_height;
        let y = if source_line
            || slot.vertical_align == RenderVerticalAlign::Top
            || slot.kind == ReplaceRegionKind::Paragraph
        {
            y
        } else {
            y + ((height - content_height) / 2.).max(0.)
        };
        placed.push(Placement {
            slot: (*slot).clone(),
            text: part,
            font,
            height: content_height,
            width,
            x,
            y,
        });
        remaining = remaining.get(end..)?.trim_start();
    }
    remaining.is_empty().then_some(placed)
}
pub fn render(result: &Replacement) -> Result<Vec<u8>, String> {
    static FONTS: OnceLock<Mutex<FontSystem>> = OnceLock::new();
    let mut fonts = FONTS
        .get_or_init(|| Mutex::new(FontSystem::new()))
        .lock()
        .map_err(|_| "Font system unavailable")?;
    let mut output = image::load_from_memory(&result.cleaned_png)
        .map_err(|error| error.to_string())?
        .to_rgb8();
    let mut cache = SwashCache::new();
    for group in &result.groups {
        if group.translated.trim().is_empty() || group.translated.trim() == group.source_text.trim()
        {
            continue;
        }
        let mut slots = result
            .slots
            .iter()
            .filter(|slot| slot.group_id == group.id)
            .collect::<Vec<_>>();
        slots.sort_by(|a, b| {
            a.anchor
                .y
                .total_cmp(&b.anchor.y)
                .then(a.anchor.x.total_cmp(&b.anchor.x))
        });
        if slots.is_empty() {
            return Err("译文缺少绘制区域".into());
        }
        let mut sizes = slots
            .iter()
            .map(|slot| slot.source_font_px)
            .filter(|size| size.is_finite() && *size > 0.)
            .collect::<Vec<_>>();
        sizes.sort_by(f32::total_cmp);
        let target = (sizes.get(sizes.len() / 2).copied().unwrap_or(14.) * 1.2).clamp(3., 128.);
        let text = group.translated.replace("\r\n", "\n").replace('\r', "\n");
        let placements = if let Some(placements) = layout(&mut fonts, &text, &slots, target) {
            placements
        } else {
            let mut low = 3.;
            let mut high = target;
            let mut best = layout(&mut fonts, &text, &slots, low).ok_or_else(|| {
                format!(
                    "区域 {} 的译文无法完整排入原图，未生成不完整图片。",
                    group.id
                )
            })?;
            for _ in 0..10 {
                let middle = (low + high) / 2.;
                if let Some(placed) = layout(&mut fonts, &text, &slots, middle) {
                    best = placed;
                    low = middle;
                } else {
                    high = middle;
                }
            }
            best
        };
        for placed in placements {
            let mut buffer = Buffer::new(&mut fonts, Metrics::new(placed.font, placed.font * 1.18));
            buffer.set_size(Some(placed.width), Some(placed.height + 1.));
            buffer.set_wrap(Wrap::WordOrGlyph);
            let align = match placed.slot.align {
                ReplaceTextAlign::Left => Align::Left,
                ReplaceTextAlign::Center => Align::Center,
                ReplaceTextAlign::Right => Align::Right,
            };
            buffer.set_text(
                &placed.text,
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                Some(align),
            );
            let color = placed.slot.source_color.trim_start_matches('#');
            let color = u32::from_str_radix(color, 16).unwrap_or(0x202020);
            let color = Color::rgb((color >> 16) as u8, (color >> 8) as u8, color as u8);
            let bounds = placed.slot.bounds;
            buffer.draw(
                &mut fonts,
                &mut cache,
                color,
                |x, y, width, height, color| {
                    let (red, green, blue, alpha) = color.as_rgba_tuple();
                    for dy in 0..height {
                        for dx in 0..width {
                            let x = placed.x.round() as i32 + x + dx as i32;
                            let y = placed.y.round() as i32 + y + dy as i32;
                            if x < 0
                                || y < 0
                                || x >= output.width() as i32
                                || y >= output.height() as i32
                                || (x as f32) < bounds.x
                                || (y as f32) < bounds.y
                                || x as f32 >= bounds.x + bounds.width
                                || y as f32 >= bounds.y + bounds.height
                            {
                                continue;
                            }
                            let pixel = output.get_pixel_mut(x as u32, y as u32);
                            for (index, source) in [red, green, blue].into_iter().enumerate() {
                                pixel[index] = ((source as u32 * alpha as u32
                                    + pixel[index] as u32 * (255 - alpha as u32)
                                    + 127)
                                    / 255) as u8;
                            }
                        }
                    }
                },
            );
        }
    }
    crate::replace_translation::encode_rgb_png(output)
}
