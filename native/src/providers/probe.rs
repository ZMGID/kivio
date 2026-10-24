use super::*;
impl ProviderSettings {
    pub(super) fn probe(&mut self, catalog: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.ready {
            return;
        }
        self.capture_editor(window, cx);
        let Some(provider) = self
            .draft
            .iter()
            .find(|provider| Some(&provider.id) == self.editing.as_ref())
            .cloned()
        else {
            return;
        };
        let id = provider.id.clone();
        let root = self
            .path
            .parent()
            .and_then(Path::parent)
            .unwrap_or(&self.path)
            .to_path_buf();
        let runtime = match crate::chat_runtime::runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let work = runtime.spawn(async move {
            if catalog {
                kivio_model_runtime::native_provider_probe::models(&provider, &root)
                    .await
                    .map(Some)
            } else {
                kivio_model_runtime::native_provider_probe::connection(&provider, &root)
                    .await
                    .map(|_| None)
            }
        });
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work
                .await
                .map_err(|e| e.to_string())
                .and_then(|result| result);
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if this.editing.as_deref() != Some(id.as_str()) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(Some(models)) => {
                        let count = models.len();
                        if let Some(provider) =
                            this.draft.iter_mut().find(|provider| provider.id == id)
                        {
                            provider.available_models = models;
                            this.dirty = true;
                        }
                        window.push_notification(
                            Notification::success(crate::ui_format!("已读取 {count} 个模型，保存配置后生效。", "Loaded {count} models. Save the configuration to apply."
                            )),
                            cx,
                        );
                    }
                    Ok(None) => window.push_notification(Notification::success("连接测试通过"), cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
