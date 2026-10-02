//! Asynchronous video generation adapters. Every supported vendor follows the same shape —
//! create a paid task once, poll it until it ends, then download the file — but each spells the
//! request, task ID, status and result differently. This module owns those differences; the
//! media station owns the job lifecycle and only calls `create_body` / `poll_url` / `parse_poll`.
use crate::settings::ModelProvider;
use serde_json::{json, Value};

/// Wire protocol of a video endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoApi {
    /// xAI `POST /videos/generations` → `request_id`; `GET /videos/{id}` → `video.url`.
    Xai,
    /// MiniMax v2 `POST /v2/video_generation` → `task_id`; `GET /v2/query/video_generation/{id}` → `task.content.url`.
    Minimax,
    /// Volcengine Ark (Seedance) `POST /contents/generations/tasks` → `id`; same path + `/{id}` → `content.video_url`.
    Ark,
    /// Alibaba DashScope (Wan) `POST /services/aigc/video-generation/video-synthesis` (async header) → `output.task_id`;
    /// `GET /tasks/{id}` → `output.video_url`.
    DashScope,
}

/// Options of one generation, already validated by the caller.
pub struct VideoRequest<'a> {
    pub model: &'a str,
    pub prompt: &'a str,
    pub aspect_ratio: &'a str,
    pub duration: u32,
    /// First frame as a `data:` URL; every adapter here accepts base64 data URLs.
    pub first_frame: Option<String>,
}

/// One poll result.
#[derive(Debug, PartialEq)]
pub enum VideoPoll {
    Pending,
    Ready(String),
}

/// Picks the protocol from the provider endpoint first (an aggregator may resell any model under
/// the xAI shape), then from the model family.
pub fn resolve_video_api(provider: &ModelProvider, model: &str) -> Option<VideoApi> {
    let base = provider.base_url.to_ascii_lowercase();
    if base.contains("minimax") {
        return Some(VideoApi::Minimax);
    }
    if base.contains("volces.com") || base.contains("byteplus") {
        return Some(VideoApi::Ark);
    }
    if base.contains("dashscope") || base.contains("maas.aliyuncs.com") {
        return Some(VideoApi::DashScope);
    }
    if base.contains("api.x.ai") {
        return Some(VideoApi::Xai);
    }
    video_api_for_model(model)
}

/// Model families with a known video protocol, for endpoints that do not identify the vendor.
/// MiniMax v2 only accepts the H3 family; older Hailuo models use a different API.
pub fn video_api_for_model(model: &str) -> Option<VideoApi> {
    let name = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    if name.starts_with("grok") && name.contains("video") {
        Some(VideoApi::Xai)
    } else if name.starts_with("minimax-h3") {
        Some(VideoApi::Minimax)
    } else if name.contains("seedance") {
        Some(VideoApi::Ark)
    } else if name.starts_with("wan") && (name.contains("-t2v") || name.contains("-i2v")) {
        Some(VideoApi::DashScope)
    } else {
        None
    }
}

/// API root for the protocol. Presets store the root each vendor documents; strip the
/// OpenAI-compatible suffixes users commonly paste so the native paths line up.
fn api_root(api: VideoApi, base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let base = match api {
        // MiniMax native paths carry their own version (`/v2/...`).
        VideoApi::Minimax => base.strip_suffix("/v1").unwrap_or(base),
        // DashScope chat lives under `/compatible-mode/v1`; video lives under `/api/v1`.
        VideoApi::DashScope => {
            let root = base
                .strip_suffix("/compatible-mode/v1")
                .or_else(|| base.strip_suffix("/api/v1"))
                .unwrap_or(base);
            return format!("{root}/api/v1");
        }
        VideoApi::Ark | VideoApi::Xai => base,
    };
    base.to_string()
}

