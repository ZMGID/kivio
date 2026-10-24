//! Setup and enablement share the existing CLI installer and Skill policy.
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, StyledExt, button::Button, switch::Switch};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{computer_control::{self, ControlTool, ControlToolStatus}, native_preferences::{self, Change, FieldEdit}, settings_config::Settings};
use std::path::PathBuf;

const TOOLS: [(ControlTool, &str, &str); 2] = [
    (ControlTool::Cua, "Cua Driver", "cua-driver"),
    (ControlTool::Playwright, "Playwright CLI", "playwright-cli"),
];
#[derive(Clone, Copy)]
enum Operation { Status, Updates, Install, Update, Enable(bool) }
#[derive(Default, Clone)]
struct ToolState {
    status: Option<ControlToolStatus>,
    skill: Option<String>,
    error: Option<String>,
}
pub struct ComputerTools {
    root: PathBuf,
    ego: Entity<super::ego::EgoControl>,
    busy: bool,
    settings: Option<Settings>,
    states: [ToolState; 2],
    message: Option<Result<String, String>>,
    task: Option<Task<()>>,
}
impl ComputerTools {
    pub(crate) fn is_busy(&self, cx: &App) -> bool { self.busy || self.ego.read(cx).is_busy() }
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let ego = cx.new(|cx| super::ego::EgoControl::new(root.clone(), cx));
        let mut this = Self { root, ego, busy: false, settings: None, states: Default::default(), message: None, task: None };
        this.run(0, Operation::Status, cx);
        this
    }
    fn run(&mut self, index: usize, operation: Operation, cx: &mut Context<Self>) {
        if self.busy { return; }
        let (tool, label, _) = TOOLS[index];
        let before = self.settings.clone();
        let previous_states = self.states.clone();
        let skill = self.states[index].skill.clone();
        self.busy = true;
        self.message = None;
        let root = self.root.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                crate::chat_runtime::runtime()?.block_on(async move {
                    let mut confirmed = native_preferences::load(&root)?;
                    match operation {
                        Operation::Enable(enabled) => {
                            let settings = before.ok_or("工具状态尚未读取")?;
                            let skill = skill.ok_or("未找到控制工具的 Skill，请先安装")?;
                            let edits = control_edits(&settings, &skill, enabled);
                            confirmed = native_preferences::save(&root, Change::Fields(edits))?;
                            kivio_model_runtime::native_tool_runtime::invalidate_resource_catalogs();
                        }
                        Operation::Install | Operation::Update => {
                            let source = if matches!(operation, Operation::Install) {
                                computer_control::computer_control_install(tool).await?
                            } else { computer_control::computer_control_update(tool).await? };
                            if !source.join("SKILL.md").is_file() {
                                return Err("CLI 安装完成，但未找到官方 Skill 文件，请重试安装。".into());
                            }
                            let settings = native_preferences::load(&root)?;
                            let before = settings.chat_tools.skill_scan_paths;
                            let mut paths = before.clone();
                            let source = source.to_string_lossy().into_owned();
                            if !paths.contains(&source) { paths.push(source); }
                            let registry = kivio_model_runtime::skills::build_registry_with_plugins_at(None, &paths, None, &[])?;
                            let skill = registry.find(TOOLS[index].2).ok_or("安装后未找到控制工具的 Skill")?;
                            let settings = native_preferences::load(&root)?;
                            let mut edits = control_edits(&settings, &skill.meta.id, true);
                            edits.push(FieldEdit {
                                path: "/chatTools/skillScanPaths".into(), before: serde_json::json!(before), after: serde_json::json!(paths),
                            });
                            confirmed = native_preferences::save(&root, Change::Fields(edits))?;
                            kivio_model_runtime::native_tool_runtime::invalidate_resource_catalogs();
                        }
                        _ => {}
                    }
                    if matches!(operation, Operation::Enable(_)) {
                        return Ok((confirmed, previous_states));
                    }
                    let registry = kivio_model_runtime::skills::build_registry_with_plugins_at(None, &confirmed.chat_tools.skill_scan_paths, None, &[]);
                    let (cua, playwright) = tokio::join!(
                        computer_control::computer_control_status(ControlTool::Cua, Some(index == 0 && matches!(operation, Operation::Updates))),
                        computer_control::computer_control_status(ControlTool::Playwright, Some(index == 1 && matches!(operation, Operation::Updates))),
                    );
                    let mut states = previous_states;
                    for (i, ((_, _, name), status)) in TOOLS.iter().zip([cua, playwright]).enumerate() {
                        states[i].error = None;
                        match &registry {
                            Ok(registry) => states[i].skill = registry.find(name).map(|record| record.meta.id.clone()),
                            Err(error) => states[i].error = Some(error.clone()),
                        }
                        match status {
                            Ok(status) => states[i].status = Some(status),
                            Err(error) => states[i].error = Some(error),
                        }
                    }
                    Ok::<_, String>((confirmed, states))
                })
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((settings, states)) => {
                        this.settings = Some(settings);
                        this.states = states;
                        if !matches!(operation, Operation::Status) {
                            this.message = Some(Ok(crate::ui_format!("{label}：设置已更新", "{label}: settings updated")));
                        }
                    }
                    Err(error) => this.message = Some(Err(format!("{label}: {error}"))),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for ComputerTools {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div().v_flex().gap_4();
        for (index, (_, label, id)) in TOOLS.iter().enumerate() {
            let state = &self.states[index];
            let ready = state.status.is_some() && state.skill.is_some();
            let enabled = ready && self.settings.as_ref().is_some_and(|settings| {
                let tools = &settings.chat_tools;
                tools.enabled && tools.native_tools.skill_runtime && tools.native_tools.run_command && tools.native_tools.read_file
                    && state.skill.as_ref().is_some_and(|id| !tools.disabled_skill_ids.contains(id))
            });
            let mut actions = div().flex().items_center().gap_2();
            if ready {
                actions = actions.child(Switch::new(SharedString::from(format!("{id}-enabled")))
                    .checked(enabled).disabled(self.busy)
                    .on_click(cx.listener(move |this, checked, _, cx| this.run(index, Operation::Enable(*checked), cx))));
            }
            let primary = if !ready { Some((if state.error.is_some() { "安装 / 修复" } else { "安装" }, Operation::Install)) }
                else if state.status.as_ref().is_some_and(|status| status.update_available) { Some(("更新", Operation::Update)) }
                else { None };
            if let Some((label, operation)) = primary {
                actions = actions.child(Button::new(SharedString::from(format!("{id}-install"))).small()
                    .label(crate::i18n::display(label)).disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.run(index, operation, cx))));
            }
            if ready {
                actions = actions.child(Button::new(SharedString::from(format!("{id}-updates"))).small()
                    .label(crate::i18n::text("检查更新")).disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.run(index, Operation::Updates, cx))));
            }
            let status = state.status.as_ref().map(|status| status.current_version.clone())
                .unwrap_or_else(|| crate::i18n::text(if self.busy { "正在检测…" } else if state.error.is_some() { "检测失败" } else { "未安装" }).into());
            content = content.child(div().flex().items_center().justify_between().gap_4().py_3()
                .border_b_1().border_color(cx.theme().border)
                .child(div().v_flex().gap_1().child(*label).child(div().text_sm().text_color(cx.theme().muted_foreground).child(status)))
                .child(actions));
            if let Some(error) = &state.error {
                content = content.child(div().text_sm().text_color(cx.theme().danger)
                    .child(crate::ui_format!("检测失败：{error}", "Detection failed: {error}")));
            }
        }
        content = content.child(Button::new("control-refresh").small().label(crate::i18n::text("重新检测")).disabled(self.busy)
            .on_click(cx.listener(|this, _, _, cx| this.run(0, Operation::Status, cx))));
        if let Some(result) = &self.message {
            content = content.child(div().text_sm().text_color(if result.is_err() { cx.theme().danger } else { cx.theme().muted_foreground })
                .child(match result { Ok(v) | Err(v) => v.clone() }));
        }
        content.child(self.ego.clone())
    }
}

fn control_edits(settings: &Settings, skill: &str, enabled: bool) -> Vec<FieldEdit> {
    let mut ids = settings.chat_tools.disabled_skill_ids.clone();
    ids.retain(|id| id != skill);
    if !enabled { ids.push(skill.to_owned()); }
    let mut edits = vec![FieldEdit {
        path: "/chatTools/disabledSkillIds".into(),
        before: serde_json::json!(settings.chat_tools.disabled_skill_ids),
        after: serde_json::json!(ids),
    }];
    if enabled {
        for (path, before) in [
            ("/chatTools/enabled", settings.chat_tools.enabled),
            ("/chatTools/nativeTools/skillRuntime", settings.chat_tools.native_tools.skill_runtime),
            ("/chatTools/nativeTools/runCommand", settings.chat_tools.native_tools.run_command),
            ("/chatTools/nativeTools/readFile", settings.chat_tools.native_tools.read_file),
        ] { edits.push(FieldEdit { path: path.into(), before: before.into(), after: true.into() }); }
    }
    edits
}
