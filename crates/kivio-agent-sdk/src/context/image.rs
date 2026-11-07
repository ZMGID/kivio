pub struct ImageModelDescriptor<'a> {
    pub name: &'a str,
    pub base_url: &'a str,
    pub api_format: &'a str,
}

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

/// Model/detail rules apply equally to decoded images and unknown-size budgets.
/// OMP supplies the sizing approach; model parameters follow OpenAI's vision guide.
pub fn estimate_image_tokens(
    provider: Option<&ImageModelDescriptor<'_>>,
    model: &str,
    dimensions: Option<(u32, u32)>,
    detail: Option<&str>,
) -> usize {
    let identifies = |name: &str| {
        let contains = |value: &str| {
            value
                .as_bytes()
                .windows(name.len())
                .any(|part| part.eq_ignore_ascii_case(name.as_bytes()))
        };
        contains(model)
            || provider.is_some_and(|provider| {
                contains(provider.name)
                    || contains(provider.base_url)
                    || contains(provider.api_format)
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
