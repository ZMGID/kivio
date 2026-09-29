//! Media creation owns its job lifetime and local files; pages only submit and observe.
use crate::{settings::ModelProvider, state::AppState};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tauri::{AppHandle, Manager, State};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MediaStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaRequest {
    pub kind: MediaKind,
    pub provider_id: String,
    pub model: String,
    pub prompt: String,
    pub aspect_ratio: String,
    pub duration: u32,
    pub reference_paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaOutput {
    pub name: String,
    pub mime_type: String,
    pub preview: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaJob {
    pub id: String,
    pub created_at: i64,
    pub request: MediaRequest,
    pub status: MediaStatus,
    pub error: Option<String>,
    pub outputs: Vec<MediaOutput>,
}

#[derive(Default)]
pub struct MediaStation(Mutex<MediaJobs>);
#[derive(Default)]
struct MediaJobs {
    loaded: bool,
    jobs: HashMap<String, MediaJob>,
    stops: HashMap<String, tokio::sync::oneshot::Sender<()>>,
}

fn root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("media-station"))
}

fn persist(root: &Path, job: &MediaJob) -> Result<(), String> {
    super::storage::atomic_write(
        &root.join(&job.id).join("job.json"),
        &serde_json::to_string(job).map_err(|e| e.to_string())?,
        "media job",
    )
}

impl MediaJobs {
    fn load(&mut self, root: &Path) -> Result<(), String> {
        if self.loaded {
            return Ok(());
        }
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let mut jobs = HashMap::new();
        for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path().join("job.json");
            if !path.exists() {
                continue;
            }
            let mut job: MediaJob =
                serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("Cannot read media history: {e}"))?;
            if !valid_id(&job.id) {
                return Err("Invalid media job ID".into());
            }
            if job.status == MediaStatus::Running {
                job.status = MediaStatus::Interrupted;
                job.error =
                    Some("应用退出中断了任务；供应商可能已生成并计费。请确认后再重试。".into());
                persist(root, &job)?;
            }
            jobs.insert(job.id.clone(), job);
        }
        self.jobs = jobs;
        self.loaded = true;
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}

fn validate(request: &MediaRequest) -> Result<(), String> {
    if request.prompt.trim().is_empty() || request.prompt.chars().count() > 8000 {
        return Err("请输入 1–8000 字的创作描述。".into());
    }
    if request.model.trim().is_empty() {
        return Err("请选择生成模型。".into());
    }
    if !["1:1", "16:9", "9:16", "4:3", "3:4"].contains(&request.aspect_ratio.as_str()) {
        return Err("Unsupported aspect ratio".into());
    }
    let limit = if request.kind == MediaKind::Video {
        1
    } else {
        4
    };
    if request.reference_paths.len() > limit {
        return Err(format!("最多选择 {limit} 张参考图。"));
    }
    if request.kind == MediaKind::Video && !(1..=15).contains(&request.duration) {
        return Err("视频时长需为 1–15 秒。".into());
    }
    Ok(())
}

#[tauri::command]
pub fn media_station_list(
    app: AppHandle,
    station: State<'_, MediaStation>,
) -> Result<Vec<MediaJob>, String> {
    let mut store = station.0.lock().map_err(|e| e.to_string())?;
    store.load(&root(&app)?)?;
    let mut jobs: Vec<_> = store.jobs.values().cloned().collect();
    jobs.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(jobs)
}