/// Endpoint, extra headers and JSON body that create the task.
pub fn create_request(
    api: VideoApi,
    base_url: &str,
    request: &VideoRequest<'_>,
) -> (String, Vec<(&'static str, &'static str)>, Value) {
    let root = api_root(api, base_url);
    match api {
        VideoApi::Xai => {
            let mut body = json!({"model": request.model, "prompt": request.prompt,
                "aspect_ratio": request.aspect_ratio, "duration": request.duration, "resolution": "720p"});
            if let Some(image) = &request.first_frame {
                body["image"] = json!({"url": image});
            }
            (format!("{root}/videos/generations"), vec![], body)
        }
        VideoApi::Minimax => {
            let mut content = vec![json!({"type": "text", "text": request.prompt})];
            if let Some(image) = &request.first_frame {
                content.push(json!({"type": "image_url", "image_url": {"url": image}, "role": "first_frame"}));
            }
            // Text-to-video requires a concrete ratio; with a first frame the image decides.
            let ratio = if request.first_frame.is_some() {
                "adaptive"
            } else {
                request.aspect_ratio
            };
            let body = json!({"model": request.model, "content": content, "ratio": ratio,
                "resolution": "768P", "duration": request.duration});
            (format!("{root}/v2/video_generation"), vec![], body)
        }
        VideoApi::Ark => {
            let mut content = vec![json!({"type": "text", "text": request.prompt})];
            if let Some(image) = &request.first_frame {
                content.push(json!({"type": "image_url", "image_url": {"url": image}, "role": "first_frame"}));
            }
            let ratio = if request.first_frame.is_some() {
                "adaptive"
            } else {
                request.aspect_ratio
            };
            let body = json!({"model": request.model, "content": content, "ratio": ratio,
                "resolution": "720p", "duration": request.duration, "watermark": false});
            (format!("{root}/contents/generations/tasks"), vec![], body)
        }
        VideoApi::DashScope => {
            let mut input = json!({"prompt": request.prompt});
            let mut parameters = json!({"resolution": "720P", "duration": request.duration,
                "prompt_extend": true, "watermark": false});
            match &request.first_frame {
                Some(image) if is_wan27(request.model) => {
                    input["media"] = json!([{"type": "first_frame", "url": image}]);
                }
                // Wan 2.6 and earlier take the first frame as a flat `img_url`.
                Some(image) => input["img_url"] = json!(image),
                // The aspect ratio follows the first frame, so `ratio` is text-to-video only.
                None => parameters["ratio"] = json!(request.aspect_ratio),
            }
            let body = json!({"model": request.model, "input": input, "parameters": parameters});
            (
                format!("{root}/services/aigc/video-generation/video-synthesis"),
                vec![("X-DashScope-Async", "enable")],
                body,
            )
        }
    }
}

fn is_wan27(model: &str) -> bool {
    model.to_ascii_lowercase().contains("wan2.7")
}

/// Task ID in the create response.
pub fn task_id(api: VideoApi, value: &Value) -> Option<String> {
    let pointer = match api {
        VideoApi::Xai => "/request_id",
        VideoApi::Minimax => "/task_id",
        VideoApi::Ark => "/id",
        VideoApi::DashScope => "/output/task_id",
    };
    value
        .pointer(pointer)
        .and_then(|v| {
            v.as_str()
                .map(str::to_string)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        })
        .filter(|id| !id.is_empty())
}

pub fn poll_url(api: VideoApi, base_url: &str, task_id: &str) -> Result<reqwest::Url, String> {
    let root = api_root(api, base_url);
    let collection = match api {
        VideoApi::Xai => format!("{root}/videos/"),
        VideoApi::Minimax => format!("{root}/v2/query/video_generation/"),
        VideoApi::Ark => format!("{root}/contents/generations/tasks/"),
        VideoApi::DashScope => format!("{root}/tasks/"),
    };
    let mut url = reqwest::Url::parse(&collection).map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "Invalid video URL")?
        .pop_if_empty()
        .push(task_id);
    Ok(url)
}

