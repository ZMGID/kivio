//! Media creation owns its job lifetime and local files; pages only submit and observe.
use super::video_generation::{self, VideoPoll};
use crate::{
    settings::{ModelProvider, ProviderApiFormat},
    state::AppState,
};
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
    /// Remote task ID of an asynchronous (video) generation, saved as soon as the provider
    /// accepts the paid request so an interrupted or abandoned wait can fetch the result later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub provider_task_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaDeleteFailure {
    pub id: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaDeleteResult {
    pub deleted_ids: Vec<String>,
    pub failures: Vec<MediaDeleteFailure>,
}

#[derive(Default)]
pub struct MediaStation(Mutex<MediaJobs>);
#[derive(Default)]
struct MediaJobs {
    loaded: bool,
    jobs: HashMap<String, MediaJob>,
    stops: HashMap<String, RunningTask>,
}

struct RunningTask {
    token: uuid::Uuid,
    stop: tokio::sync::oneshot::Sender<()>,
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
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let directory = entry.path();
            let path = directory.join("job.json");
            if !path.exists() {
                continue;
            }
            let mut job: MediaJob =
                serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("Cannot read media history: {e}"))?;
            if !valid_id(&job.id)
                || directory.file_name().and_then(|name| name.to_str()) != Some(job.id.as_str())
            {
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

    fn active(&self, id: &str, token: uuid::Uuid) -> bool {
        self.stops.get(id).is_some_and(|task| task.token == token)
            && self
                .jobs
                .get(id)
                .is_some_and(|job| job.status == MediaStatus::Running)
    }

    fn record_task(
        &mut self,
        root: &Path,
        id: &str,
        token: uuid::Uuid,
        task_id: &str,
    ) -> Result<(), String> {
        if !self.active(id, token) {
            return Ok(());
        }
        let job = self.jobs.get_mut(id).ok_or("Task not found")?;
        job.provider_task_id = Some(task_id.to_string());
        persist(root, job)
    }

    fn write_output(
        &self,
        root: &Path,
        id: &str,
        token: uuid::Uuid,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        if !self.active(id, token) {
            return Err("Task is no longer running".into());
        }
        if Path::new(name).file_name().and_then(|name| name.to_str()) != Some(name) {
            return Err("Invalid output path".into());
        }
        fs::write(root.join(id).join(name), bytes).map_err(|e| e.to_string())
    }

    fn finish(
        &mut self,
        root: &Path,
        id: &str,
        token: uuid::Uuid,
        result: Result<Vec<MediaOutput>, String>,
    ) {
        if !self.active(id, token) {
            return;
        }
        self.stops.remove(id);
        if let Some(job) = self.jobs.get_mut(id) {
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
            if let Err(error) = persist(root, job) {
                job.status = MediaStatus::Failed;
                job.error = Some(format!("保存任务失败：{error}"));
            }
        }
    }

    fn register(
        &mut self,
        root: &Path,
        job: &MediaJob,
        task: RunningTask,
        resume: bool,
    ) -> Result<MediaJob, String> {
        self.load(root)?;
        if resume {
            let current = self.jobs.get(&job.id).ok_or("Task not found")?;
            if current.status == MediaStatus::Running || current.status == MediaStatus::Completed {
                return Ok(current.clone());
            }
        }
        if self.stops.len() >= 3 {
            return Err("已有 3 个任务运行中，请等待完成。".into());
        }
        persist(root, job)?;
        self.jobs.insert(job.id.clone(), job.clone());
        self.stops.insert(job.id.clone(), task);
        Ok(job.clone())
    }

    fn delete(
        &mut self,
        ids: Vec<String>,
        mut remove: impl FnMut(&str) -> Result<(), String>,
    ) -> MediaDeleteResult {
        let mut result = MediaDeleteResult {
            deleted_ids: vec![],
            failures: vec![],
        };
        for id in ids {
            let outcome = if !valid_id(&id) {
                Err("Invalid media job ID".into())
            } else if self
                .jobs
                .get(&id)
                .is_some_and(|job| job.status == MediaStatus::Running)
            {
                Err("请先停止本地等待，再删除任务。".into())
            } else {
                remove(&id)
            };
            match outcome {
                Ok(()) => {
                    self.jobs.remove(&id);
                    self.stops.remove(&id);
                    result.deleted_ids.push(id);
                }
                Err(error) => result.failures.push(MediaDeleteFailure { id, error }),
            }
        }
        result
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
    let provider = media_provider(&state, &request)?;
    let job = MediaJob {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now().timestamp_millis(),
        request,
        status: MediaStatus::Running,
        error: None,
        outputs: vec![],
        provider_task_id: None,
    };
    run_job(app, &station, job, provider, false)
}

/// Re-attach to a video the provider already accepted (after an app restart, a stopped wait
/// or a lost poll). Only polls and downloads, so it never submits another paid request.
#[tauri::command]
pub fn media_station_resume(
    app: AppHandle,
    state: State<'_, AppState>,
    station: State<'_, MediaStation>,
    id: String,
) -> Result<MediaJob, String> {
    let mut job = {
        let mut store = station.0.lock().map_err(|e| e.to_string())?;
        store.load(&root(&app)?)?;
        store.jobs.get(&id).cloned().ok_or("Task not found")?
    };
    if job.status == MediaStatus::Running || job.status == MediaStatus::Completed {
        return Ok(job);
    }
    if job.provider_task_id.is_none() {
        return Err("该任务没有供应商任务编号，无法继续获取。".into());
    }
    let provider = media_provider(&state, &job.request)?;
    job.status = MediaStatus::Running;
    job.error = None;
    run_job(app, &station, job, provider, true)
}

/// Gate shared by start and resume; the page mirrors it when listing providers.
fn media_provider(state: &AppState, request: &MediaRequest) -> Result<ModelProvider, String> {
    let provider = state
        .settings_read()
        .get_provider(&request.provider_id)
        .cloned()
        .filter(|p| p.enabled)
        .ok_or("供应商未启用或已删除。")?;
    if provider.request.oauth.is_some() {
        return Err("媒体生成不支持账号 OAuth 登录。".into());
    }
    match (provider.api_format_kind(), &request.kind) {
        (ProviderApiFormat::AnthropicMessages, _)
        | (ProviderApiFormat::Gemini, MediaKind::Video) => {
            Err("该供应商的接口格式不支持此类媒体生成。".into())
        }
        (_, MediaKind::Video)
            if video_generation::resolve_video_api(&provider, &request.model).is_none() =>
        {
            Err("这个模型没有已知的视频接口；目前支持 Grok、MiniMax H3、Seedance 和万相。".into())
        }
        _ => Ok(provider),
    }
}

/// Registers the job as running, then generates in the background until it finishes or the
/// user stops waiting. The store is the only writer of job state; generation reports back here.
fn run_job(
    app: AppHandle,
    station: &MediaStation,
    job: MediaJob,
    provider: ModelProvider,
    resume: bool,
) -> Result<MediaJob, String> {
    let dir = root(&app)?;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let token = uuid::Uuid::new_v4();
    {
        let mut store = station.0.lock().map_err(|e| e.to_string())?;
        let registered = store.register(&dir, &job, RunningTask { token, stop }, resume)?;
        if !store.active(&job.id, token) {
            return Ok(registered);
        }
    }
    let running = job.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let record_task = |task_id: &str| {
            let station = app.state::<MediaStation>();
            let mut store = station.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(error) = store.record_task(&dir, &running.id, token, task_id) {
                eprintln!("[media_station] failed to save provider task id: {error}");
            }
        };
        let write_output = |name: &str, bytes: &[u8]| {
            let station = app.state::<MediaStation>();
            let store = station.0.lock().unwrap_or_else(|e| e.into_inner());
            // Hold the lifetime lock through the write: cancellation/deletion must
            // not race a synchronous write following the final network await.
            store.write_output(&dir, &running.id, token, name, bytes)
        };
        let result = tokio::select! {
            result = generate(&running, &provider, &state, record_task, write_output) => result,
            _ = stopped => return,
        };
        let station = app.state::<MediaStation>();
        let mut store = station.0.lock().unwrap_or_else(|e| e.into_inner());
        store.finish(&dir, &running.id, token, result);
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
    if let Some(task) = store.stops.remove(&id) {
        let _ = task.stop.send(());
    }
    Ok(())
}

#[tauri::command]
pub fn media_station_delete(
    app: AppHandle,
    station: State<'_, MediaStation>,
    ids: Vec<String>,
) -> Result<MediaDeleteResult, String> {
    let dir = root(&app)?;
    let mut store = station.0.lock().map_err(|e| e.to_string())?;
    store.load(&dir)?;
    Ok(store.delete(ids, |id| super::artifacts::delete_media_job(&app, &dir, id)))
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

/// Consecutive transient poll failures tolerated before giving up on a video wait.
const POLL_RETRY_LIMIT: u32 = 6;

async fn generate(
    job: &MediaJob,
    provider: &ModelProvider,
    state: &AppState,
    record_task: impl FnOnce(&str),
    write_output: impl Fn(&str, &[u8]) -> Result<(), String>,
) -> Result<Vec<MediaOutput>, String> {
    let request = &job.request;
    if request.kind == MediaKind::Image {
        let paths: Vec<_> = request.reference_paths.iter().map(PathBuf::from).collect();
        let images = super::image_generation::load_input_images_from_paths(&paths)?;
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
            write_output(&name, &bytes)?;
            // The image is paid for and saved; a thumbnail failure only costs the preview.
            let preview = image_preview(&bytes).unwrap_or_else(|error| {
                eprintln!("[media_station] preview failed for {name}: {error}");
                String::new()
            });
            outputs.push(MediaOutput {
                name,
                mime_type: artifact.mime_type,
                preview,
            });
        }
        return Ok(outputs);
    }
    // Asynchronous video API: create once, then poll the task until it ends.
    let api = video_generation::resolve_video_api(provider, &request.model)
        .ok_or("这个模型没有已知的视频接口；目前支持 Grok、MiniMax H3、Seedance 和万相。")?;
    let key = provider.preferred_api_key().unwrap_or_default();
    let send = |builder: reqwest::RequestBuilder| {
        crate::provider_request::apply(
            crate::provider_request::apply_api_key_auth(
                builder,
                ProviderApiFormat::OpenAiChat,
                key,
            ),
            provider,
            None,
        )
            .timeout(Duration::from_secs(60))
    };
    let task_id = match &job.provider_task_id {
        Some(id) => id.clone(),
        None => {
            let paths: Vec<_> = request.reference_paths.iter().map(PathBuf::from).collect();
            let images = super::image_generation::load_input_images_from_paths(&paths)?;
            let (url, headers, body) = video_generation::create_request(
                api,
                &provider.base_url,
                &video_generation::VideoRequest {
                    model: &request.model,
                    prompt: &request.prompt,
                    aspect_ratio: &request.aspect_ratio,
                    duration: request.duration,
                    first_frame: images.first().map(|image| image.data_url()),
                },
            );
            let mut builder = state.client_for(provider).post(url);
            for (name, value) in headers {
                builder = builder.header(name, value);
            }
            // Creation is deliberately not retried: an ambiguous timeout may already have incurred a charge.
            let created = read_json(send(builder).json(&body).send().await)
                .await
                .map_err(|e| e.message)?;
            let id = video_generation::task_id(api, &created)
                .ok_or("视频服务没有返回任务编号；请确认供应商地址与模型匹配。")?;
            record_task(&id);
            id
        }
    };
    let poll_url = video_generation::poll_url(api, &provider.base_url, &task_id)?;
    tokio::time::timeout(Duration::from_secs(900), async {
        let mut failures = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            // Polling is a read, so transient failures are retried; the task keeps running remotely.
            let value = match read_json(
                send(state.client_for(provider).get(poll_url.clone()))
                    .send()
                    .await,
            )
            .await
            {
                Ok(value) => {
                    failures = 0;
                    value
                }
                Err(error) if error.transient && failures < POLL_RETRY_LIMIT => {
                    failures += 1;
                    eprintln!(
                        "[media_station] video poll failed ({failures}): {}",
                        error.message
                    );
                    continue;
                }
                Err(error) => return Err(format!("{}（可稍后继续获取结果）", error.message)),
            };
            match video_generation::parse_poll(api, &value)? {
                VideoPoll::Pending => continue,
                VideoPoll::Ready(url) => {
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
                    write_output("video.mp4", &bytes)?;
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
    .map_err(|_| "等待视频超时；供应商可能仍在生成，可稍后继续获取结果。".to_string())?
}

fn image_preview(bytes: &[u8]) -> Result<String, String> {
    let mut preview = std::io::Cursor::new(Vec::new());
    image::load_from_memory(bytes)
        .map_err(|e| e.to_string())?
        .thumbnail(384, 384)
        .write_to(&mut preview, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(preview.into_inner())
    ))
}

struct MediaHttpError {
    message: String,
    /// Network failures, 408/429 and 5xx: safe to repeat for reads, never for creation.
    transient: bool,
}

async fn read_json(
    response: Result<reqwest::Response, reqwest::Error>,
) -> Result<Value, MediaHttpError> {
    let response = response.map_err(|e| MediaHttpError {
        message: format!("媒体请求失败：{e}"),
        transient: true,
    })?;
    let status = response.status();
    let text = response.text().await.map_err(|e| MediaHttpError {
        message: format!("读取媒体响应失败：{e}"),
        transient: true,
    })?;
    http_json(status, &text)
}

fn http_json(status: reqwest::StatusCode, text: &str) -> Result<Value, MediaHttpError> {
    let value = serde_json::from_str::<Value>(text);
    if status.is_success() {
        return value.map_err(|e| MediaHttpError {
            message: format!("Invalid media response: {e}"),
            transient: false,
        });
    }
    // Gateways often answer errors with HTML or plain text; keep the status and a short excerpt.
    let detail = value
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("error"))
                .or_else(|| v.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| text.trim().chars().take(200).collect());
    Err(MediaHttpError {
        message: format!("HTTP {status}: {detail}"),
        transient: status.is_server_error()
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status == reqwest::StatusCode::REQUEST_TIMEOUT,
    })
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
    fn restart_preserves_history_and_marks_inflight_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let job = MediaJob {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: 1,
            request: request(),
            status: MediaStatus::Running,
            error: None,
            outputs: vec![],
            provider_task_id: Some("remote-1".into()),
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
        assert_eq!(saved.provider_task_id.as_deref(), Some("remote-1"));
    }

    #[test]
    fn http_errors_keep_status_and_classify_retryable_failures() {
        use reqwest::StatusCode;
        let gateway = http_json(StatusCode::BAD_GATEWAY, "<html>Bad Gateway</html>")
            .err()
            .unwrap();
        assert!(gateway.transient);
        assert!(gateway.message.contains("502") && gateway.message.contains("Bad Gateway"));
        let limited = http_json(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"slow down"}}"#,
        )
        .err()
        .unwrap();
        assert!(limited.transient);
        assert!(limited.message.ends_with("slow down"));
        let denied = http_json(StatusCode::UNAUTHORIZED, r#"{"error":"bad key"}"#)
            .err()
            .unwrap();
        assert!(!denied.transient);
        assert!(denied.message.ends_with("bad key"));
        assert!(http_json(StatusCode::OK, r#"{"status":"pending"}"#).is_ok());
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
            provider_task_id: None,
        };
        persist(dir.path(), &job).unwrap();
        let outputs = tokio::time::timeout(
            Duration::from_secs(10),
            generate(
                &job,
                &provider,
                &state,
                |_| {},
                |name, bytes| {
                    fs::write(dir.path().join(&job.id).join(name), bytes).map_err(|e| e.to_string())
                },
            ),
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

    fn job(status: MediaStatus) -> MediaJob {
        MediaJob {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: 1,
            request: request(),
            status,
            error: None,
            outputs: vec![],
            provider_task_id: Some("remote-1".into()),
        }
    }

    fn task(token: uuid::Uuid) -> RunningTask {
        let (stop, _) = tokio::sync::oneshot::channel();
        RunningTask { token, stop }
    }

    #[test]
    fn batch_delete_preserves_failed_items_and_rejects_running_and_invalid_ids() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = tempfile::tempdir().unwrap();
        let completed = job(MediaStatus::Completed);
        let running = job(MediaStatus::Running);
        let failed = job(MediaStatus::Failed);
        for job in [&completed, &running, &failed] {
            persist(dir.path(), job).unwrap();
        }
        let mut store = MediaJobs {
            loaded: true,
            ..Default::default()
        };
        for job in [&completed, &running, &failed] {
            store.jobs.insert(job.id.clone(), job.clone());
        }
        let missing = uuid::Uuid::new_v4().to_string();
        let result = store.delete(
            vec![
                completed.id.clone(),
                running.id.clone(),
                "../escape".into(),
                failed.id.clone(),
                missing.clone(),
            ],
            |id| {
                if id == failed.id {
                    return Err("filesystem failure".into());
                }
                super::super::artifacts::delete_media_job_in(artifacts.path(), dir.path(), id)
            },
        );
        assert_eq!(
            result.deleted_ids,
            vec![completed.id.clone(), missing.clone()]
        );
        assert_eq!(
            result
                .failures
                .iter()
                .map(|failure| failure.id.as_str())
                .collect::<Vec<_>>(),
            vec![running.id.as_str(), "../escape", failed.id.as_str()]
        );
        assert!(store.jobs.contains_key(&failed.id));
        assert!(store.jobs.contains_key(&running.id));
        assert!(!dir.path().join(&completed.id).exists());
        assert!(dir.path().join(&failed.id).join("job.json").exists());
        let again = store.delete(vec![completed.id.clone(), missing], |id| {
            super::super::artifacts::delete_media_job_in(artifacts.path(), dir.path(), id)
        });
        assert!(again.failures.is_empty());
        let mut restarted = MediaJobs::default();
        restarted.load(dir.path()).unwrap();
        assert!(!restarted.jobs.contains_key(&completed.id));
    }

    #[test]
    fn deleted_jobs_reject_stale_resume_and_all_late_writes() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = tempfile::tempdir().unwrap();
        let mut running = job(MediaStatus::Running);
        let old = uuid::Uuid::new_v4();
        let mut store = MediaJobs::default();
        store
            .register(dir.path(), &running, task(old), false)
            .unwrap();
        store.jobs.get_mut(&running.id).unwrap().status = MediaStatus::Cancelled;
        store.stops.remove(&running.id);
        let result = store.delete(vec![running.id.clone()], |id| {
            super::super::artifacts::delete_media_job_in(artifacts.path(), dir.path(), id)
        });
        assert!(result.failures.is_empty());
        running.status = MediaStatus::Running;
        assert!(store
            .register(dir.path(), &running, task(uuid::Uuid::new_v4()), true)
            .is_err());
        store
            .record_task(dir.path(), &running.id, old, "late")
            .unwrap();
        assert!(store
            .write_output(dir.path(), &running.id, old, "video.mp4", b"late")
            .is_err());
        store.finish(dir.path(), &running.id, old, Ok(vec![]));
        assert!(!dir.path().join(&running.id).exists());
        assert!(!store.jobs.contains_key(&running.id));
    }

    #[test]
    fn old_run_cannot_modify_or_stop_a_resumed_job() {
        let dir = tempfile::tempdir().unwrap();
        let running = job(MediaStatus::Running);
        let old = uuid::Uuid::new_v4();
        let current = uuid::Uuid::new_v4();
        let mut store = MediaJobs::default();
        store
            .register(dir.path(), &running, task(old), false)
            .unwrap();
        store.jobs.get_mut(&running.id).unwrap().status = MediaStatus::Cancelled;
        store.stops.remove(&running.id);
        store
            .register(dir.path(), &running, task(current), true)
            .unwrap();
        store
            .record_task(dir.path(), &running.id, old, "stale")
            .unwrap();
        store.finish(dir.path(), &running.id, old, Err("stale".into()));
        assert!(store
            .write_output(dir.path(), &running.id, old, "video.mp4", b"stale")
            .is_err());
        assert!(store.active(&running.id, current));
        assert_eq!(
            store.jobs[&running.id].provider_task_id.as_deref(),
            Some("remote-1")
        );
        let duplicate = uuid::Uuid::new_v4();
        store
            .register(dir.path(), &running, task(duplicate), true)
            .unwrap();
        assert!(store.active(&running.id, current));
        assert!(!store.active(&running.id, duplicate));
        store
            .write_output(dir.path(), &running.id, current, "video.mp4", b"current")
            .unwrap();
        store.finish(dir.path(), &running.id, current, Ok(vec![]));
        assert_eq!(store.jobs[&running.id].status, MediaStatus::Completed);
    }
}
