
use serde_json::Value;

use crate::chat::agent_preparation as agent_prepare;
use crate::settings::ModelProvider;

fn ceil_div_u32(value: u32, divisor: u32) -> usize {
    value.div_ceil(divisor) as usize
}

fn estimate_openai_tile_image_tokens(
    width: u32,
    height: u32,
    base_tokens: usize,
    tile_tokens: usize,
) -> usize {
    let mut scaled_width = width.max(1) as f64;
    let mut scaled_height = height.max(1) as f64;
    let longest = scaled_width.max(scaled_height);
    if longest > 2048.0 {
        let scale = 2048.0 / longest;
        scaled_width *= scale;
        scaled_height *= scale;
    }
    let shortest = scaled_width.min(scaled_height);
    if shortest > 768.0 {
        let scale = 768.0 / shortest;
        scaled_width *= scale;
        scaled_height *= scale;
    }
    let tiles = (scaled_width / 512.0).ceil().max(1.0) as usize
        * (scaled_height / 512.0).ceil().max(1.0) as usize;
    base_tokens + tiles * tile_tokens
}

fn estimate_openai_patch_image_tokens(
    width: u32,
    height: u32,
    patch_budget: Option<usize>,
    multiplier: f64,
    max_dimension: u32,
) -> usize {
    // Match OMP's order: fit pixel dimensions first, then the patch budget.
    let mut width = width.max(1) as f64;
    let mut height = height.max(1) as f64;
    let longest = width.max(height);
    if longest > max_dimension as f64 {
        let scale = max_dimension as f64 / longest;
        width = (width * scale).round().max(1.0);
        height = (height * scale).round().max(1.0);
    }
    let mut patches = (width / 32.0).ceil() as usize * (height / 32.0).ceil() as usize;
    if let Some(budget) = patch_budget.filter(|budget| patches > *budget) {
        let shrink = (budget as f64 * 32.0 * 32.0 / (width * height)).sqrt();
        let scaled_width = width * shrink / 32.0;
        let scaled_height = height * shrink / 32.0;
        let adjusted = shrink
            * (scaled_width.floor() / scaled_width).min(scaled_height.floor() / scaled_height);
        let resized_width = (width * adjusted).floor() as u32;
        let resized_height = (height * adjusted).floor() as u32;
        // Extreme aspect ratios can round one side to zero; use the budget,
        // not a zero-token image or an iterative one-percent shrink loop.
        patches = if resized_width == 0 || resized_height == 0 {
            budget
        } else {
            (ceil_div_u32(resized_width, 32) * ceil_div_u32(resized_height, 32)).min(budget)
        };
    }
    (patches as f64 * multiplier).ceil() as usize
}

fn estimate_anthropic_image_tokens(model: &str, width: u32, height: u32) -> usize {
    let lower = model.to_ascii_lowercase();
    let high_resolution_opus = lower.contains("opus")
        && (lower.contains("4.7")
            || lower.contains("4-7")
            || lower.contains("4.8")
            || lower.contains("4-8"));
    let cap = if high_resolution_opus { 4_784 } else { 1_600 };
    ((width.max(1) as f64 * height.max(1) as f64) / 750.0)
        .ceil()
        .min(cap as f64) as usize
}

fn estimate_gemini_image_tokens(width: u32, height: u32) -> usize {
    if width <= 384 && height <= 384 {
        return 258;
    }
    let tiles = ceil_div_u32(width.max(1), 768) * ceil_div_u32(height.max(1), 768);
    tiles.max(1) * 258
}

/// Decode only the image header, following OMP's bounded metadata probe.
fn image_header_dimensions(data: &str) -> Option<(u32, u32)> {
    use base64::Engine as _;
    const HEADER_BASE64_CHARS: usize = 65_536_usize.div_ceil(3) * 4;
    let count = data.len().min(HEADER_BASE64_CHARS) / 4 * 4;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data.as_bytes()[..count])
        .ok()?;
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

fn image_url_dimensions(url: &str) -> Option<(u32, u32)> {
    let (metadata, data) = url.strip_prefix("data:")?.split_once(',')?;
    metadata
        .ends_with(";base64")
        .then(|| image_header_dimensions(data))
        .flatten()
}

/// Estimate request image parts, never their encoded length or remote contents.
pub(crate) fn request_image_tokens(
    value: &Value,
    provider: Option<&ModelProvider>,
    model: &str,
) -> usize {
    match value {
        Value::Array(values) => values
            .iter()
            .map(|value| request_image_tokens(value, provider, model))
            .sum(),
        Value::Object(object)
            if object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| agent_prepare::IMAGE_PART_TYPES.contains(&kind)) =>
        {
            let image = object.get("image_url");
            let source = object.get("source");
            let detail = image
                .and_then(|image| image.get("detail"))
                .or_else(|| object.get("detail"))
                .and_then(Value::as_str);
            let url = image
                .and_then(|image| image.as_str().or_else(|| image.get("url")?.as_str()))
                .or_else(|| object.get("dataUrl")?.as_str())
                .or_else(|| source?.get("url")?.as_str());
            let dimensions = if let Some(url) = url {
                image_url_dimensions(url)
            } else {
                source
                    .and_then(|source| source.get("data"))
                    .or_else(|| object.get("data"))
                    .and_then(Value::as_str)
                    .and_then(image_header_dimensions)
            };
            estimate_image_tokens(provider, model, dimensions, detail)
        }
        Value::Object(object) => object
            .values()
            .map(|value| request_image_tokens(value, provider, model))
            .sum(),
        _ => 0,
    }
}

