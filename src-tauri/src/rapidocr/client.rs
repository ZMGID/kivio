/// 前端拉状态用:就绪情况 + 模型目录(供 UI 显示)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RapidOcrStatus {
    pub standard_available: bool,
    pub high_available: bool,
    pub model_dir: Option<String>,
}

/// install() 返回值:成功/失败 + 人类可读 message。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RapidOcrInstallResult {
    pub success: bool,
    pub message: String,
}

/// AppState 持有的 RapidOCR 客户端。线程安全,Tauri command 间共享。
pub struct RapidOcrClient {
    models: Arc<OfflineModelManager>,
    /// A second concurrent worker would double the transient ~hundreds-of-MB model footprint.
    /// Serialize OCR operations while still keeping every worker one-shot.
    worker_lock: Mutex<()>,
    worker_executable: Option<PathBuf>,
}

impl RapidOcrClient {
    pub fn new(models: Arc<OfflineModelManager>) -> Arc<Self> {
        Arc::new(Self {
            models,
            worker_lock: Mutex::new(()),
            worker_executable: None,
        })
    }

    /// Native bundles keep the OCR runtime isolated from the voice runtime.
    pub fn with_worker(models: Arc<OfflineModelManager>, executable: PathBuf) -> Arc<Self> {
        Arc::new(Self { models, worker_lock: Mutex::new(()), worker_executable: Some(executable) })
    }

    /// 测试用:不持有 AppHandle 的占位实例。所有 OCR 操作立即 Err。
    #[cfg(test)]
    pub fn disabled() -> Arc<Self> {
        Self::new(OfflineModelManager::headless(reqwest::Client::new()))
    }

    /// 测试用:绕开 AppHandle,直接指定模型目录(供真实推理 E2E 测试落地固定缓存目录)。
    /// 所有其他行为(install/status/ocr_image)与生产路径完全一致,只是 model_dir() 来源不同。
    #[cfg(test)]
    pub fn with_model_dir(dir: PathBuf, http: reqwest::Client) -> Arc<Self> {
        Self::new(OfflineModelManager::with_model_dir(dir, http))
    }

    /// Headless (no-AppHandle) client for the `kivio-code` CLI. OCR is never
    /// invoked from the terminal agent; carries `app: None` so any OCR call
    /// errors out, identical to the test `disabled()` placeholder.
    pub fn headless(models: Arc<OfflineModelManager>) -> Arc<Self> {
        Self::new(models)
    }

    /// 模型 + dylib 落盘位置:`{app_data_dir}/rapidocr-models/`。
    fn model_dir(&self) -> Result<PathBuf, String> {
        self.models.model_dir()
    }

    /// 分档就绪情况。任一档必备文件不全 → 该档 available=false,前端渲染对应下载按钮。
    pub fn status(&self) -> RapidOcrStatus {
        let Ok(dir) = self.model_dir() else {
            return RapidOcrStatus {
                standard_available: false,
                high_available: false,
                model_dir: None,
            };
        };
        RapidOcrStatus {
            standard_available: self.models.rapidocr_ready(ModelTier::Standard),
            high_available: self.models.rapidocr_ready(ModelTier::High),
            model_dir: Some(dir.to_string_lossy().into_owned()),
        }
    }

    /// 按档位顺序下载必备文件:GET → 写 .tmp → rename。任一失败立刻返回 fail,不留半成品。
    /// dylib 等共享文件两档都会跳过已就绪检查,天然只下载一次。install_lock 防止双击并发。
    pub async fn install(&self, tier: ModelTier) -> RapidOcrInstallResult {
        let result = self.models.install_rapidocr(tier).await;
        RapidOcrInstallResult {
            success: result.success,
            message: result.message,
        }
    }

    /// 解析模型目录 + 校验该档位文件齐全。模型 pipeline 在一次性子进程中创建；子进程退出
    /// 后由操作系统回收整个 ONNX Runtime 地址空间，主进程不加载 OCR 模型。
    /// `ocr_image` / `ocr_image_lines` 共用:文件不齐 → `rapidocr_models_missing`;
    fn checked_model_dir(&self, tier: ModelTier) -> Result<PathBuf, String> {
        let dir = self.model_dir()?;
        if !self.models.rapidocr_ready(tier) {
            return Err("rapidocr_models_missing".into());
        }
        Ok(dir)
    }

    /// OCR 主入口。文件不齐 → `rapidocr_models_missing` 错误码,前端渲染下载提示。
    /// 每次调用启动一次性 worker；识别结束后 worker 退出并释放全部模型内存。
    pub async fn ocr_image(
        self: &Arc<Self>,
        image_path: &std::path::Path,
        tier: ModelTier,
    ) -> Result<String, String> {
        let _worker_slot = self.worker_lock.lock().await;
        let owner = self.clone();
        let model_dir = tokio::task::spawn_blocking(move || owner.checked_model_dir(tier)).await
            .map_err(|error| format!("OCR model validation failed: {error}"))??;
        match run_worker(self.worker_executable.as_deref(), model_dir, image_path, tier, WorkerMode::Text).await? {
            WorkerResponse::Text(text) => Ok(text),
            WorkerResponse::Error(error) => Err(error),
            WorkerResponse::Lines(_) => {
                Err("RapidOCR worker returned lines for text request".into())
            }
        }
    }

    /// 带 bbox 的行级 OCR，供替换翻译在原位绘制译文。与 `ocr_image` 共用同档位 pipeline。
    pub async fn ocr_image_lines(
        self: &Arc<Self>,
        image_path: &std::path::Path,
        tier: ModelTier,
    ) -> Result<Vec<RapidOcrLine>, String> {
        let _worker_slot = self.worker_lock.lock().await;
        let owner = self.clone();
        let model_dir = tokio::task::spawn_blocking(move || owner.checked_model_dir(tier)).await
            .map_err(|error| format!("OCR model validation failed: {error}"))??;
        match run_worker(self.worker_executable.as_deref(), model_dir, image_path, tier, WorkerMode::Lines).await? {
            WorkerResponse::Lines(lines) => Ok(lines),
            WorkerResponse::Error(error) => Err(error),
            WorkerResponse::Text(_) => {
                Err("RapidOCR worker returned text for lines request".into())
            }
        }
    }
}

#[cfg(not(test))]
async fn run_worker(
    executable: Option<&Path>,
    model_dir: PathBuf,
    image_path: &Path,
    tier: ModelTier,
    mode: WorkerMode,
) -> Result<WorkerResponse, String> {
    let executable = match executable {
        Some(path) => path.to_path_buf(),
        None => std::env::current_exe().map_err(|error| format!("resolve RapidOCR worker executable: {error}"))?,
    };
    let mut command = tokio::process::Command::new(executable);
    command
        .arg(RAPIDOCR_WORKER_ARG)
        .arg(model_dir)
        .arg(tier.as_str())
        .arg(mode.as_str())
        .arg(image_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.no_console_window();
    let output = command
        .output()
        .await
        .map_err(|error| format!("spawn RapidOCR worker: {error}"))?;
    let decoded = serde_json::from_slice::<WorkerResponse>(&output.stdout);
    if !output.status.success() {
        if let Ok(WorkerResponse::Error(error)) = &decoded {
            return Ok(WorkerResponse::Error(error.clone()));
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("RapidOCR worker exited with {}", output.status)
        } else {
            format!("RapidOCR worker failed: {stderr}")
        });
    }
    decoded.map_err(|error| format!("decode RapidOCR worker response: {error}"))
}
