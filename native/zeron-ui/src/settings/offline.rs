//! Retained offline package installer, using the original verified model manifest.
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt, button::Button, progress::Progress,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::offline_models::{
    OcrModelTier, OfflineModelManager, OfflineModelProgress, OfflineModelProgressState,
    ReplaceTranslationPackStatus,
};
use std::{path::PathBuf, sync::Arc};

struct Retained(Entity<OfflinePackages>);
impl Global for Retained {}
pub struct OfflinePackages {
    manager: Arc<OfflineModelManager>,
    status: Vec<ReplaceTranslationPackStatus>,
    progress: Option<OfflineModelProgress>,
    busy: bool,
    message: Option<Result<String, String>>,
    task: Option<Task<()>>,
    _events: Task<()>,
}
impl OfflinePackages {
    pub(crate) fn is_busy(&self) -> bool { self.busy }
    pub fn retained(root: PathBuf, cx: &mut App) -> Entity<Self> {
        if let Some(existing) = cx.try_global::<Retained>() {
            return existing.0.clone();
        }
        let entity = cx.new(|cx| Self::new(root, cx));
        cx.set_global(Retained(entity.clone()));
        entity
    }
    fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let (sender, receiver) = async_channel::unbounded();
        let manager = OfflineModelManager::with_progress(
            Ok(root.join("rapidocr-models")),
            kivio_model_runtime::transport::build_http_client(),
            Some(Arc::new(move |event| {
                let _ = sender.try_send(event);
            })),
        );
        let events = cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                if this
                    .update(cx, |this, cx| {
                        this.progress = Some(event);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut this = Self {
            manager,
            status: vec![],
            progress: None,
            busy: false,
            message: None,
            task: None,
            _events: events,
        };
        this.run(None, cx);
        this
    }
    fn run(&mut self, tier: Option<OcrModelTier>, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.progress = None;
        self.message = None;
        let manager = self.manager.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let (message, statuses) = cx
                .background_spawn(async move {
                    let message = tier.map(|tier| -> Result<String, String> {
                        let result = crate::chat_runtime::runtime()?
                            .block_on(manager.install_replace_translation(tier));
                        if result.success {
                            Ok(result.message)
                        } else {
                            Err(result.message)
                        }
                    });
                    let statuses = [OcrModelTier::Standard, OcrModelTier::High]
                        .into_iter()
                        .map(|tier| manager.replace_translation_status(tier))
                        .collect();
                    (message, statuses)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.status = statuses;
                this.message = message;
                cx.notify();
            });
        }));
        cx.notify();
    }
}
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.)
}
impl Render for OfflinePackages {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div()
            .v_flex()
            .gap_4()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(crate::i18n::text("离线 OCR 模型包"))
                    .child(
                        Button::new("refresh-ocr-models")
                            .small()
                            .label(crate::i18n::text("重新检测"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.run(None, cx))),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::text("截图 OCR 与替换翻译共用模型。下载会校验文件，失败后重试可继续下载。")),
            );
        for status in &self.status {
            let tier = OcrModelTier::parse(&status.tier);
            let label = if tier == OcrModelTier::High {
                "高精度 · PP-OCRv6"
            } else {
                "标准 · PP-OCRv5"
            };
            let mut details = div().v_flex().gap_2().child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(crate::i18n::display(label))
                    .child(
                        Button::new(SharedString::from(format!("install-ocr-{}", status.tier)))
                            .small()
                            .label(if status.ready { crate::i18n::text("已安装") } else { crate::i18n::text("下载 / 修复") })
                            .disabled(self.busy || status.ready)
                            .on_click(cx.listener(move |this, _, _, cx| this.run(Some(tier), cx))),
                    ),
            );
            details = details.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if status.ready {
                        "文件校验通过".into()
                    } else {
                        crate::ui_format!("还需下载 {}", "{} left to download", megabytes(status.missing_bytes))
                    }),
            );
            for file in &status.files {
                details = details.child(
                    div()
                        .flex()
                        .justify_between()
                        .gap_4()
                        .text_xs()
                        .child(file.file_name.clone())
                        .child(match file.state {
                            kivio_model_runtime::offline_models::OfflineModelFileState::Ready => {
                                "已校验"
                            }
                            kivio_model_runtime::offline_models::OfflineModelFileState::Missing => {
                                "未安装"
                            }
                            kivio_model_runtime::offline_models::OfflineModelFileState::Invalid => {
                                "文件损坏，需修复"
                            }
                        }),
                );
            }
            body = body.child(
                div()
                    .p_4()
                    .rounded(px(10.))
                    .bg(cx.theme().group_box)
                    .child(details),
            );
        }
        if self.busy {
            if let Some(event) = &self.progress {
                let label = match event.state {
                    OfflineModelProgressState::Downloading => "下载中",
                    OfflineModelProgressState::Retrying => "正在重试",
                    OfflineModelProgressState::Verifying => "校验中",
                    OfflineModelProgressState::Extracting => "解压中",
                    OfflineModelProgressState::Completed => "已完成",
                    OfflineModelProgressState::Failed => "下载失败",
                };
                let percent = if event.overall_total_bytes > 0 {
                    100. * event.overall_downloaded_bytes as f32 / event.overall_total_bytes as f32
                } else {
                    0.
                };
                body = body.child(
                    div()
                        .v_flex()
                        .gap_2()
                        .child(format!(
                            "{label} · {} · {} / {}",
                            event.file_name,
                            megabytes(event.overall_downloaded_bytes),
                            megabytes(event.overall_total_bytes)
                        ))
                        .child(Progress::new("ocr-download-progress").value(percent)),
                );
            } else {
                body = body.child(crate::i18n::text("正在检查模型文件…"));
            }
        }
        if let Some(message) = &self.message {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(if message.is_err() {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(match message {
                        Ok(message) | Err(message) => message.clone(),
                    }),
            );
        }
        body
    }
}
