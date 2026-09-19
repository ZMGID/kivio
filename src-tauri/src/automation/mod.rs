//! 本机自动化：用户编排的触发器 → 动作图。
//!
//! 分层（不要把跑图塞进存储，也不要把画布 / 执行器塞进 Chat 或 Hooks）：
//! - [`types`]：schema v1
//! - [`storage`]：`{app_data}/automations/{id}.json`
//! - [`mutations`]：保存、启停、删除及热键副作用的单一写入口
//! - [`runner`]：手动 / 定时 / 热键 / Agent 执行
//! - [`schedule`]：托盘进程内 tokio 调度
//! - [`commands`]：Tauri IPC
//! - [`validate`]：agent 落库前的图校验 + 自动布局
//! - [`tools`]：宿主无关工具层（chat native tools 的实现；未来 MCP server 复用）
//!
//! 和聊天生命周期 Hooks 分家：Hooks 是对话旁路观察；自动化是用户意图的主路径。

mod application;
pub(crate) mod commands;
mod events;
mod history;
mod hotkeys;
mod interpolate;
mod mutations;
pub(crate) mod notify;
mod runner;
mod schedule;
mod state;
mod storage;
pub(crate) mod tools;
mod types;
mod validate;
mod workspace;

pub use types::{Automation, AutomationMeta, SCHEMA_VERSION};
pub use workspace::workspace_for_conversation;

pub(crate) use hotkeys::enabled_bindings;
pub(crate) use runner::{
    cancel_all as cancel_all_runs, enqueue, wait_all_finished as wait_runs_finished,
};
pub(crate) use schedule::spawn as spawn_scheduler;
pub(crate) use state::AutomationRunState;
pub(crate) use types::RunOrigin;
