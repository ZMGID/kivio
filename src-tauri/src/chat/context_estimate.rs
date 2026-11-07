use serde_json::Value;

use crate::chat::agent_preparation as agent_prepare;
use crate::settings::ModelProvider;

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

pub(crate) fn estimate_image_tokens(
    provider: Option<&ModelProvider>,
    model: &str,
    dimensions: Option<(u32, u32)>,
    detail: Option<&str>,
) -> usize {
    let descriptor = provider.map(|p| kivio_agent_sdk::context::ImageModelDescriptor {
        name: &p.name,
        base_url: &p.base_url,
        api_format: &p.api_format,
    });
    kivio_agent_sdk::context::estimate_image_tokens(descriptor.as_ref(), model, dimensions, detail)
}