#[tauri::command]
pub fn media_station_start(
    app: AppHandle,
    state: State<'_, AppState>,
    station: State<'_, MediaStation>,
    request: MediaRequest,
) -> Result<MediaJob, String> {
    validate(&request)?;
    let provider = state
        .settings_read()
        .get_provider(&request.provider_id)
        .cloned()
        .filter(|p| p.enabled)
        .ok_or("供应商未启用或已删除。")?;
    if !provider.has_credentials() {
        return Err("请在设置中填写供应商 API Key。".into());
    }
    if request.kind == MediaKind::Video && provider.preferred_api_key().is_none() {
        return Err("视频生成需要 API Key，不支持 OAuth 登录。".into());
    }
    let dir = root(&app)?;
    let job = MediaJob {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now().timestamp_millis(),
        request,
        status: MediaStatus::Running,
        error: None,
        outputs: vec![],
    };
    let (stop, stopped) = tokio::sync::oneshot::channel();
    {
        let mut store = station.0.lock().map_err(|e| e.to_string())?;
        store.load(&dir)?;
        if store.stops.len() >= 3 {
            return Err("已有 3 个任务运行中，请等待完成。".into());
        }
        persist(&dir, &job)?;
        store.jobs.insert(job.id.clone(), job.clone());
        store.stops.insert(job.id.clone(), stop);
    }
    let running = job.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let result = tokio::select! {
            result = generate(&dir, &running, &provider, &state) => result,
            _ = stopped => return,
        };
        let station = app.state::<MediaStation>();
        let mut store = station.0.lock().unwrap_or_else(|e| e.into_inner());
        store.stops.remove(&running.id);
        if let Some(job) = store.jobs.get_mut(&running.id) {
            if job.status != MediaStatus::Running {
                return;
            }
            match result {
                Ok(outputs) => {
                    job.outputs = outputs;
                    job.status = MediaStatus::Completed;
                }
                Err(error) => {
                    job.error = Some(error);
                    job.status = MediaStatus::Failed;
                }
            }
            if let Err(error) = persist(&dir, job) {
                job.status = MediaStatus::Failed;
                job.error = Some(format!("保存任务失败：{error}"));
            }
        }
    });
    Ok(job)
}

#[tauri::command]
pub fn media_station_cancel(
    app: AppHandle,
    station: State<'_, MediaStation>,
    id: String,
) -> Result<(), String> {
    let mut store = station.0.lock().map_err(|e| e.to_string())?;
    let job = store.jobs.get(&id).ok_or("Task not found")?;
    if job.status != MediaStatus::Running {
        return Ok(());
    }
    let mut cancelled = job.clone();
    cancelled.status = MediaStatus::Cancelled;
    cancelled.error = Some("已停止本地等待；供应商任务可能继续并计费。".into());
    persist(&root(&app)?, &cancelled)?;
    store.jobs.insert(id.clone(), cancelled);
    if let Some(stop) = store.stops.remove(&id) {
        let _ = stop.send(());
    }
    Ok(())
}

fn output_path(
    app: &AppHandle,
    station: &MediaStation,
    id: &str,
    index: usize,
) -> Result<PathBuf, String> {
    let mut store = station.0.lock().map_err(|e| e.to_string())?;
    let dir = root(app)?;
    store.load(&dir)?;
    let output = store
        .jobs
        .get(id)
        .and_then(|j| j.outputs.get(index))
        .ok_or("Output not found")?;
    if !valid_id(id)
        || Path::new(&output.name).file_name().and_then(|s| s.to_str())
            != Some(output.name.as_str())
    {
        return Err("Invalid output path".into());
    }
    Ok(dir.join(id).join(&output.name))
}

#[tauri::command]
pub fn media_station_read(
    app: AppHandle,
    station: State<'_, MediaStation>,
    id: String,
    index: usize,
) -> Result<tauri::ipc::Response, String> {
    let path = output_path(&app, &station, &id, index)?;
    Ok(tauri::ipc::Response::new(
        fs::read(path).map_err(|e| e.to_string())?,
    ))
}