/// Interprets a poll response. Failures that arrive with HTTP 200 become errors here, so a
/// rejected task never looks like it is still running.
pub fn parse_poll(api: VideoApi, value: &Value) -> Result<VideoPoll, String> {
    let text = |pointer: &str| value.pointer(pointer).and_then(Value::as_str);
    let ready = |pointer: &str| {
        text(pointer)
            .filter(|url| !url.is_empty())
            .map(|url| VideoPoll::Ready(url.to_string()))
            .ok_or_else(|| "视频任务已完成但没有返回文件。".to_string())
    };
    let failed = |message: Option<&str>| {
        Err(message
            .filter(|m| !m.trim().is_empty())
            .unwrap_or("视频生成失败或已过期。请调整描述后重试。")
            .to_string())
    };
    match api {
        VideoApi::Xai => match text("/status") {
            Some("pending") => Ok(VideoPoll::Pending),
            Some("done") => {
                if value
                    .pointer("/video/respect_moderation")
                    .and_then(Value::as_bool)
                    == Some(false)
                {
                    return Err("视频未通过供应商内容审核。".into());
                }
                ready("/video/url")
            }
            Some("failed" | "expired") => failed(text("/error/message")),
            _ => Err("视频服务返回未知任务状态。".into()),
        },
        VideoApi::Minimax => match text("/task/status") {
            Some("queued" | "running") => Ok(VideoPoll::Pending),
            Some("succeeded") => ready("/task/content/url"),
            Some("failed" | "cancelled") => failed(text("/task/error/message")),
            _ => Err("视频服务返回未知任务状态。".into()),
        },
        VideoApi::Ark => match text("/status") {
            Some("queued" | "running") => Ok(VideoPoll::Pending),
            Some("succeeded") => ready("/content/video_url"),
            Some("failed" | "cancelled" | "expired") => failed(text("/error/message")),
            _ => Err("视频服务返回未知任务状态。".into()),
        },
        VideoApi::DashScope => match text("/output/task_status") {
            Some("PENDING" | "RUNNING") => Ok(VideoPoll::Pending),
            Some("SUCCEEDED") => ready("/output/video_url"),
            Some("FAILED" | "CANCELED") => failed(text("/output/message")),
            Some("UNKNOWN") => Err("任务不存在或已超过 24 小时查询期限。".into()),
            _ => Err("视频服务返回未知任务状态。".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(base_url: &str) -> ModelProvider {
        serde_json::from_value(json!({"id": "p", "name": "P", "baseUrl": base_url})).unwrap()
    }

    fn request(first_frame: Option<&str>) -> VideoRequest<'static> {
        VideoRequest {
            model: "m",
            prompt: "Ocean",
            aspect_ratio: "16:9",
            duration: 5,
            first_frame: first_frame.map(str::to_string),
        }
    }

    #[test]
    fn endpoint_decides_protocol_before_model_name() {
        let cases = [
            (
                "https://api.minimaxi.com/v1",
                "MiniMax-H3",
                VideoApi::Minimax,
            ),
            (
                "https://ark.cn-beijing.volces.com/api/v3",
                "doubao-seedance-2-5-260628",
                VideoApi::Ark,
            ),
            (
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "wan2.7-t2v",
                VideoApi::DashScope,
            ),
            ("https://api.x.ai/v1", "grok-imagine-video", VideoApi::Xai),
            // An aggregator falls back to the model family.
            (
                "https://relay.example/v1",
                "wan2.6-i2v",
                VideoApi::DashScope,
            ),
            (
                "https://relay.example/v1",
                "grok-imagine-video",
                VideoApi::Xai,
            ),
        ];
        for (base, model, api) in cases {
            assert_eq!(
                resolve_video_api(&provider(base), model),
                Some(api),
                "{base} {model}"
            );
        }
        assert_eq!(
            resolve_video_api(&provider("https://relay.example/v1"), "gpt-4o"),
            None
        );
    }

    #[test]
    fn create_requests_match_each_vendor_shape() {
        let (url, headers, body) = create_request(
            VideoApi::Minimax,
            "https://api.minimaxi.com/v1",
            &request(None),
        );
        assert_eq!(url, "https://api.minimaxi.com/v2/video_generation");
        assert!(headers.is_empty());
        assert_eq!(body["content"][0], json!({"type": "text", "text": "Ocean"}));
        assert_eq!(body["ratio"], "16:9");

        let (url, _, body) = create_request(
            VideoApi::Ark,
            "https://ark.cn-beijing.volces.com/api/v3",
            &request(Some("data:image/png;base64,AA")),
        );
        assert_eq!(
            url,
            "https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks"
        );
        assert_eq!(body["content"][1]["role"], "first_frame");
        assert_eq!(
            body["content"][1]["image_url"]["url"],
            "data:image/png;base64,AA"
        );
        assert_eq!(body["ratio"], "adaptive");

        let (url, headers, body) = create_request(
            VideoApi::DashScope,
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            &request(None),
        );
        assert_eq!(
            url,
            "https://dashscope.aliyuncs.com/api/v1/services/aigc/video-generation/video-synthesis"
        );
        assert_eq!(headers, vec![("X-DashScope-Async", "enable")]);
        assert_eq!(body["input"]["prompt"], "Ocean");
        assert_eq!(body["parameters"]["ratio"], "16:9");

        let mut wan27 = request(Some("data:image/png;base64,AA"));
        wan27.model = "wan2.7-i2v";
        let (_, _, body) = create_request(
            VideoApi::DashScope,
            "https://dashscope.aliyuncs.com/api/v1",
            &wan27,
        );
        assert_eq!(body["input"]["media"][0]["type"], "first_frame");
        assert!(body["parameters"].get("ratio").is_none());
        let mut wan26 = request(Some("data:image/png;base64,AA"));
        wan26.model = "wan2.6-i2v";
        let (_, _, body) = create_request(
            VideoApi::DashScope,
            "https://dashscope.aliyuncs.com",
            &wan26,
        );
        assert_eq!(body["input"]["img_url"], "data:image/png;base64,AA");
    }

    #[test]
    fn task_ids_and_poll_urls_follow_each_vendor() {
        assert_eq!(
            task_id(VideoApi::Minimax, &json!({"task_id": "42"})).as_deref(),
            Some("42")
        );
        assert_eq!(
            task_id(VideoApi::Ark, &json!({"id": "cgt-1"})).as_deref(),
            Some("cgt-1")
        );
        assert_eq!(
            task_id(VideoApi::DashScope, &json!({"output": {"task_id": "t1"}})).as_deref(),
            Some("t1")
        );
        assert_eq!(task_id(VideoApi::Xai, &json!({})), None);
        assert_eq!(
            poll_url(VideoApi::Minimax, "https://api.minimaxi.com/v1", "42")
                .unwrap()
                .as_str(),
            "https://api.minimaxi.com/v2/query/video_generation/42"
        );
        assert_eq!(
            poll_url(
                VideoApi::DashScope,
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "t1"
            )
            .unwrap()
            .as_str(),
            "https://dashscope.aliyuncs.com/api/v1/tasks/t1"
        );
    }

    #[test]
    fn poll_results_never_hide_failures() {
        let ready = |url: &str| Ok(VideoPoll::Ready(url.into()));
        assert_eq!(
            parse_poll(VideoApi::Minimax, &json!({"task": {"status": "running"}})),
            Ok(VideoPoll::Pending)
        );
        assert_eq!(
            parse_poll(
                VideoApi::Minimax,
                &json!({"task": {"status": "succeeded", "content": {"url": "https://v/1.mp4"}}})
            ),
            ready("https://v/1.mp4")
        );
        assert_eq!(
            parse_poll(
                VideoApi::Minimax,
                &json!({"task": {"status": "failed", "error": {"message": "sensitive"}}})
            ),
            Err("sensitive".into())
        );
        assert_eq!(
            parse_poll(
                VideoApi::Ark,
                &json!({"status": "succeeded", "content": {"video_url": "https://v/2.mp4"}})
            ),
            ready("https://v/2.mp4")
        );
        assert!(parse_poll(VideoApi::Ark, &json!({"status": "expired"})).is_err());
        assert_eq!(
            parse_poll(
                VideoApi::DashScope,
                &json!({"output": {"task_status": "SUCCEEDED", "video_url": "https://v/3.mp4"}})
            ),
            ready("https://v/3.mp4")
        );
        assert_eq!(
            parse_poll(
                VideoApi::DashScope,
                &json!({"output": {"task_status": "FAILED", "message": "bad"}})
            ),
            Err("bad".into())
        );
        assert!(parse_poll(
            VideoApi::DashScope,
            &json!({"output": {"task_status": "UNKNOWN"}})
        )
        .is_err());
        assert_eq!(
            parse_poll(
                VideoApi::Xai,
                &json!({"status": "done", "video": {"url": "https://v/4.mp4"}})
            ),
            ready("https://v/4.mp4")
        );
        for api in [
            VideoApi::Xai,
            VideoApi::Minimax,
            VideoApi::Ark,
            VideoApi::DashScope,
        ] {
            assert!(parse_poll(api, &json!({"status": "weird"})).is_err());
        }
    }
}
