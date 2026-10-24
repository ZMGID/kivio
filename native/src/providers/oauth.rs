//! Native lifecycle for the shared OAuth owner. Secrets stay in the credential
//! store; only login metadata and a durably saved provider reference enter GPUI.
use super::{ProviderDocument, ProviderSettings};
use crate::provider_settings::{ModelProvider, OAuthConfig};
use gpui_kit::component::{
    ActiveTheme, Disableable, StyledExt,
    button::{Button, ButtonVariants},
};
use gpui_kit::{prelude::FluentBuilder, *};
use kivio_model_runtime::provider_oauth::{self as oauth, Login};
use std::{path::PathBuf, time::Duration};

// Dropping the sender also cancels before the start response arrives.
pub(super) struct OAuthRun {
    _cancel: tokio::sync::oneshot::Sender<()>,
}

enum Update {
    Started(Login),
    Saved(Result<ProviderDocument, String>),
    Cancelled,
}

fn cancelled(cancel: &mut tokio::sync::oneshot::Receiver<()>) -> bool {
    !matches!(
        cancel.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    )
}

async fn login(
    mut document: ProviderDocument,
    path: PathBuf,
    provider: ModelProvider,
    mut cancel: tokio::sync::oneshot::Receiver<()>,
    events: async_channel::Sender<Update>,
) {
    let Some(kind) = provider
        .request
        .oauth
        .as_ref()
        .map(|auth| auth.provider.clone())
    else {
        return;
    };
    let login = match oauth::provider_oauth_start(kind, provider.request.use_system_proxy).await {
        Ok(login) => login,
        Err(error) => {
            let _ = events
                .send(if cancelled(&mut cancel) {
                    Update::Cancelled
                } else {
                    Update::Saved(Err(error))
                })
                .await;
            return;
        }
    };
    let id = login.login_id.clone();
    let mut interval = login.interval;
    if cancelled(&mut cancel) || events.send(Update::Started(login)).await.is_err() {
        oauth::provider_oauth_cancel(id).await;
        let _ = events.send(Update::Cancelled).await;
        return;
    }
    loop {
        tokio::select! {
            _ = &mut cancel => {
                oauth::provider_oauth_cancel(id).await;
                let _ = events.send(Update::Cancelled).await;
                return;
            }
            _ = tokio::time::sleep(Duration::from_secs(interval.max(1) as u64)) => {}
        }
        // Await the bounded request so cancellation cannot abandon a credential
        // that the shared poll owner has just durably created.
        let result = oauth::provider_oauth_poll(id.clone()).await;
        if cancelled(&mut cancel) || events.is_closed() {
            oauth::provider_oauth_cancel(id.clone()).await;
            if let Ok(result) = result {
                if let Some(auth) = result.auth {
                    if let Some(id) = auth.credential_id {
                        let _ = oauth::provider_oauth_disconnect(id).await;
                    }
                }
            }
            let _ = events.send(Update::Cancelled).await;
            return;
        }
        match result {
            Ok(result) if result.status == "pending" => interval = result.interval,
            Ok(result) => {
                let Some(auth) = result.auth else {
                    oauth::provider_oauth_cancel(id).await;
                    let _ = events
                        .send(Update::Saved(
                            Err("授权未返回账号引用，请重新登录。".into()),
                        ))
                        .await;
                    return;
                };
                let credential = auth.credential_id.clone();
                let Some(target) = document
                    .providers
                    .iter_mut()
                    .find(|value| value.id == provider.id)
                else {
                    return;
                };
                target.request.oauth = Some(auth);
                let saved = tokio::task::spawn_blocking(move || document.save(&path))
                    .await
                    .map_err(|_| "保存授权配置的任务已中断。".to_string())
                    .and_then(|result| result);
                if saved.is_err() {
                    if let Some(id) = credential {
                        let _ = oauth::provider_oauth_disconnect(id).await;
                    }
                }
                let _ = events.send(Update::Saved(saved)).await;
                return;
            }
            Err(error) => {
                oauth::provider_oauth_cancel(id).await;
                let _ = events.send(Update::Saved(Err(error))).await;
                return;
            }
        }
    }
}