#[tauri::command]
pub fn media_station_export(
    app: AppHandle,
    station: State<'_, MediaStation>,
    id: String,
    index: usize,
    destination: String,
) -> Result<(), String> {
    let path = output_path(&app, &station, &id, index)?;
    fs::copy(path, destination).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn media_station_reference(
    app: AppHandle,
    station: State<'_, MediaStation>,
    id: String,
    index: usize,
) -> Result<String, String> {
    let path = output_path(&app, &station, &id, index)?;
    super::image_generation::load_input_images_from_paths(std::slice::from_ref(&path))?;
    Ok(path.to_string_lossy().into_owned())
}

async fn generate(
    root: &Path,
    job: &MediaJob,
    provider: &ModelProvider,
    state: &AppState,
) -> Result<Vec<MediaOutput>, String> {
    let request = &job.request;
    let paths: Vec<_> = request.reference_paths.iter().map(PathBuf::from).collect();
    let images = super::image_generation::load_input_images_from_paths(&paths)?;
    let dir = root.join(&job.id);
    if request.kind == MediaKind::Image {
        let result = super::image_generation::generate_image_with_provider(
            state,
            provider,
            &request.model,
            &json!({"prompt": request.prompt, "aspect_ratio": request.aspect_ratio, "n": 1}),
            &images,
            1,
            "Media station image generation",
        )
        .await?;
        if result.artifacts.is_empty() {
            return Err(result.content);
        }
        let mut outputs = Vec::new();
        for artifact in result.artifacts {
            let encoded = artifact
                .data_url
                .split_once(',')
                .ok_or("Invalid image response")?
                .1;
            let bytes = STANDARD.decode(encoded).map_err(|e| e.to_string())?;
            let name = artifact.name;
            fs::write(dir.join(&name), &bytes).map_err(|e| e.to_string())?;
            let mut preview = std::io::Cursor::new(Vec::new());
            image::load_from_memory(&bytes)
                .map_err(|e| e.to_string())?
                .thumbnail(384, 384)
                .write_to(&mut preview, image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            outputs.push(MediaOutput {
                name,
                mime_type: artifact.mime_type,
                preview: format!(
                    "data:image/png;base64,{}",
                    STANDARD.encode(preview.into_inner())
                ),
            });
        }
        return Ok(outputs);
    }
    let mut body = json!({"model": request.model, "prompt": request.prompt,
        "aspect_ratio": request.aspect_ratio, "duration": request.duration, "resolution": "720p"});
    if let Some(image) = images.first() {
        body["image"] = json!({"url": image.data_url()});
    }
    // Creation is deliberately not retried: an ambiguous timeout may already have incurred a charge.
    let base = provider.base_url.trim_end_matches('/');
    let key = provider.preferred_api_key().ok_or("API Key missing")?;
    let send = |builder: reqwest::RequestBuilder| {
        crate::provider_request::apply(builder.bearer_auth(key), provider, None)
            .timeout(Duration::from_secs(60))
    };
    let created = read_json(
        send(
            state
                .client_for(provider)
                .post(format!("{base}/videos/generations")),
        )
        .json(&body)
        .send()
        .await,
    )
    .await?;
    let id = created
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("视频服务未返回 request_id；请确认支持 xAI 视频接口。")?;
    let mut poll_url =
        reqwest::Url::parse(&format!("{base}/videos/")).map_err(|e| e.to_string())?;
    poll_url
        .path_segments_mut()
        .map_err(|_| "Invalid video URL")?
        .pop_if_empty()
        .push(id);
    tokio::time::timeout(Duration::from_secs(900), async {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let value = read_json(
                send(state.client_for(provider).get(poll_url.clone()))
                    .send()
                    .await,
            )
            .await?;
            match video_result(&value)? {
                None => continue,
                Some(url) => {
                    // Do not attach provider credentials to the returned CDN URL.
                    let url = reqwest::Url::parse(&url).map_err(|e| e.to_string())?;
                    if url.scheme() != "https" {
                        return Err("Video URL must use HTTPS".into());
                    }
                    let mut response = state
                        .client_for(provider)
                        .get(url)
                        .timeout(Duration::from_secs(180))
                        .send()
                        .await
                        .map_err(|e| e.to_string())?
                        .error_for_status()
                        .map_err(|e| e.to_string())?;
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
                        if bytes.len() + chunk.len() > 128 * 1024 * 1024 {
                            return Err("视频超过 128 MB 下载上限。".into());
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    if bytes.is_empty() {
                        return Err("视频内容为空。".into());
                    }
                    fs::write(dir.join("video.mp4"), bytes).map_err(|e| e.to_string())?;
                    return Ok(vec![MediaOutput {
                        name: "video.mp4".into(),
                        mime_type: "video/mp4".into(),
                        preview: String::new(),
                    }]);
                }
            }
        }
    })
    .await
    .map_err(|_| "等待视频超时；供应商可能仍在生成，请确认后再重试。".to_string())?
}

async fn read_json(response: Result<reqwest::Response, reqwest::Error>) -> Result<Value, String> {
    let response = response.map_err(|e| format!("媒体请求失败（未自动重试）：{e}"))?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|e| format!("Invalid media response: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "HTTP {status}: {}",
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Media request failed")
        ));
    }
    Ok(value)
}

