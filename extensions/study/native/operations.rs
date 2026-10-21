//! Document exports and PDF translation jobs. Source PDFs and reading notes are
//! never rewritten. This module owns admission, process trees, outputs and recovery.
use super::{directory, document_dir, setup, ReadingDocument, ReadingNote};
use crate::extension_sdk::{model, AppState, NoConsoleWindow};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Mutex,
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const BRIDGE: &str = include_str!("translate.py");
static JOB: Mutex<Option<Job>> = Mutex::new(None);
static VIEWS: Mutex<()> = Mutex::new(());
struct Job {
    document_id: String,
    snapshot: Translation,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tauri::async_runtime::JoinHandle<()>>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Translation {
    run_id: String,
    phase: String,
    percent: f64,
    stage: String,
    model: String,
    target_lang: String,
    #[serde(default)]
    page: Option<u32>,
    error: Option<String>,
}
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum PdfKind {
    #[default]
    Source,
    Mono,
    Dual,
}
impl PdfKind {
    fn filename(self) -> &'static str {
        match self {
            Self::Source => "source.pdf",
            Self::Mono => "mono.pdf",
            Self::Dual => "dual.pdf",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationOutput {
    run_id: String,
    target_lang: String,
    page: Option<u32>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViewSelection {
    kind: PdfKind,
    latest_run_id: Option<String>,
    run_id: Option<String>,
}
#[derive(Serialize)]
pub struct ReaderView {
    kind: PdfKind,
    output: Option<TranslationOutput>,
    latest: Option<TranslationOutput>,
}
#[derive(Serialize)]
pub struct ReaderState {
    translation: Option<Translation>,
    view: ReaderView,
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    crate::extension_sdk::atomic_write(
        path,
        &serde_json::to_string(value).map_err(|e| e.to_string())?,
        "Study reading version",
    )
}
fn output_dir(dir: &Path, run_id: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(run_id).map_err(|_| "翻译版本无效")?;
    Ok(dir.join("translations").join(run_id))
}
fn output_metadata(dir: &Path, run_id: &str) -> Result<TranslationOutput, String> {
    let result: TranslationOutput = serde_json::from_slice(
        &std::fs::read(output_dir(dir, run_id)?.join("result.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if result.run_id != run_id {
        return Err("翻译版本记录不匹配".into());
    }
    Ok(result)
}
fn apply_translation(dir: &Path, translation: &Translation) -> Result<(), String> {
    let output = TranslationOutput {
        run_id: translation.run_id.clone(),
        target_lang: translation.target_lang.clone(),
        page: translation.page,
    };
    let folder = output_dir(dir, &output.run_id)?;
    certify_output(&folder, &folder.join("mono.pdf"))?;
    certify_output(&folder, &folder.join("dual.pdf"))?;
    write_json(&folder.join("result.json"), &output)?;
    write_json(
        &dir.join("reader-view.json"),
        &ViewSelection {
            kind: PdfKind::Mono,
            latest_run_id: Some(output.run_id.clone()),
            run_id: Some(output.run_id),
        },
    )
}
// Called with VIEWS held. Old successful translations are adopted into the reader.
fn load_selection(dir: &Path, translation: Option<&Translation>) -> Result<ViewSelection, String> {
    match std::fs::read(dir.join("reader-view.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(ready) = translation.filter(|t| t.phase == "ready") {
                apply_translation(dir, ready)?;
                return Ok(ViewSelection {
                    kind: PdfKind::Mono,
                    latest_run_id: Some(ready.run_id.clone()),
                    run_id: Some(ready.run_id.clone()),
                });
            }
            Ok(ViewSelection::default())
        }
        Err(e) => Err(e.to_string()),
    }
}
fn reader_state(dir: &Path, id: &str) -> Result<ReaderState, String> {
    let translation = snapshot(dir, id)?;
    let _lock = VIEWS.lock().map_err(|e| e.to_string())?;
    let selected = load_selection(dir, translation.as_ref())?;
    let latest = selected
        .latest_run_id
        .as_deref()
        .map(|run| output_metadata(dir, run))
        .transpose()?;
    let output = selected
        .run_id
        .as_deref()
        .map(|run| output_metadata(dir, run))
        .transpose()?;
    Ok(ReaderState {
        translation,
        view: ReaderView {
            kind: selected.kind,
            output,
            latest,
        },
    })
}
#[tauri::command]
pub async fn reading_view_select(
    app: AppHandle,
    id: String,
    kind: PdfKind,
    run_id: Option<String>,
) -> Result<ReaderState, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    let dir = document_dir(&directory(&app)?, &id)?;
    choose_view(&dir, &id, kind, run_id)
}
fn choose_view(
    dir: &Path,
    id: &str,
    kind: PdfKind,
    run_id: Option<String>,
) -> Result<ReaderState, String> {
    let translation = snapshot(dir, id)?;
    {
        let _lock = VIEWS.lock().map_err(|e| e.to_string())?;
        let mut selection = load_selection(dir, translation.as_ref())?;
        selection.kind = kind;
        selection.run_id = if kind == PdfKind::Source {
            None
        } else {
            let run = run_id
                .or_else(|| selection.latest_run_id.clone())
                .ok_or("这份文档还没有译文")?;
            output_metadata(dir, &run)?;
            pdf_path(dir, kind, Some(&run))?;
            Some(run)
        };
        write_json(&dir.join("reader-view.json"), &selection)?;
    }
    reader_state(dir, id)
}
pub(super) fn pdf_path(dir: &Path, kind: PdfKind, run_id: Option<&str>) -> Result<PathBuf, String> {
    let folder = if kind == PdfKind::Source {
        dir.to_path_buf()
    } else {
        output_dir(dir, run_id.ok_or("缺少翻译版本")?)?
    };
    let path = folder.join(kind.filename());
    certify_output(&folder, &path)?;
    Ok(path)
}
fn read_document(dir: &Path) -> Result<ReadingDocument, String> {
    serde_json::from_slice(&std::fs::read(dir.join("document.json")).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn save(dir: &Path, snapshot: &Translation) -> Result<(), String> {
    crate::extension_sdk::atomic_write(
        &dir.join("translation.json"),
        &serde_json::to_string(snapshot).map_err(|e| e.to_string())?,
        "Study translation",
    )
}
fn snapshot(dir: &Path, id: &str) -> Result<Option<Translation>, String> {
    let job = JOB.lock().map_err(|e| e.to_string())?;
    if let Some(job) = job.as_ref().filter(|j| j.document_id == id) {
        return Ok(Some(job.snapshot.clone()));
    }
    match std::fs::read(dir.join("translation.json")) {
        Ok(bytes) => {
            let mut value: Translation =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if value.phase == "running" {
                value.phase = "cancelled".into();
                value.error = Some("上次翻译被中断，可重新开始。".into());
                save(dir, &value)?;
            }
            Ok(Some(value))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingVersion {
    #[serde(flatten)]
    output: TranslationOutput,
    created_at: u64,
}
// Successful outputs are immutable children of a paper. Partial runs and
// unrelated folders never become attachments in the library.
fn versions(dir: &Path, id: &str) -> Result<Vec<ReadingVersion>, String> {
    let state = reader_state(dir, id)?; // Also adopts legacy successful runs.
    let _lock = VIEWS.lock().map_err(|e| e.to_string())?;
    let entries = match std::fs::read_dir(dir.join("translations")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let run = entry.file_name().to_string_lossy().into_owned();
        let Ok(output) = output_metadata(dir, &run) else {
            continue;
        };
        if pdf_path(dir, PdfKind::Mono, Some(&run)).is_err()
            || pdf_path(dir, PdfKind::Dual, Some(&run)).is_err()
        {
            continue;
        }
        let created_at = entry
            .path()
            .join("result.json")
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        result.push(ReadingVersion { output, created_at });
    }
    let latest = state.view.latest.map(|output| output.run_id);
    result.sort_by(|a, b| {
        (Some(&b.output.run_id) == latest.as_ref())
            .cmp(&(Some(&a.output.run_id) == latest.as_ref()))
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.output.run_id.cmp(&b.output.run_id))
    });
    Ok(result)
}
#[tauri::command]
pub async fn reading_versions(app: AppHandle, id: String) -> Result<Vec<ReadingVersion>, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || {
        versions(&document_dir(&directory(&app)?, &id)?, &id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn reading_translation_status(app: AppHandle, id: String) -> Result<ReaderState, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    reader_state(&document_dir(&directory(&app)?, &id)?, &id)
}

#[tauri::command]
pub async fn reading_translation_start(
    app: AppHandle,
    id: String,
    provider_id: String,
    model: String,
    source_lang: String,
    target_lang: String,
    page: Option<u32>,
) -> Result<ReaderState, String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    let study = setup::root(&app)?;
    if !setup::installed(&study) {
        return Err("请先在配置选项中安装 PDF2zh。".into());
    }
    if !["en", "zh", "ja", "de", "fr", "es", "ko"].contains(&source_lang.as_str())
        || !["en", "zh", "ja", "de", "fr", "es", "ko"].contains(&target_lang.as_str())
        || source_lang == target_lang
    {
        return Err("请选择不同的原文和译文语言。".into());
    }
    if model.trim().is_empty() || page == Some(0) {
        return Err("请选择模型和有效页码。".into());
    }
    let provider = app
        .state::<AppState>()
        .settings_read()
        .providers
        .iter()
        .find(|p| p.id == provider_id && p.enabled)
        .cloned()
        .ok_or("当前模型的供应商不可用")?;
    if !provider.authentication_ready() {
        return Err("请先登录当前模型供应商。".into());
    }
    let dir = document_dir(&directory(&app)?, &id)?;
    let doc = read_document(&dir)?;
    if !doc.has_pdf || doc.trashed_at.is_some() {
        return Err("原文 PDF 不可用。".into());
    }
    let mut current = JOB.lock().map_err(|e| e.to_string())?;
    if current
        .as_ref()
        .is_some_and(|j| j.snapshot.phase == "running")
    {
        return Err("已有文档正在翻译，请完成或停止后再开始。".into());
    }
    let state = Translation {
        run_id: uuid::Uuid::new_v4().to_string(),
        phase: "running".into(),
        percent: 0.0,
        stage: "正在准备文档".into(),
        model: model.clone(),
        target_lang: target_lang.clone(),
        page,
        error: None,
    };
    save(&dir, &state)?;
    let (cancel, mut cancelled) = tokio::sync::oneshot::channel();
    *current = Some(Job {
        document_id: id.clone(),
        snapshot: state.clone(),
        cancel: Some(cancel),
        task: None,
    });
    let result = state.clone();
    let view_dir = dir.clone();
    let task = tauri::async_runtime::spawn(async move {
        let output = dir.join("translations").join(&state.run_id);
        let request = json!({"input":dir.join("source.pdf"), "output":output, "sourceLang":source_lang,
            "targetLang":target_lang, "pages":page.map(|p|p.to_string()), "model":model, "providerId":provider_id});
        let outcome = tokio::select! {
            _ = &mut cancelled => Err("翻译已停止。".to_string()),
            value = tokio::time::timeout(Duration::from_secs(7200), translate(&app, &study, &output, &id, &provider, request)) =>
                value.unwrap_or_else(|_| Err("翻译超过两小时，已停止。请缩小页码范围后重试。".into())),
        };
        let mut current = JOB.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = current
            .as_mut()
            .filter(|j| j.snapshot.run_id == state.run_id)
        {
            job.cancel = None;
            match outcome {
                Ok(()) => {
                    job.snapshot.phase = "ready".into();
                    job.snapshot.percent = 100.0;
                    job.snapshot.stage = "译文与双语 PDF 已生成".into();
                    let _lock = VIEWS.lock().unwrap_or_else(|e| e.into_inner());
                    if let Err(error) = apply_translation(&dir, &job.snapshot) {
                        job.snapshot.phase = "error".into();
                        job.snapshot.error =
                            Some(format!("译文已生成，但未能切换阅读版本：{error}"));
                    }
                }
                Err(error) => {
                    job.snapshot.phase = if error == "翻译已停止。" {
                        "cancelled"
                    } else {
                        "error"
                    }
                    .into();
                    job.snapshot.error = Some(error);
                    let _ = std::fs::remove_dir_all(&output);
                }
            }
            if let Err(error) = save(&dir, &job.snapshot) {
                job.snapshot.phase = "error".into();
                job.snapshot.error = Some(format!("无法保存翻译结果：{error}"));
            }
        }
    });
    current.as_mut().unwrap().task = Some(task);
    drop(current);
    let _lock = VIEWS.lock().map_err(|e| e.to_string())?;
    let selected = load_selection(&view_dir, None)?;
    Ok(ReaderState {
        translation: Some(result),
        view: ReaderView {
            kind: selected.kind,
            output: selected
                .run_id
                .as_deref()
                .map(|run| output_metadata(&view_dir, run))
                .transpose()?,
            latest: selected
                .latest_run_id
                .as_deref()
                .map(|run| output_metadata(&view_dir, run))
                .transpose()?,
        },
    })
}

#[tauri::command]
pub async fn reading_translation_cancel(id: String) -> Result<(), String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    let mut job = JOB.lock().map_err(|e| e.to_string())?;
    if let Some(job) = job.as_mut().filter(|j| j.document_id == id) {
        if let Some(cancel) = job.cancel.take() {
            let _ = cancel.send(());
        }
    }
    Ok(())
}
pub async fn shutdown() {
    let task = {
        let mut job = JOB.lock().unwrap_or_else(|e| e.into_inner());
        job.as_mut().and_then(|j| {
            if let Some(cancel) = j.cancel.take() {
                let _ = cancel.send(());
            }
            j.task.take()
        })
    };
    if let Some(task) = task {
        let _ = task.await;
    }
}

fn command(executable: &Path, study: &Path) -> process_wrap::tokio::CommandWrap {
    let mut cmd = tokio::process::Command::new(executable);
    cmd.current_dir(study)
        .env("HOME", study.join("home"))
        .env("USERPROFILE", study.join("home"))
        .env("PYTHONNOUSERSITE", "1")
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .no_console_window();
    let mut wrapped = process_wrap::tokio::CommandWrap::from(cmd);
    #[cfg(unix)]
    wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    {
        let mut flags = process_wrap::tokio::CreationFlags(Default::default());
        flags.0 .0 = crate::extension_sdk::CREATE_NO_WINDOW;
        wrapped.wrap(flags).wrap(process_wrap::tokio::JobObject);
    }
    wrapped.wrap(process_wrap::tokio::KillOnDrop);
    wrapped
}
fn translation_options(
    provider: &crate::extension_sdk::ModelProvider,
    model: &str,
) -> model::GenerateOptions {
    let capabilities = crate::extension_sdk::thinking_capabilities_for_model(Some(provider), model);
    // Translation prefers the lightest supported effort. Some reasoning models
    // cannot turn thinking off; non-reasoning models have no effort field at all.
    model::GenerateOptions {
        thinking_enabled: true,
        thinking_level: capabilities.levels.first().cloned(),
        ..Default::default()
    }
}
async fn translate(
    app: &AppHandle,
    study: &Path,
    output: &Path,
    id: &str,
    provider: &crate::extension_sdk::ModelProvider,
    mut request: Value,
) -> Result<(), String> {
    std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let work = output.join("work");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let source = request["input"].as_str().ok_or("缺少原文路径")?;
    std::fs::copy(source, work.join("input.pdf")).map_err(|e| e.to_string())?;
    request["input"] = json!(work.join("input.pdf"));
    request["output"] = json!(work);
    let script = output.join("bridge.py");
    std::fs::write(&script, BRIDGE).map_err(|e| e.to_string())?;
    let mut cmd = command(&setup::python(study), study);
    cmd.command_mut().arg("-u").arg(&script);
    let mut child = cmd.spawn().map_err(|e| format!("无法启动 PDF2zh：{e}"))?;
    let mut input = child.stdin().take().ok_or("翻译输入管道不可用")?;
    let mut lines = BufReader::new(child.stdout().take().ok_or("翻译输出管道不可用")?).lines();
    input
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut finished = false;
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        if line.len() > 2 * 1024 * 1024 {
            return Err("翻译消息过长，已停止。".into());
        }
        let event: Value = serde_json::from_str(&line).map_err(|e| format!("翻译通信失败：{e}"))?;
        match event["type"].as_str() {
            Some("model") => {
                let messages = event["messages"]
                    .as_array()
                    .cloned()
                    .ok_or("翻译模型消息无效")?;
                let request = model::generate_request_from_openai_messages(
                    request["model"].as_str().unwrap_or_default(),
                    messages,
                    None,
                    translation_options(provider, request["model"].as_str().unwrap_or_default()),
                    "study_pdf_translation",
                    model::GenerateRequestContext::default(),
                );
                let state = app.state::<AppState>();
                let answer = tokio::time::timeout(
                    Duration::from_secs(180),
                    model::generate_with_chat_provider(&state, provider, 1, request),
                )
                .await
                .map_err(|_| "翻译模型请求超时，请重试。".to_string())?
                .map_err(|e| format!("翻译模型调用失败：{e}"))?;
                if answer.text.trim().is_empty() || answer.cancelled {
                    return Err("翻译模型未返回正文。".into());
                }
                input
                    .write_all(format!("{}\n", json!({"text":answer.text})).as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Some("progress") => {
                let mut job = JOB.lock().map_err(|e| e.to_string())?;
                if let Some(job) = job.as_mut().filter(|j| j.document_id == id) {
                    job.snapshot.percent =
                        event["percent"].as_f64().unwrap_or(0.0).clamp(0.0, 99.0);
                    job.snapshot.stage = event["stage"].as_str().unwrap_or_default().into();
                }
            }
            Some("error") => {
                return Err(format!(
                    "PDF2zh 翻译失败：{}",
                    event["error"].as_str().unwrap_or("未知错误")
                ))
            }
            Some("finish") => {
                for kind in ["mono", "dual"] {
                    let path = PathBuf::from(event[kind].as_str().ok_or("翻译缺少输出文件")?);
                    certify_output(output, &path)?;
                    std::fs::copy(&path, output.join(format!("{kind}.pdf")))
                        .map_err(|e| e.to_string())?;
                }
                finished = true;
            }
            _ => return Err("无法识别 PDF2zh 返回的消息。".into()),
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() || !finished {
        return Err("PDF2zh 提前退出，未生成完整译文。".into());
    }
    let _ = std::fs::remove_file(script);
    let _ = std::fs::remove_dir_all(work);
    Ok(())
}
fn certify_output(root: &Path, output: &Path) -> Result<(), String> {
    let canonical = output.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
        return Err("翻译输出超出文档目录。".into());
    }
    let mut file = std::fs::File::open(canonical).map_err(|e| e.to_string())?;
    let mut header = [0; 5];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header != b"%PDF-" {
        return Err("翻译结果不是有效的 PDF。".into());
    }
    Ok(())
}
fn notes_markdown(doc: &ReadingDocument, notes: &[ReadingNote], draft: &str) -> String {
    let title = doc.name.replace(['\n', '\r'], " ");
    let mut text = format!("# {title}\n\n");
    for note in notes {
        if let Some(anchor) = &note.anchor {
            text.push_str(&format!("## 第 {} 页\n\n", anchor.page));
        }
        text.push_str(&note.content);
        text.push_str("\n\n---\n\n");
    }
    if !draft.trim().is_empty() {
        text.push_str("## 未保存草稿\n\n");
        text.push_str(draft);
        text.push('\n');
    }
    text
}
#[tauri::command]
pub async fn reading_export(
    app: AppHandle,
    id: String,
    path: String,
    format: String,
    notes: Vec<ReadingNote>,
    draft: String,
    run_id: Option<String>,
) -> Result<(), String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    let root = directory(&app)?;
    let dir = document_dir(&root, &id)?;
    let doc = read_document(&dir)?;
    let destination = PathBuf::from(path);
    let extension = match format.as_str() {
        "source" | "mono" | "dual" => "pdf",
        "md" => "md",
        "docx" => "docx",
        "epub" => "epub",
        _ => return Err("不支持的导出格式".into()),
    };
    if destination
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .as_deref()
        != Some(extension)
    {
        return Err(format!("请选择 .{extension} 文件。"));
    }
    let parent = destination
        .parent()
        .ok_or("导出路径无效")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if parent.starts_with(&data) {
        return Err("请选择应用数据目录以外的导出位置。".into());
    }
    let study = setup::root(&app)?;
    // Stage on the destination volume, then atomically replace after full success.
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    if ["source", "mono", "dual"].contains(&format.as_str()) {
        let kind = match format.as_str() {
            "mono" => PdfKind::Mono,
            "dual" => PdfKind::Dual,
            _ => PdfKind::Source,
        };
        let (source, export_run) = if format == "source" {
            (dir.join("source.pdf"), None)
        } else {
            let latest = reader_state(&dir, &id)?.view.latest.ok_or("请先完成翻译")?;
            let run = run_id.as_deref().unwrap_or(&latest.run_id);
            output_metadata(&dir, run)?;
            (pdf_path(&dir, kind, Some(run))?, Some(run.to_string()))
        };
        certify_output(&dir, &source)?;
        staged = tauri::async_runtime::spawn_blocking(move || {
            super::annotations::write_pdf(
                &source,
                &mut staged,
                &notes,
                kind,
                export_run.as_deref(),
            )?;
            Ok::<_, String>(staged)
        })
        .await
        .map_err(|e| e.to_string())??;
    } else {
        if notes.is_empty() && draft.trim().is_empty() {
            return Err("还没有可导出的笔记。".into());
        }
        let content = notes_markdown(&doc, &notes, &draft);
        if content.len() > 8 * 1024 * 1024 {
            return Err("笔记超过 8 MB，请减少内容后导出。".into());
        }
        if format == "md" {
            staged
                .write_all(content.as_bytes())
                .map_err(|e| e.to_string())?;
        } else {
            if !setup::pandoc::installed(&study) {
                return Err("请先在配置选项中安装 Pandoc。".into());
            }
            let work = tempfile::tempdir().map_err(|e| e.to_string())?;
            let input = work.path().join("notes.md");
            let output = work.path().join(format!("notes.{format}"));
            std::fs::write(&input, content).map_err(|e| e.to_string())?;
            let mut cmd = command(&setup::pandoc::executable(&study), &study);
            cmd.command_mut()
                .args([
                    "--sandbox",
                    "--from=markdown-raw_html-raw_tex",
                    "--standalone",
                    "--metadata",
                ])
                .arg(format!("title={}", doc.name.replace(['\n', '\r'], " ")))
                .arg("--to")
                .arg(&format)
                .arg(&input)
                .arg("--output")
                .arg(&output);
            let mut child = cmd.spawn().map_err(|e| e.to_string())?;
            let status = tokio::time::timeout(Duration::from_secs(60), child.wait())
                .await
                .map_err(|_| "导出超时，已停止。")?
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("Pandoc 导出失败，请检查笔记中的格式或外部资源引用。".into());
            }
            std::io::copy(
                &mut std::fs::File::open(output).map_err(|e| e.to_string())?,
                &mut staged,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    staged.persist(destination).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successful_translation_becomes_the_reading_version_and_keeps_versioned_sources() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("source.pdf"), b"%PDF-original").unwrap();
        let ready = |language: &str, page| {
            let result = Translation {
                run_id: uuid::Uuid::new_v4().to_string(),
                phase: "ready".into(),
                percent: 100.0,
                stage: String::new(),
                model: "model".into(),
                target_lang: language.into(),
                page,
                error: None,
            };
            let output = output_dir(dir.path(), &result.run_id).unwrap();
            std::fs::create_dir_all(&output).unwrap();
            std::fs::write(
                output.join("mono.pdf"),
                format!("%PDF-translated-{language}"),
            )
            .unwrap();
            std::fs::write(output.join("dual.pdf"), b"%PDF-bilingual").unwrap();
            result
        };
        let first = ready("zh", None);
        save(dir.path(), &first).unwrap();
        // Adopt already-generated files too: no retranslation or export dialog.
        let reopened = reader_state(dir.path(), "paper").unwrap();
        assert_eq!(reopened.view.kind, PdfKind::Mono);
        assert_eq!(reopened.view.output.unwrap().target_lang, "zh");
        assert_eq!(
            choose_view(dir.path(), "paper", PdfKind::Source, None)
                .unwrap()
                .view
                .kind,
            PdfKind::Source
        );
        let second = ready("ja", Some(2));
        apply_translation(dir.path(), &second).unwrap();
        let reopened = reader_state(dir.path(), "paper").unwrap();
        assert_eq!(reopened.view.kind, PdfKind::Mono);
        assert_eq!(reopened.view.latest.unwrap().page, Some(2));
        assert_eq!(reopened.view.output.unwrap().target_lang, "ja");
        let partial = dir
            .path()
            .join("translations")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(partial.join("mono.pdf"), b"%PDF-incomplete").unwrap();
        let children = versions(dir.path(), "paper").unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].output.run_id, second.run_id);
        assert_eq!(children[1].output.run_id, first.run_id);
        // A saved annotation can still select its earlier immutable translation.
        let older = choose_view(
            dir.path(),
            "paper",
            PdfKind::Mono,
            Some(first.run_id.clone()),
        )
        .unwrap();
        assert_eq!(older.view.output.unwrap().run_id, first.run_id);
        assert_eq!(older.view.latest.unwrap().run_id, second.run_id);
        assert_eq!(
            std::fs::read(pdf_path(dir.path(), PdfKind::Source, None).unwrap()).unwrap(),
            b"%PDF-original"
        );
        assert!(choose_view(
            dir.path(),
            "paper",
            PdfKind::Dual,
            Some("../../source".into())
        )
        .is_err());
        let persisted = reader_state(dir.path(), "paper").unwrap();
        assert_eq!(persisted.view.kind, PdfKind::Mono);
        assert_eq!(persisted.view.output.unwrap().run_id, first.run_id);
    }
    #[test]
    fn translation_uses_a_supported_effort_for_always_on_models() {
        let provider: crate::extension_sdk::ModelProvider = serde_json::from_value(json!({
            "id":"test", "name":"test", "baseUrl":"https://example.com/v1", "apiFormat":"openai_responses"
        })).unwrap();
        let options = translation_options(&provider, "gpt-6.1-sol");
        assert!(
            options.thinking_enabled,
            "always-on models reject reasoning.effort=none"
        );
        assert_eq!(options.thinking_level.as_deref(), Some("low"));
        let ordinary = translation_options(&provider, "gpt-4o");
        assert!(ordinary.thinking_level.is_none());
    }
    #[test]
    fn interrupted_translation_recovers_without_certifying_partial_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let value = Translation {
            run_id: uuid::Uuid::new_v4().to_string(),
            phase: "running".into(),
            percent: 42.0,
            stage: "Translate Paragraphs".into(),
            model: "test".into(),
            target_lang: "zh".into(),
            page: None,
            error: None,
        };
        save(dir.path(), &value).unwrap();
        let reopened = snapshot(dir.path(), "not-active").unwrap().unwrap();
        assert_eq!(reopened.phase, "cancelled");
        assert!(reopened.error.unwrap().contains("中断"));
        let saved: Translation =
            serde_json::from_slice(&std::fs::read(dir.path().join("translation.json")).unwrap())
                .unwrap();
        assert_eq!(saved.phase, "cancelled");
        // An engine-reported path must be an actual PDF inside its own output directory.
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"%PDF-1.7").unwrap();
        assert!(certify_output(dir.path(), outside.path()).is_err());
        let partial = dir.path().join("partial.pdf");
        std::fs::write(&partial, b"not a pdf").unwrap();
        assert!(certify_output(dir.path(), &partial).is_err());
        std::fs::write(&partial, b"%PDF-1.7").unwrap();
        certify_output(dir.path(), &partial).unwrap();
    }
}
