use std::{io::Cursor, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{ImageFormat, ImageReader};
use reqwest::{redirect::Policy, Url};
use rmcp::model::{Icon, IconTheme};
use serde::Serialize;

const MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct McpIconImage {
    pub src: String,
    pub theme: Option<IconTheme>,
}

fn remote_url(src: &str, origin: Option<&str>) -> Option<Url> {
    let url = Url::parse(src).ok()?;
    let origin = Url::parse(origin?).ok()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && origin.scheme() == "https"
        && url.origin() == origin.origin())
    .then_some(url)
}

fn mime_matches(mime: Option<&str>, format: ImageFormat) -> bool {
    match mime.map(|mime| mime.split(';').next().unwrap_or("").trim()) {
        None | Some("application/octet-stream") => true,
        Some("image/png") => format == ImageFormat::Png,
        Some("image/jpeg" | "image/jpg") => format == ImageFormat::Jpeg,
        Some("image/webp") => format == ImageFormat::WebP,
        _ => false,
    }
}

fn raster_icon(bytes: &[u8], declared: Option<&str>, content_type: Option<&str>) -> Option<String> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let format = image::guess_format(bytes).ok()?;
    // shortcut: raster icons only; add an isolated SVG renderer when SVG support is needed.
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) || !mime_matches(declared, format)
        || !mime_matches(content_type, format)
    {
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(128, 128);
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(png.into_inner())
    ))
}

async fn load_icon(client: &reqwest::Client, icon: &Icon, origin: Option<&str>) -> Option<String> {
    let (bytes, content_type) = if let Some(data) = icon.src.strip_prefix("data:") {
        if data.len() > MAX_BYTES.div_ceil(3) * 4 + 100 {
            return None;
        }
        let (header, encoded) = data.split_once(',')?;
        let mime = header.strip_suffix(";base64")?;
        (STANDARD.decode(encoded).ok()?, Some(mime.to_string()))
    } else {
        let url = remote_url(&icon.src, origin)?;
        // This client has no MCP headers, cookies, or redirect handling.
        let mut response = client.get(url).send().await.ok()?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|size| size > MAX_BYTES as u64)
        {
            return None;
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return None;
            }
            bytes.extend_from_slice(&chunk);
        }
        (bytes, mime)
    };
    let declared = icon.mime_type.clone();
    tokio::task::spawn_blocking(move || {
        raster_icon(&bytes, declared.as_deref(), content_type.as_deref())
    })
    .await
    .ok()
    .flatten()
}

pub(super) async fn load_icons(icons: &[Icon], origin: Option<&str>) -> Vec<McpIconImage> {
    let Ok(client) = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return Vec::new();
    };
    let mut images: Vec<McpIconImage> = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(3), async {
        for icon in icons.iter().take(8) {
            if images.iter().any(|image| image.theme == icon.theme) {
                continue;
            }
            if let Some(src) = load_icon(&client, icon, origin).await {
                images.push(McpIconImage {
                    src,
                    theme: icon.theme.clone(),
                });
            }
        }
    })
    .await;
    images
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_icons_require_the_same_https_origin_without_credentials() {
        let origin = Some("https://mcp.example/service");
        assert!(remote_url("https://mcp.example/icon.png", origin).is_some());
        for src in [
            "http://mcp.example/icon.png",
            "https://other.example/icon.png",
            "https://user:secret@mcp.example/icon.png",
            "https://mcp.example:444/icon.png",
            "file:///tmp/icon.png",
            "javascript:alert(1)",
        ] {
            assert!(remote_url(src, origin).is_none(), "{src}");
        }
        assert!(remote_url("https://mcp.example/icon.png", None).is_none());
    }

    #[tokio::test]
    async fn icons_are_validated_resized_and_selected_once_per_theme() {
        let png = include_bytes!("../../../public/kivio-server.png");
        let source = Icon::new(format!("data:image/png;base64,{}", STANDARD.encode(png)))
            .with_mime_type("image/png");
        let invalid = Icon::new("data:image/png;base64,aW52YWxpZA==");
        let icons = load_icons(
            &[
                invalid,
                source.clone(),
                source.clone(),
                source.with_theme(IconTheme::Dark),
            ],
            None,
        )
        .await;
        assert_eq!(icons.len(), 2);
        assert_eq!(icons[0].theme, None);
        assert_eq!(icons[1].theme, Some(IconTheme::Dark));
        let bytes = STANDARD
            .decode(icons[0].src.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (128, 128));
        assert_eq!(image.to_rgba8().get_pixel(0, 0)[3], 0);
        assert!(raster_icon(png, Some("image/jpeg"), None).is_none());
        assert!(raster_icon(&vec![0; MAX_BYTES + 1], None, None).is_none());
        let mut oversized = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2049, 1)
            .write_to(&mut oversized, ImageFormat::Png)
            .unwrap();
        assert!(raster_icon(oversized.get_ref(), None, None).is_none());
        assert!(raster_icon(
            b"<svg onload='alert(1)'></svg>",
            Some("image/svg+xml"),
            None
        )
        .is_none());
    }
}