impl ProviderSettings {
    pub(crate) fn cancel_pending_login(&mut self, cx: &mut Context<Self>) {
        self.oauth_run = None;
        self.oauth_login = None;
        cx.notify();
    }
    pub(super) fn change_auth(
        &mut self,
        kind: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_editor(window, cx);
        let Some(provider) = self
            .draft
            .iter_mut()
            .find(|value| Some(&value.id) == self.editing.as_ref())
        else {
            return;
        };
        if provider
            .request
            .oauth
            .as_ref()
            .map(|auth| auth.provider.as_str())
            == kind
        {
            return;
        }
        provider.request.oauth = kind.map(|kind| OAuthConfig {
            provider: kind.into(),
            credential_id: None,
            project_id: None,
        });
        match kind {
            Some("codex") => {
                provider.base_url = oauth::CODEX_BASE.into();
                provider.api_format = "openai_responses".into();
            }
            Some("kimi") => {
                provider.base_url = oauth::KIMI_BASE.into();
                provider.api_format = "openai_chat".into();
            }
            Some("antigravity") => {
                provider.base_url = oauth::antigravity::BASE.into();
                provider.api_format = "gemini".into();
            }
            _ => {}
        }
        self.open_editor(self.editing.clone(), window, cx);
        self.dirty = true;
        self.error = None;
        cx.notify();
    }