fn video_result(value: &Value) -> Result<Option<String>, String> {
    match value.get("status").and_then(Value::as_str) {
        Some("pending") => Ok(None),
        Some("done") => {
            if value
                .pointer("/video/respect_moderation")
                .and_then(Value::as_bool)
                == Some(false)
            {
                return Err("视频未通过供应商内容审核。".into());
            }
            value
                .pointer("/video/url")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| Some(s.into()))
                .ok_or("视频任务已完成但没有返回文件。".into())
        }
        Some("failed" | "expired") => Err(value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("视频生成失败或已过期。请调整描述后重试。")
            .into()),
        _ => Err("视频服务返回未知任务状态。".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> MediaRequest {
        MediaRequest {
            kind: MediaKind::Video,
            provider_id: "p".into(),
            model: "video".into(),
            prompt: "Ocean".into(),
            aspect_ratio: "16:9".into(),
            duration: 5,
            reference_paths: vec![],
        }
    }
    #[test]
    fn validates_before_spending() {
        let mut r = request();
        assert!(validate(&r).is_ok());
        r.duration = 0;
        assert!(validate(&r).is_err());
        r.duration = 5;
        r.prompt = " ".into();
        assert!(validate(&r).is_err());
        r.prompt = "Ocean".into();
        r.reference_paths = vec!["a".into(), "b".into()];
        assert!(validate(&r).is_err());
    }
    #[test]
    fn video_terminal_states_do_not_hide_failures() {
        assert_eq!(video_result(&json!({"status":"pending"})).unwrap(), None);
        assert_eq!(
            video_result(&json!({"status":"done","video":{"url":"https://example.com/v.mp4"}}))
                .unwrap(),
            Some("https://example.com/v.mp4".into())
        );
        for value in [
            json!({"status":"failed"}),
            json!({"status":"expired"}),
            json!({"status":"done"}),
            json!({"status":"unknown"}),
            json!({"status":"done","video":{"url":"https://example.com","respect_moderation":false}}),
        ] {
            assert!(video_result(&value).is_err());
        }
    }
    #[test]
    fn restart_preserves_history_and_marks_inflight_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let job = MediaJob {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: 1,
            request: request(),
            status: MediaStatus::Running,
            error: None,
            outputs: vec![],
        };
        persist(dir.path(), &job).unwrap();
        let mut store = MediaJobs::default();
        store.load(dir.path()).unwrap();
        assert_eq!(store.jobs[&job.id].status, MediaStatus::Interrupted);
        let saved: MediaJob =
            serde_json::from_slice(&fs::read(dir.path().join(&job.id).join("job.json")).unwrap())
                .unwrap();
        assert_eq!(saved.status, MediaStatus::Interrupted);
        assert_eq!(saved.request.prompt, "Ocean");
    }

    #[tokio::test]
    async fn image_request_uses_existing_provider_and_saves_actual_output() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(16, 16)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let png = png.into_inner();
        let response = json!({"data": [{"b64_json": STANDARD.encode(&png)}]}).to_string();
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let serving = tokio::spawn(async move {
            let (mut socket, _) = server.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() < end + 4 + length {
                        continue;
                    }
                    assert!(header.starts_with("post /v1/images/generations "));
                    assert!(header.contains("authorization: bearer test-key"));
                    let body: Value = serde_json::from_slice(&bytes[end + 4..]).unwrap();
                    assert_eq!(body["prompt"], "Ocean");
                    assert_eq!(body["model"], "gpt-image-1");
                    break;
                }
            }
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).as_bytes()).await.unwrap();
        });
        let provider = ModelProvider {
            id: "test".into(),
            name: "Test".into(),
            api_keys: vec!["test-key".into()],
            api_key_legacy: None,
            base_url: format!("http://{address}/v1"),
            available_models: vec![],
            enabled_models: vec![],
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides: HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new_headless(
            crate::settings::Settings::default(),
            dir.path().join("usage"),
        );
        let mut request = request();
        request.kind = MediaKind::Image;
        request.model = "gpt-image-1".into();
        let job = MediaJob {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: 1,
            request,
            status: MediaStatus::Running,
            error: None,
            outputs: vec![],
        };
        persist(dir.path(), &job).unwrap();
        let outputs = tokio::time::timeout(
            Duration::from_secs(10),
            generate(dir.path(), &job, &provider, &state),
        )
        .await
        .unwrap()
        .unwrap();
        serving.await.unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(
            fs::read(dir.path().join(job.id).join(&outputs[0].name)).unwrap(),
            png
        );
        assert!(outputs[0].preview.starts_with("data:image/png;base64,"));
    }
}