pub(crate) fn runtime_message_image_tokens(
    message: &crate::chat::agent::history::RuntimeMessage,
    provider: Option<&ModelProvider>,
    model: &str,
) -> usize {
    use crate::chat::model::MessagePart;

    message
        .model_message()
        .into_iter()
        .flat_map(|message| &message.content)
        .map(|part| match part {
            MessagePart::Image {
                data, detail, path, ..
            } if !data.is_empty() || path.is_some() => estimate_image_tokens(
                provider,
                model,
                image_header_dimensions(data),
                detail.as_deref(),
            ),
            MessagePart::ImageUrl { url, detail } => estimate_image_tokens(
                provider,
                model,
                image_url_dimensions(url),
                detail.as_deref(),
            ),
            MessagePart::AnthropicNativeBlock { block, .. } => {
                request_image_tokens(block, provider, model)
            }
            _ => 0,
        })
        .sum()
}

/// Model/detail rules apply equally to decoded images and unknown-size budgets.
/// OMP supplies the sizing approach; model parameters follow OpenAI's vision guide.
pub(crate) fn estimate_image_tokens(
    provider: Option<&ModelProvider>,
    model: &str,
    dimensions: Option<(u32, u32)>,
    detail: Option<&str>,
) -> usize {
    // Avoid allocating a lowercased provider/model descriptor for every image.
    let identifies = |name: &str| {
        let contains = |value: &str| {
            value
                .as_bytes()
                .windows(name.len())
                .any(|part| part.eq_ignore_ascii_case(name.as_bytes()))
        };
        contains(model)
            || provider.is_some_and(|provider| {
                contains(&provider.name)
                    || contains(&provider.base_url)
                    || contains(&provider.api_format)
            })
    };
    if identifies("anthropic") || identifies("claude") {
        return dimensions.map_or_else(
            || estimate_anthropic_image_tokens(model, u32::MAX, u32::MAX),
            |(width, height)| estimate_anthropic_image_tokens(model, width, height),
        );
    }
    if identifies("gemini") || identifies("google") {
        return dimensions.map_or(1_600, |(width, height)| {
            estimate_gemini_image_tokens(width, height)
        });
    }

    let patch = |budget: Option<usize>, multiplier: f64, max_dimension: u32| {
        dimensions.map_or_else(
            // Original detail on newer models has a rejection limit, not a
            // resizing budget. Unknown inputs use the largest accepted count.
            || (budget.unwrap_or(30_000) as f64 * multiplier).ceil() as usize,
            |(width, height)| {
                estimate_openai_patch_image_tokens(width, height, budget, multiplier, max_dimension)
            },
        )
    };
    let astra = identifies("gpt-6-astra");
    let extended_original = astra || identifies("gpt-5.6") || identifies("gpt-5-6");
    let v55 = identifies("gpt-5.5") || identifies("gpt-5-5");
    let v54 = identifies("gpt-5.4") || identifies("gpt-5-4");
    if extended_original || v55 || v54 {
        return match detail.unwrap_or("auto") {
            "low" if v54 => patch(Some(6_144), 1.2, 2_048),
            "low" => patch(Some(256), 1.2, 512),
            "high" => patch(Some(2_500), 1.2, if astra { 65_535 } else { 2_048 }),
            "auto" if v54 => patch(Some(2_500), 1.2, 2_048),
            _ if extended_original => patch(None, 1.2, 65_535),
            _ => patch(Some(10_000), 1.2, 6_000),
        };
    }
    if identifies("gpt-5.2") || identifies("gpt-5-2") {
        return patch(Some(6_144), 1.2, 2_048);
    }
    if identifies("gpt-4.1-mini") || identifies("gpt-4-1-mini") {
        return patch(Some(6_144), 1.62, 2_048);
    }
    if identifies("gpt-4.1-nano") || identifies("gpt-4-1-nano") {
        return patch(Some(1_536), 2.46, 2_048);
    }
    if identifies("gpt-5-mini") {
        return patch(Some(1_536), 1.2, 2_048);
    }
    if identifies("gpt-5-nano") {
        return patch(Some(1_536), 1.5, 2_048);
    }
    if identifies("o4-mini") {
        return patch(Some(1_536), 1.72, 2_048);
    }
    let (base, tile) = if identifies("gpt-4o-mini") {
        (2_833, 5_667)
    } else if identifies("gpt-5") {
        (70, 140)
    } else if identifies("o1") || identifies("o3") {
        (75, 150)
    } else if identifies("computer-use") {
        (65, 129)
    } else {
        (85, 170)
    };
    if detail == Some("low") {
        return base;
    }
    // A 2048px long side and at most 768px short side cover at most eight tiles.
    dimensions.map_or(base + 8 * tile, |(width, height)| {
        estimate_openai_tile_image_tokens(width, height, base, tile)
    })
}