    pub(super) fn sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.dirty {
            return;
        }
        let Some(provider) = self
            .confirmed
            .providers
            .iter()
            .find(|value| Some(&value.id) == self.editing.as_ref())
            .cloned()
        else {
            return;
        };
        let runtime = match crate::chat_runtime::runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let (events, receive) = async_channel::unbounded();
        runtime.spawn(login(
            self.confirmed.clone(),
            self.path.clone(),
            provider,
            cancelled,
            events,
        ));
        self.oauth_run = Some(OAuthRun { _cancel: cancel });
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn_in(window, async move |owner, cx| {
            while let Ok(event) = receive.recv().await {
                let finished = !matches!(event, Update::Started(_));
                if owner
                    .update_in(cx, |this, window, cx| {
                        match event {
                            Update::Started(login) => this.oauth_login = Some(login),
                            Update::Saved(result) => {
                                this.busy = false;
                                this.oauth_run = None;
                                this.oauth_login = None;
                                match result {
                                    Ok(document) => this.accept(document, window, cx),
                                    Err(error) => this.error = Some(error),
                                }
                            }
                            Update::Cancelled => {
                                this.busy = false;
                                this.oauth_run = None;
                                this.oauth_login = None;
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                if finished {
                    break;
                }
            }
        }));
        cx.notify();
    }

    pub(super) fn disconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.dirty {
            return;
        }
        let mut document = self.confirmed.clone();
        let Some(provider) = document
            .providers
            .iter_mut()
            .find(|value| Some(&value.id) == self.editing.as_ref())
        else {
            return;
        };
        let Some(auth) = provider.request.oauth.as_mut() else {
            return;
        };
        let Some(credential) = auth.credential_id.take() else {
            return;
        };
        let shared = document.providers.iter().any(|value| {
            value
                .request
                .oauth
                .as_ref()
                .and_then(|auth| auth.credential_id.as_ref())
                == Some(&credential)
        });
        let runtime = match crate::chat_runtime::runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let path = self.path.clone();
        let work = runtime.spawn(async move {
            let saved = tokio::task::spawn_blocking(move || document.save(&path))
                .await
                .map_err(|_| "保存授权配置的任务已中断。".to_string())
                .and_then(|value| value)?;
            let cleanup = if shared {
                Ok(())
            } else {
                oauth::provider_oauth_disconnect(credential).await
            };
            Ok::<_, String>((saved, cleanup.err()))
        });
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work
                .await
                .map_err(|_| "断开账号的任务已中断。".to_string())
                .and_then(|value| value);
            let _ = owner.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((document, error)) => {
                        this.accept(document, window, cx);
                        this.error = error;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn fetch_oauth_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.dirty {
            return;
        }
        let Some(provider) = self
            .confirmed
            .providers
            .iter()
            .find(|value| Some(&value.id) == self.editing.as_ref())
            .cloned()
        else {
            return;
        };
        let runtime = match crate::chat_runtime::runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let work = runtime.spawn(async move { oauth::models(&provider).await });
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work
                .await
                .map_err(|_| "读取模型的任务已中断。".to_string())
                .and_then(|value| value);
            let _ = owner.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(models) => {
                        if let Some(provider) = this
                            .draft
                            .iter_mut()
                            .find(|value| Some(&value.id) == this.editing.as_ref())
                        {
                            provider.available_models = models;
                            this.dirty = true;
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn refresh_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.dirty {
            return;
        }
        let Some(provider) = self
            .confirmed
            .providers
            .iter()
            .find(|value| Some(&value.id) == self.editing.as_ref())
            .cloned()
        else {
            return;
        };
        let runtime = match crate::chat_runtime::runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let work = runtime.spawn(async move {
            tokio::join!(
                oauth::account::provider_oauth_account(provider.clone()),
                oauth::usage::provider_oauth_usage(provider)
            )
        });
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work.await;
            let _ = owner.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok((account, usage)) => {
                        let mut errors = Vec::new();
                        match account {
                            Ok(account) => this.oauth_account = Some(account),
                            Err(error) => errors.push(error),
                        }
                        match usage {
                            Ok(usage) => this.oauth_usage = Some(usage),
                            Err(error) => errors.push(error),
                        }
                        this.error = (!errors.is_empty()).then(|| errors.join("\n"));
                    }
                    Err(_) => this.error = Some("读取账号信息的任务已中断。".into()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn render_oauth(&self, provider: &ModelProvider, cx: &Context<Self>) -> AnyElement {
        let signed = provider
            .request
            .oauth
            .as_ref()
            .and_then(|auth| auth.credential_id.as_ref())
            .is_some();
        let mut content = div()
            .v_flex()
            .gap_3()
            .child(div().text_sm().child(if signed {
                "已连接账号"
            } else {
                "使用账号授权，无需填写 API Key。"
            }));
        if let Some(account) = &self.oauth_account {
            let identity = [
                account.name.clone(),
                account.email.clone(),
                account.account_id.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            content = content.child(div().text_sm().child(if identity.is_empty() {
                "账号未提供可显示的信息".into()
            } else {
                identity
            }));
        }
        if let Some(usage) = &self.oauth_usage {
            if let Some(plan) = &usage.plan {
                content = content.child(div().text_sm().child(plan.clone()));
            }
            for window in &usage.windows {
                content = content.child(div().text_sm().child(format!(
                        "{}：{}",
                        window.label,
                        window
                            .used_percent
                            .map(|used| crate::ui_format!("剩余 {:.0}%", "{:.0}% remaining", 100. - used))
                            .unwrap_or_else(|| "用量未知".into())
                    )));
            }
        }
        if let Some(login) = &self.oauth_login {
            let url = login.verification_url.clone();
            let code = login.user_code.clone();
            content = content.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .when(!code.is_empty(), |row| {
                        row.child(div().text_base().child(code.clone()))
                    })
                    .child(
                        Button::new("oauth-open-browser")
                            .primary()
                            .label(crate::i18n::text("打开浏览器授权"))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .when(!code.is_empty(), |row| {
                        row.child(Button::new("oauth-copy-code").label(crate::i18n::text("复制授权码")).on_click(
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()))
                            },
                        ))
                    }),
            );
        }
        content =
            content.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(
                        Button::new("oauth-sign-in")
                            .primary()
                            .label(if self.oauth_run.is_some() {
                                "正在等待授权…"
                            } else {
                                "登录账号"
                            })
                            .disabled(self.busy || self.dirty || signed)
                            .on_click(cx.listener(|this, _, window, cx| this.sign_in(window, cx))),
                    )
                    .when(self.oauth_run.is_some(), |row| {
                        row.child(Button::new("oauth-cancel").label(crate::i18n::text("取消登录")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.cancel_pending_login(cx);
                            }),
                        ))
                    })
                    .when(signed, |row| {
                        row.child(
                            Button::new("oauth-fetch-models")
                                .label(crate::i18n::text("获取可用模型"))
                                .disabled(self.busy || self.dirty)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.fetch_oauth_models(window, cx)
                                })),
                        )
                        .child(
                            Button::new("oauth-account")
                                .label(crate::i18n::text("刷新账号与用量"))
                                .disabled(self.busy || self.dirty)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.refresh_account(window, cx)
                                })),
                        )
                        .child(
                            Button::new("oauth-disconnect")
                                .label(crate::i18n::text("断开账号"))
                                .disabled(self.busy || self.dirty)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.disconnect(window, cx)),
                                ),
                        )
                    }),
            );
        content
            .when(self.dirty, |view| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::i18n::text("保存供应商配置后再登录。")),
                )
            })
            .into_any_element()
    }
}
