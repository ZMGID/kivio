//! Persistent cross-turn session registry for external CLI agents (Phase 2).
//!
//! A live session keeps the CLI process alive across user turns so the server holds prior
//! context natively (no full-history replay). Each session is owned by a dedicated actor task
//! reachable only through an `mpsc::Sender<SessionCommand>` — the registry never holds the
//! `Child` or any lock across a turn await, only the cheap clonable control sender.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};

use crate::external_agents::types::{RuntimeAgentDef, UnifiedAgentEvent};

tokio::task_local! {
    pub(crate) static PROCESS_CONVERSATION: String;
    pub(crate) static MAINTENANCE_OWNER: ResourceLease;
}

#[derive(Default)]
struct ResourceState {
    uses: HashMap<uuid::Uuid, ResourceEntry>,
}
struct ResourceEntry {
    keys: Vec<String>,
    exclusive: bool,
    conversation: Option<String>,
    pending: bool,
}
static RESOURCE_STATE: std::sync::LazyLock<Mutex<ResourceState>> =
    std::sync::LazyLock::new(Default::default);

pub(crate) fn resource_path(path: &std::path::Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let components: Vec<_> = absolute.components().collect();
    let mut normalized = absolute.clone();
    for count in (1..=components.len()).rev() {
        let ancestor: std::path::PathBuf = components[..count].iter().collect();
        if let Ok(mut resolved) = std::fs::canonicalize(ancestor) {
            for component in &components[count..] {
                match component {
                    std::path::Component::CurDir => {}
                    std::path::Component::ParentDir => {
                        resolved.pop();
                    }
                    other => resolved.push(other.as_os_str()),
                }
            }
            normalized = resolved;
            break;
        }
    }
    let value = crate::utils::strip_windows_verbatim_prefix(normalized)
        .to_string_lossy()
        .replace('\\', "/");
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}

pub(crate) fn runtime_resources(agent: Option<&str>, binary: &std::path::Path) -> Vec<String> {
    let mut keys = vec![format!("binary:{}", resource_path(binary))];
    if let Some(agent) = agent {
        keys.push(format!("agent:{agent}"));
        if let Some(def) = crate::external_agents::registry::get_agent_def(agent) {
            keys.extend(
                crate::external_agents::provider_profile::config_resource_keys(
                    def,
                    &crate::external_agents::overrides::agent_config(agent).unwrap_or_default(),
                ),
            );
            if let Some(package) = def.install.npm_package {
                if let Some(prefix) =
                    crate::external_agents::installer::proven_npm_prefix(binary, package)
                {
                    keys.push(format!("prefix:{}", resource_path(&prefix)));
                }
            }
        }
    }
    keys
}

#[derive(Clone)]
pub(crate) struct ResourceLease(Arc<ResourceLeaseOwner>);
struct ResourceLeaseOwner {
    id: uuid::Uuid,
}
impl ResourceLease {
    pub(crate) fn acquire(keys: Vec<String>, exclusive: bool) -> Result<Self, String> {
        Self::acquire_allowing(keys, exclusive, &Default::default(), false)
    }
    fn acquire_allowing(
        keys: Vec<String>,
        exclusive: bool,
        idle: &std::collections::HashSet<String>,
        pending: bool,
    ) -> Result<Self, String> {
        let mut state = RESOURCE_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let owner = MAINTENANCE_OWNER.try_with(|lease| lease.id()).ok();
        if exclusive {
            if let Some(owner_id) = owner {
                if state.uses.get(&owner_id).is_some_and(|entry| {
                    entry.exclusive && keys.iter().all(|key| entry.keys.contains(key))
                }) {
                    return MAINTENANCE_OWNER
                        .try_with(Clone::clone)
                        .map_err(|_| "维护授权已失效".into());
                }
            }
        }
        for (id, entry) in &state.uses {
            if Some(*id) == owner {
                continue;
            }
            if (exclusive || entry.exclusive) && keys.iter().any(|key| entry.keys.contains(key)) {
                if !entry.exclusive
                    && !entry.pending
                    && entry
                        .conversation
                        .as_ref()
                        .is_some_and(|id| idle.contains(id))
                {
                    continue;
                }
                return Err("外部 CLI 正在使用或维护中，请稍后重试".into());
            }
        }
        let id = uuid::Uuid::new_v4();
        state.uses.insert(
            id,
            ResourceEntry {
                keys,
                exclusive,
                conversation: PROCESS_CONVERSATION.try_with(Clone::clone).ok(),
                pending,
            },
        );
        Ok(Self(Arc::new(ResourceLeaseOwner { id })))
    }
    pub(crate) fn pending(agent: &str) -> Result<Self, String> {
        Self::acquire_allowing(
            vec![format!("agent:{agent}")],
            false,
            &Default::default(),
            true,
        )
    }
    pub(crate) fn id(&self) -> uuid::Uuid {
        self.0.id
    }
}
impl Drop for ResourceLeaseOwner {
    fn drop(&mut self) {
        RESOURCE_STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .uses
            .remove(&self.id);
    }
}

/// How long an unused live CLI **process** may sit before the sweeper drops it.
///
/// This is only a memory cap. The native session id stays on disk (`live-*.json`) and
/// the next turn — 40 minutes later, or after the app restarts — must `thread/resume`
/// / `--resume` that same id. Do not treat this TTL as a context lifetime.
pub const LIVE_SESSION_IDLE_TTL: Duration = Duration::from_secs(600);

/// 一条待用户答复的工具审批询问（claude 的 `control_request` / `can_use_tool`）。
///
/// 会话侧只负责把它送出去、并在收到 `ApprovalDecision` 后回一条 `control_response`；
/// 「弹卡片给谁看」是宿主（app 层）的事，会话里没有 `AppHandle` 也不该有。
#[derive(Debug, Clone)]
pub struct ApprovalAsk {
    /// 这次交互的宿主标识。原生请求标识由会话持有。
    pub interaction_id: String,
    /// claude 的 `toolu_…`。用作 Kivio 侧的 `toolCallId`，这样审批卡和工具卡指向同一个 id。
    /// CLI 偶尔不给（schema 里是 optional），缺失时回落到 `request_id`。
    pub tool_call_id: String,
    /// CLI 报的工具原名（`Write` / `Bash` / `mcp__server__tool`，PascalCase 有意义，不归一化）。
    pub tool_name: String,
    /// 工具入参原文，用于卡片上的摘要。
    pub input: serde_json::Value,
    /// CLI 标记「这个工具要用户在卡片上直接作答」。问用户经 `ask_user::codec_for`
    /// 转成 Kivio 已有的卡片，答复经 `ApprovalDecision::updated_input` 回去；
    /// claude 的 `ExitPlanMode` 走审批卡、批准时额外经
    /// `ApprovalDecision::set_permission_mode` 切档位。其余（CLI 将来新增的
    /// 交互工具）仍当场拒（见 `claude_stream::APPROVAL_INTERACTIVE_UNSUPPORTED`）。
    pub requires_user_interaction: bool,
    /// CLI 声明这次询问**必须由人确认**，「完全」档也不能替用户点头。claude 2.1.281+ 的
    /// 安全检查（危险 `rm`、敏感文件 / 工作区外符号链接写入）在 `can_use_tool` 上带
    /// `classifier_approvable: false` 或 `default_to_no: true`；其余 codec 恒为 false。
    pub requires_manual_approval: bool,
    /// claude `can_use_tool.permission_suggestions`：CLI 建议的「以后别再问」规则。
    /// 用户点「总是允许」时改成 session 作用域回给 CLI（见 `ApprovalDecision::updated_permissions`）。
    pub permission_suggestions: Option<serde_json::Value>,
}

/// 用户对某条询问的答复。`interaction_id` 与 `ApprovalAsk` 一一对应。
#[derive(Debug, Clone)]
pub struct ApprovalDecision {
    pub interaction_id: String,
    pub approved: bool,
    /// 批准时要**改写**给 CLI 的工具入参（官方 `PermissionResult.updatedInput`）。
    ///
    /// 存在的唯一理由是 `AskUserQuestion`：官方答法就是从这条通道回
    /// `allow + updatedInput.answers`（见 `claude_stream::APPROVAL_INTERACTIVE_UNSUPPORTED`），
    /// 没有第二条控制请求。`None` = 不改入参，CLI 用原入参。
    pub updated_input: Option<serde_json::Value>,
    /// 回这条答复**之前**要先把 CLI 切到的权限档位（`set_permission_mode` 控制请求）。
    ///
    /// 存在的唯一理由是 `ExitPlanMode`：批准一个计划 = 「同意，去做吧」，而 CLI 不会因为
    /// 这次 `allow` 自己离开计划档 —— 不切档位的话它下一句 `Edit` 又被计划档挡回来，
    /// 用户点了「批准」却什么都没发生。paseo 的做法一样（`await setPermissionMode(...)`
    /// 在 resolve 之前），顺序不能反：先切档、再放行。
    pub set_permission_mode: Option<String>,
    /// 批准时一并回给 CLI 的权限规则（claude `PermissionResult.updatedPermissions`）。
    /// 只在用户点「总是允许」时有值，作用域一律 `session`。
    pub updated_permissions: Option<serde_json::Value>,
    /// 用户点的是「总是允许」（或此前点过、本次由 Kivio 的 always-allow 表放行）。
    /// codex 据此回 `acceptForSession`，让 CLI 本会话内不再问（t3code `acceptAlways` 同款）。
    pub always: bool,
}

/// 一轮的权限审批通道。宿主持 `requests` 的接收端与 `decisions` 的发送端；会话持另一半。
pub struct ApprovalBridge {
    /// 会话 → 宿主。
    pub requests: mpsc::Sender<ApprovalAsk>,
    /// 宿主 → 会话。
    pub decisions: mpsc::Receiver<ApprovalDecision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageInjectionKind {
    Steer,
    FollowUp,
}

/// Shared evidence for one dispatch attempt, advanced by the actor before the user operation write.
#[derive(Clone, Default)]
pub struct TurnDelivery(Arc<AtomicBool>);

impl TurnDelivery {
    pub fn mark_possibly_sent(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn possibly_sent(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// A command sent to a live session's actor task.
pub enum SessionCommand {
    /// Run one turn: write the prompt, stream `UnifiedAgentEvent`s into `events`, and report the
    /// terminal result through `done`. The actor processes exactly one turn at a time.
    RunTurn {
        delivery: TurnDelivery,
        cancelled: Option<tokio_util::sync::CancellationToken>,
        prompt: String,
        model: Option<String>,
        reasoning: Option<String>,
        /// 本轮用户消息的原生图片块（ACP → image content block；Codex → localImage 临时文件）。空=无图。
        images: Vec<crate::external_agents::attachments::ImageBlock>,
        /// Codex `workspace-write` 默认锁在 cwd：附件目录 / 临时图片必须作为
        /// `runtimeWorkspaceRoots` 下发，否则 CLI 读不到。其它协议忽略。
        extra_writable_roots: Vec<String>,
        events: mpsc::Sender<UnifiedAgentEvent>,
        done: oneshot::Sender<Result<(), String>>,
        /// 本轮的权限审批通道。`None` = 宿主不接权限询问（未启用 / 协议不支持）⇒ 会话对
        /// `can_use_tool` 仍走那条 fail-closed 的 error 兜底，绝不沉默（spec 第 29 条）。
        approvals: Option<ApprovalBridge>,
    },
    /// Interrupt the in-flight turn without killing the process (protocol-level interrupt).
    Cancel,
    /// 运行中注入一条用户消息：不中断在飞的轮次。
    ///
    /// `kind` 区分立刻引导（当前轮）与 follow-up（下一轮）。`accepted` 回 true 只表示
    /// 协议层受理了（codex `turn/steer`、Pi `steer` / `follow_up`、dsh `session/steer` /
    /// `session/prompt`）。
    /// 回 false 的情形都要让调用方把这条消息留在队列里、按普通消息在轮末发出去：
    /// 轮次之间没有可注入的对象、该 CLI 的协议不支持、或者对端明确拒绝
    /// （codex 的 review / compact 轮次不可 steer）。**绝不能悄悄吞掉**。
    Steer {
        /// 前端生成的 id，原样回到 `user_steer` 卡上供前端对账出队。
        id: String,
        text: String,
        images: Vec<crate::external_agents::attachments::ImageBlock>,
        kind: MessageInjectionKind,
        accepted: oneshot::Sender<bool>,
    },
    /// 停止 CLI 侧的一个后台任务（Background tasks 面板的停止按钮）。
    ///
    /// claude 写 `control_request` 的 `stop_task`；dsh 写 `session/stop-job`
    ///（bridge：`ctx.jobs.kill`，子代理 `childSessionId` 走 `subagents.interrupt`）。
    /// 其余 CLI 的 actor 收到后直接忽略。无 ack —— 发起方已乐观置 stopped，
    /// 注册表由后续任务边沿帧修正。
    StopTask { task_id: String },
    /// Shut the session down (close stdin + kill the child) and end the actor.
    Close,
}

/// 「用户已取消，但这个会话本身已经不能再用了」的哨兵。
///
/// 出口按**取消**呈现（不弹错误气泡、不发上下文重置提示、更不重发本轮 prompt——用户刚刚
/// 才把它停掉），但注册表条目必须**丢弃**：进程已经死了，或者协议级取消超时后被硬 `Close`
/// 掉了。用普通的 `"cancelled"` 会让 claude 的「取消后保留常驻会话」把一个死 actor 留下来，
/// 下一轮才发现。
pub const CANCELLED_SESSION_LOST: &str = "__cancelled_session_lost__";

/// 影响 CLI **启动参数**的配置指纹。
///
/// 常驻打破了一个此前白捡的便宜：换模型 / 换 sandbox 档位 / 换 reasoning 档位 / 改系统提示
/// 或 Memory，靠的是「下一轮换个新进程带新 flag」自动生效。进程一常驻就不生效了 ——
/// 界面显示一套、会话实际跑另一套，这**违反 spec 第 8 条**（UI 所见必须与会话实际配置一致），
/// 是功能退步而不是缺功能。指纹变了就换个进程。
///
/// 只有启动时锁死、会话内改不了的配置才进指纹（claude：`--effort` / `--permission-mode`；
/// codex：sandbox / approvalPolicy，只在 `thread/start`）。Pi 和 ACP 可在会话内切换配置。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchConfig {
    /// `model|reasoning|sandbox`，恒可知。
    pub flags: String,
    /// 启动时注入的会话级系统指令哈希。`None` = 本轮不注入（斜杠命令走 passthrough，
    /// 不带 `--append-system-prompt-file`），**不参与判定**——否则一条斜杠命令会把常驻进程
    /// 重启一次、紧跟的普通消息再重启一次，来回抖。
    pub instructions: Option<String>,
    pub(crate) identity: Option<crate::external_agents::overrides::LaunchIdentity>,
}

impl LaunchConfig {
    /// 已建立的会话（`self`，注册时的配置）能否服务配置为 `incoming` 的这一轮。
    pub fn accepts(&self, incoming: &LaunchConfig) -> bool {
        if self.flags != incoming.flags || self.identity != incoming.identity {
            return false;
        }
        match incoming.instructions.as_deref() {
            // 本轮不注入指令 ⇒ 不据此判定（见字段注释）。
            None => true,
            // 本轮要注入 ⇒ 会话必须是带着**同一份**指令启动的。注册时为 `None`
            // （例如会话是被一条斜杠命令拉起来的）同样算不匹配，否则用户配置的系统提示
            // 与 Memory 会在这个会话里静默失效。
            Some(hash) => self.instructions.as_deref() == Some(hash),
        }
    }
}

/// Clones retain actor ownership; the final owner aborts a forgotten actor.
#[derive(Clone)]
pub struct SessionControl {
    sender: mpsc::Sender<SessionCommand>,
    owner: Option<Arc<ActorOwner>>,
    instance_id: uuid::Uuid,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionIdentity(Arc<Mutex<Option<SessionInstance>>>);

#[derive(Debug)]
struct SessionInstance {
    id: uuid::Uuid,
    sender: mpsc::WeakSender<SessionCommand>,
}

impl SessionIdentity {
    pub(crate) fn from_control(control: &SessionControl) -> Self {
        let identity = Self::default();
        identity.bind(control);
        identity
    }
    pub(crate) fn same_instance(&self, other: &Self) -> bool {
        matches!((self.instance_id(), other.instance_id()), (Some(a), Some(b)) if a == b)
    }
    pub(crate) fn instance_id(&self) -> Option<uuid::Uuid> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|instance| instance.id)
    }
    pub(crate) fn sender(&self) -> Option<mpsc::Sender<SessionCommand>> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|instance| instance.sender.upgrade())
    }
    pub(crate) async fn closed(&self) {
        if let Some(sender) = self.sender() {
            sender.closed().await;
        }
    }
    pub(crate) fn bind(&self, control: &SessionControl) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(SessionInstance {
            id: control.instance_id,
            sender: control.sender.downgrade(),
        });
    }
}
struct ActorOwner {
    abort: tokio::task::AbortHandle,
    finished: tokio::sync::watch::Receiver<bool>,
}
impl Drop for ActorOwner {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
impl std::ops::Deref for SessionControl {
    type Target = mpsc::Sender<SessionCommand>;
    fn deref(&self) -> &Self::Target {
        &self.sender
    }
}
impl SessionControl {
    pub fn spawn(
        sender: mpsc::Sender<SessionCommand>,
        task: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Self {
        let task = tokio::spawn(task);
        let abort = task.abort_handle();
        let (finished, receipt) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            let _ = task.await;
            let _ = finished.send(true);
        });
        Self {
            sender,
            instance_id: uuid::Uuid::new_v4(),
            owner: Some(Arc::new(ActorOwner {
                abort,
                finished: receipt,
            })),
        }
    }
    pub(crate) fn abort(&self) {
        if let Some(owner) = &self.owner {
            owner.abort.abort();
        }
    }
    pub async fn finished(&self) {
        if let Some(owner) = &self.owner {
            let mut receipt = owner.finished.clone();
            let _ = receipt.wait_for(|done| *done).await;
        } else {
            self.sender.closed().await;
        }
    }
    pub async fn close(&self) -> Result<(), String> {
        let graceful = async {
            let _ = self.sender.send(SessionCommand::Close).await;
            self.finished().await;
        };
        if tokio::time::timeout(Duration::from_millis(1_500), graceful)
            .await
            .is_err()
        {
            if let Some(owner) = &self.owner {
                owner.abort.abort();
                tokio::time::timeout(Duration::from_secs(1), self.finished())
                    .await
                    .map_err(|_| "external CLI actor cleanup timed out".to_string())?;
            }
        }
        Ok(())
    }
    #[cfg(test)]
    fn untracked(sender: mpsc::Sender<SessionCommand>) -> Self {
        Self {
            sender,
            instance_id: uuid::Uuid::new_v4(),
            owner: None,
        }
    }
}
pub(crate) struct AbortOnDrop(pub tokio::task::AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Registry entry: the control channel plus metadata used to decide reuse.
pub struct LiveSession {
    pub control: SessionControl,
    pub(crate) _chat_credential: Option<crate::app_mcp::McpCredential>,
    pub agent_id: String,
    pub cwd: String,
    /// 会话建立时生效的启动配置。与本轮的指纹不符 ⇒ 不可复用（见 `LaunchConfig`）。
    pub launch_config: LaunchConfig,
    /// Last time a turn was sent/started; drives idle reclamation + LRU eviction.
    pub last_activity: Instant,
    /// 常驻子进程的 pid。**纯元数据**，注册表不拿它做任何决策（关停一律走 actor 的
    /// `Close`，绝不按 pid 杀）。存它是因为「这两轮是不是同一个进程」在别处根本没有
    /// 可观测信号——不记就只能去数系统进程表。
    pub child_pid: Option<u32>,
    /// 这个进程已经服过几轮（注册即 1，之后每次被复用 +1）。同样是纯元数据。
    pub turns_served: u32,
    /// 本会话当前有没有在飞的轮次。
    ///
    /// `last_activity` 只在轮**开始**时写一次，轮内的帧不刷新它 —— 而一轮跑十几分钟是
    /// 合法的（没有轮内超时）。少了这一位，清扫器和 LRU 会把正在干活的会话回收掉：
    /// 这一轮不会断（调用方自己持着 `control` 克隆），但轮末最后一个 sender 落地 ⇒ actor
    /// 关掉进程 ⇒ 下一轮白付一次冷启动。结果是**越重度使用的会话越吃不到常驻**，而且
    /// 没有任何可观测信号。
    pub busy: Arc<AtomicBool>,
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.control.abort();
    }
}

/// 轮次期间把会话标成 busy，任何退出路径（含 `?` 与 panic）都会清掉。
pub struct TurnBusyGuard(Arc<AtomicBool>);

impl TurnBusyGuard {
    pub fn new(busy: Arc<AtomicBool>) -> Self {
        busy.store(true, Ordering::Release);
        Self(busy)
    }

    pub fn try_new(busy: Arc<AtomicBool>) -> Option<Self> {
        busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(busy))
    }
}

impl Drop for TurnBusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl LiveSession {
    /// A session is reusable only if its actor is still listening, it targets the same
    /// agent + working directory as the incoming turn, and it was launched with a
    /// configuration that still matches what the UI currently shows.
    pub fn is_reusable(&self, agent_id: &str, cwd: &str, launch_config: &LaunchConfig) -> bool {
        !self.control.is_closed()
            && self.agent_id == agent_id
            && self.cwd == cwd
            && self.launch_config.accepts(launch_config)
    }

    /// Reclaimable: the actor already exited, or the session has been idle past `ttl`.
    ///
    /// 有在飞轮次时永不可回收 —— 见 `busy`。
    pub fn is_idle(&self, ttl: Duration) -> bool {
        if self.control.is_closed() {
            return true;
        }
        if self.busy.load(Ordering::Acquire) {
            return false;
        }
        Instant::now().saturating_duration_since(self.last_activity) >= ttl
    }
}

/// Owns every reusable external CLI process handle. The map lock is held only
/// while cloning/removing control handles; command sends and shutdown waits
/// always happen after the guard has been dropped.
#[derive(Default)]
pub(crate) struct LiveSessionRegistry {
    sessions: Mutex<RegistryState>,
}

#[derive(Default)]
struct RegistryState {
    live: HashMap<String, LiveSession>,
    pending: HashMap<String, Arc<PendingState>>,
    shutting_down: bool,
    deleting: std::collections::HashSet<String>,
}
impl std::ops::Deref for RegistryState {
    type Target = HashMap<String, LiveSession>;
    fn deref(&self) -> &Self::Target {
        &self.live
    }
}
impl std::ops::DerefMut for RegistryState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.live
    }
}
struct PendingState {
    cancel: tokio_util::sync::CancellationToken,
    finished: tokio_util::sync::CancellationToken,
}
pub(crate) struct ConversationRetirement<'a> {
    registry: &'a LiveSessionRegistry,
    conversation_id: String,
}
impl Drop for ConversationRetirement<'_> {
    fn drop(&mut self) {
        let mut registry = self
            .registry
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        registry.deleting.remove(&self.conversation_id);
        if registry
            .pending
            .get(&self.conversation_id)
            .is_some_and(|entry| entry.finished.is_cancelled())
        {
            registry.pending.remove(&self.conversation_id);
        }
    }
}
pub(crate) struct SessionLease<'a> {
    registry: &'a LiveSessionRegistry,
    conversation_id: String,
    state: Arc<PendingState>,
    published: Mutex<Option<SessionControl>>,
    completed: AtomicBool,
}
impl SessionLease<'_> {
    pub(crate) fn track(&self, control: &SessionControl) {
        *self.published.lock().unwrap_or_else(|e| e.into_inner()) = Some(control.clone());
    }
    pub(crate) fn cancel(&self) {
        self.state.cancel.cancel();
    }
    pub(crate) fn complete(&self) {
        self.completed.store(true, Ordering::Release);
    }
    pub(crate) async fn cancelled(&self) {
        self.state.cancel.cancelled().await;
    }
    pub(crate) fn is_cancelled(&self) -> bool {
        self.state.cancel.is_cancelled()
    }
    pub(crate) fn publish(
        &self,
        session: LiveSession,
        commit: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut state = self
            .registry
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.shutting_down
            || self.is_cancelled()
            || !state
                .pending
                .get(&self.conversation_id)
                .is_some_and(|current| Arc::ptr_eq(current, &self.state))
        {
            return Err("cancelled".into());
        }
        commit()?;
        *self.published.lock().unwrap_or_else(|e| e.into_inner()) = Some(session.control.clone());
        LiveSessionRegistry::register_locked(&mut state, self.conversation_id.clone(), session);
        Ok(())
    }
}
impl Drop for SessionLease<'_> {
    fn drop(&mut self) {
        let mut registry = self
            .registry
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let control = self
            .published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let keep_live = self.completed.load(Ordering::Acquire)
            && control.as_ref().is_some_and(|control| {
                registry
                    .get(&self.conversation_id)
                    .is_some_and(|entry| entry.control.same_channel(control))
            });
        if let Some(control) = control.filter(|_| !keep_live) {
            control.abort();
            if registry
                .get(&self.conversation_id)
                .is_some_and(|entry| entry.control.same_channel(&control))
            {
                registry.remove(&self.conversation_id);
            }
            let state = self.state.clone();
            tokio::spawn(async move {
                control.finished().await;
                state.finished.cancel();
            });
        } else {
            if registry
                .pending
                .get(&self.conversation_id)
                .is_some_and(|current| Arc::ptr_eq(current, &self.state))
            {
                registry.pending.remove(&self.conversation_id);
            }
            self.state.finished.cancel();
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) struct LiveSessionDiagnostic {
    pub(crate) registered: bool,
    pub(crate) alive: bool,
    pub(crate) child_pid: Option<u32>,
    pub(crate) turns_served: Option<u32>,
    pub(crate) registry_size: usize,
}

impl LiveSessionRegistry {
    pub(crate) async fn prepare_settings_change(
        &self,
        agents: &[String],
        keys: Vec<String>,
    ) -> Result<Option<ResourceLease>, String> {
        if agents.is_empty() {
            return Ok(None);
        }
        let (lease, controls, idle) = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let affected: Vec<_> = sessions
                .iter()
                .filter(|(_, session)| agents.contains(&session.agent_id))
                .map(|(id, session)| (id.clone(), session.busy.load(Ordering::Acquire)))
                .collect();
            if affected.iter().any(|(_, busy)| *busy) {
                return Err("外部 CLI 正在回复，请结束后重试".into());
            }
            let idle: std::collections::HashSet<String> =
                affected.iter().map(|(id, _)| id.clone()).collect();
            let lease = ResourceLease::acquire_allowing(keys, true, &idle, false)?;
            let controls: Vec<_> = affected
                .into_iter()
                .filter_map(|(id, _)| sessions.remove(&id))
                .collect();
            (lease, controls, idle)
        };
        for session in &controls {
            session.control.close().await?;
        }
        drop(controls);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let alive = RESOURCE_STATE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .uses
                    .values()
                    .any(|entry| {
                        entry
                            .conversation
                            .as_ref()
                            .is_some_and(|id| idle.contains(id))
                    });
                if !alive {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "CLI 进程尚未退出，请稍后重试".to_string())?;
        Ok(Some(lease))
    }

    pub(crate) fn reserve(&self, conversation_id: &str) -> Result<SessionLease<'_>, String> {
        let mut registry = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if registry
            .pending
            .get(conversation_id)
            .is_some_and(|entry| entry.finished.is_cancelled())
        {
            registry.pending.remove(conversation_id);
        }
        if registry.shutting_down
            || registry.deleting.contains(conversation_id)
            || registry.pending.contains_key(conversation_id)
        {
            return Err("external CLI session is busy or shutting down".into());
        }
        let state = Arc::new(PendingState {
            cancel: tokio_util::sync::CancellationToken::new(),
            finished: tokio_util::sync::CancellationToken::new(),
        });
        registry
            .pending
            .insert(conversation_id.into(), state.clone());
        Ok(SessionLease {
            registry: self,
            conversation_id: conversation_id.into(),
            state,
            published: Mutex::new(None),
            completed: AtomicBool::new(false),
        })
    }

    pub(crate) fn with_current<T>(
        &self,
        conversation_id: &str,
        identity: &SessionIdentity,
        operation: impl FnOnce() -> T,
    ) -> Option<T> {
        let registry = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let sender = identity.sender()?;
        registry
            .get(conversation_id)
            .filter(|entry| !sender.is_closed() && entry.control.same_channel(&sender))
            .map(|_| operation())
    }
    pub(crate) fn is_identity_current(
        &self,
        conversation_id: &str,
        identity: &SessionIdentity,
    ) -> bool {
        self.with_current(conversation_id, identity, || ())
            .is_some()
    }
    pub(crate) fn is_current(&self, conversation_id: &str, control: &SessionControl) -> bool {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(conversation_id)
            .is_some_and(|entry| entry.control.same_channel(control))
    }

    pub(crate) fn remove_exact(&self, conversation_id: &str, control: &SessionControl) -> bool {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if sessions
            .get(conversation_id)
            .is_some_and(|entry| entry.control.same_channel(control))
        {
            sessions.remove(conversation_id);
            true
        } else {
            false
        }
    }

    pub(crate) fn claim_control(
        &self,
        conversation_id: &str,
        agent_id: &str,
        cwd: &str,
        config: &LaunchConfig,
    ) -> Result<Option<(SessionControl, TurnBusyGuard, bool)>, String> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let Some(session) = sessions.get_mut(conversation_id) else {
            return Ok(None);
        };
        if session.busy.load(Ordering::Acquire) {
            return Err("external CLI session is busy".into());
        }
        let reusable = session.is_reusable(agent_id, cwd, config);
        let busy =
            TurnBusyGuard::try_new(session.busy.clone()).ok_or("external CLI session is busy")?;
        session.last_activity = Instant::now();
        session.turns_served = session.turns_served.saturating_add(1);
        Ok(Some((session.control.clone(), busy, reusable)))
    }

    #[cfg(debug_assertions)]
    pub(crate) fn diagnostic(&self, conversation_id: &str) -> LiveSessionDiagnostic {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let registry_size = sessions.len();
        match sessions.get(conversation_id) {
            Some(session) => LiveSessionDiagnostic {
                registered: true,
                alive: !session.control.is_closed(),
                child_pid: session.child_pid,
                turns_served: Some(session.turns_served),
                registry_size,
            },
            None => LiveSessionDiagnostic {
                registered: false,
                alive: false,
                child_pid: None,
                turns_served: None,
                registry_size,
            },
        }
    }

    pub(crate) fn mark_busy_exact(
        &self,
        conversation_id: &str,
        control: &SessionControl,
    ) -> Option<TurnBusyGuard> {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        sessions
            .get(conversation_id)
            .filter(|session| session.control.same_channel(control))
            .map(|session| TurnBusyGuard::new(session.busy.clone()))
    }

    pub(crate) fn control_any(&self, conversation_id: &str) -> Option<SessionControl> {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(conversation_id)
            .map(|session| session.control.clone())
    }

    pub(crate) fn injection_control(
        &self,
        conversation_id: &str,
    ) -> Option<(SessionControl, &'static RuntimeAgentDef)> {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let session = sessions.get(conversation_id)?;
        let definition = crate::external_agents::registry::get_agent_def(&session.agent_id)?;
        Some((session.control.clone(), definition))
    }

    pub(crate) fn register(&self, conversation_id: String, session: LiveSession) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        Self::register_locked(&mut sessions, conversation_id, session);
    }
    fn register_locked(
        sessions: &mut RegistryState,
        conversation_id: String,
        session: LiveSession,
    ) {
        const MAX_LIVE_SESSIONS: usize = 6;
        if sessions.shutting_down {
            return;
        }
        sessions.retain(|_, existing| !existing.is_idle(LIVE_SESSION_IDLE_TTL));
        while sessions.len() >= MAX_LIVE_SESSIONS {
            let Some(oldest) = sessions
                .iter()
                .filter(|(_, existing)| !existing.busy.load(Ordering::Acquire))
                .min_by_key(|(_, existing)| existing.last_activity)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            sessions.remove(&oldest);
        }
        sessions.insert(conversation_id, session);
    }

    pub(crate) async fn retire_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationRetirement<'_>, String> {
        let (retirement, session, pending) = {
            let mut registry = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if !registry.deleting.insert(conversation_id.into()) {
                return Err("external CLI conversation is already being deleted".into());
            }
            let retirement = ConversationRetirement {
                registry: self,
                conversation_id: conversation_id.into(),
            };
            let session = registry.remove(conversation_id);
            let mut pending = registry
                .pending
                .get(conversation_id)
                .filter(|entry| !entry.finished.is_cancelled())
                .cloned();
            if pending.is_none() {
                if let Some(session) = &session {
                    let state = Arc::new(PendingState {
                        cancel: tokio_util::sync::CancellationToken::new(),
                        finished: tokio_util::sync::CancellationToken::new(),
                    });
                    registry
                        .pending
                        .insert(conversation_id.into(), state.clone());
                    pending = Some(state.clone());
                    let control = session.control.clone();
                    tokio::spawn(async move {
                        control.finished().await;
                        state.finished.cancel();
                    });
                }
            }
            if let Some(pending) = &pending {
                pending.cancel.cancel();
            }
            (retirement, session, pending)
        };
        if let Some(session) = session {
            session.control.close().await?;
        }
        if let Some(pending) = pending {
            tokio::time::timeout(Duration::from_secs(5), pending.finished.cancelled())
                .await
                .map_err(|_| "external CLI connection cleanup timed out".to_string())?;
        }
        Ok(retirement)
    }

    pub(crate) fn remove(&self, conversation_id: &str) -> bool {
        let mut state = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pending) = state.pending.get(conversation_id) {
            pending.cancel.cancel();
        }
        state.remove(conversation_id).is_some()
    }

    pub(crate) fn move_session(
        &self,
        source_conversation_id: &str,
        destination_conversation_id: &str,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if sessions.contains_key(destination_conversation_id) {
            return Err("destination conversation already has a live session".to_string());
        }
        let mut session = sessions
            .remove(source_conversation_id)
            .ok_or_else(|| "source live session disappeared".to_string())?;
        session.last_activity = Instant::now();
        sessions.insert(destination_conversation_id.to_string(), session);
        Ok(())
    }

    pub(crate) fn sweep_idle(&self, idle_ttl: Duration) -> usize {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let before = sessions.len();
        sessions.retain(|_, session| !session.is_idle(idle_ttl));
        before - sessions.len()
    }

    /// Drain under the lock, then await actor shutdown without holding it.
    pub(crate) async fn close_all(&self) {
        let (sessions, pending) = {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            sessions.shutting_down = true;
            let pending: Vec<_> = sessions.pending.values().cloned().collect();
            for entry in &pending {
                entry.cancel.cancel();
            }
            (
                sessions
                    .drain()
                    .map(|(_, session)| session)
                    .collect::<Vec<_>>(),
                pending,
            )
        };
        futures::future::join_all(sessions.into_iter().map(|session| async move {
            let _ = session.control.close().await;
        }))
        .await;
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            futures::future::join_all(pending.iter().map(|entry| entry.finished.cancelled())),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(agent: &str, cwd: &str) -> (LiveSession, mpsc::Receiver<SessionCommand>) {
        let (tx, rx) = mpsc::channel(1);
        (
            LiveSession {
                control: SessionControl::untracked(tx),
                _chat_credential: None,
                agent_id: agent.to_string(),
                cwd: cwd.to_string(),
                launch_config: LaunchConfig::default(),
                last_activity: Instant::now(),
                child_pid: None,
                turns_served: 1,
                busy: Arc::new(AtomicBool::new(false)),
            },
            rx,
        )
    }

    #[test]
    fn resource_key_survives_nested_path_creation_and_windows_prefixes() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("new").join("nested").join("settings.json");
        let before = resource_path(&target);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"{}").unwrap();
        assert_eq!(before, resource_path(&target));
        assert_eq!(
            before,
            resource_path(
                &target
                    .parent()
                    .unwrap()
                    .join("..")
                    .join("nested")
                    .join("settings.json")
            )
        );
        assert_eq!(
            before,
            resource_path(&std::fs::canonicalize(&target).unwrap())
        );
    }
    #[tokio::test]
    async fn settings_preparation_rejects_busy_and_pending_then_retires_idle_actor() {
        let registry = LiveSessionRegistry::default();
        let agent = format!("test-{}", uuid::Uuid::new_v4());
        let conversation = uuid::Uuid::new_v4().to_string();
        let keys = vec![format!("agent:{agent}")];
        let (mut session, _) = make(&agent, "/project");
        let (tx, mut rx) = mpsc::channel(1);
        let runtime = PROCESS_CONVERSATION
            .sync_scope(conversation.clone(), || {
                ResourceLease::acquire(keys.clone(), false)
            })
            .unwrap();
        let control = SessionControl::spawn(tx, async move {
            let _runtime = runtime;
            while let Some(command) = rx.recv().await {
                if matches!(command, SessionCommand::Close) {
                    break;
                }
            }
        });
        session.control = control.clone();
        session.busy.store(true, Ordering::Release);
        let busy = session.busy.clone();
        registry.register(conversation.clone(), session);
        assert!(registry
            .prepare_settings_change(&[agent.clone()], keys.clone())
            .await
            .is_err());
        assert!(!control.is_closed());
        busy.store(false, Ordering::Release);
        let pending = ResourceLease::pending(&agent).unwrap();
        assert!(registry
            .prepare_settings_change(&[agent.clone()], keys.clone())
            .await
            .is_err());
        drop(pending);
        let guard = registry
            .prepare_settings_change(&[agent.clone()], keys)
            .await
            .unwrap()
            .unwrap();
        assert!(control.is_closed());
        assert!(registry.control_any(&conversation).is_none());
        assert!(ResourceLease::pending(&agent).is_err());
        drop(guard);
        assert!(ResourceLease::pending(&agent).is_ok());
    }

    #[tokio::test]
    async fn maintenance_excludes_real_probe_and_alias_until_child_reaped() {
        use crate::external_agents::spawn::{cli_command, AgentChild};
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join(if cfg!(windows) {
            "fixture.cmd"
        } else {
            "fixture"
        });
        std::fs::write(
            &binary,
            if cfg!(windows) {
                "@echo off\n:spin\ngoto spin\n"
            } else {
                "#!/bin/sh\nwhile :; do :; done\n"
            },
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let keys = runtime_resources(None, &binary);
        let mut child = AgentChild::spawn(&mut cli_command(&binary)).unwrap();
        assert!(ResourceLease::acquire(keys.clone(), true).is_err());
        child.start_kill().unwrap();
        child.wait().await.unwrap();
        drop(child);
        let lease = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match ResourceLease::acquire(keys.clone(), true) {
                    Ok(lease) => break lease,
                    Err(_) => tokio::task::yield_now().await,
                }
            }
        })
        .await
        .expect("child resources reaped");
        assert!(AgentChild::spawn(&mut cli_command(&binary)).is_err());
        let alias = root.path().join(".").join(binary.file_name().unwrap());
        assert!(AgentChild::spawn(&mut cli_command(&alias)).is_err());
        drop(lease);
        let mut retry = AgentChild::spawn(&mut cli_command(&binary)).unwrap();
        retry.start_kill().unwrap();
        retry.wait().await.unwrap();
    }
    #[test]
    fn maintenance_excludes_pending_start_and_shared_prefix() {
        let id = format!("fixture-{}", uuid::Uuid::new_v4());
        let pending = ResourceLease::pending(&id).unwrap();
        assert!(ResourceLease::acquire(vec![format!("agent:{id}")], true).is_err());
        drop(pending);
        let maintenance =
            ResourceLease::acquire(vec![format!("agent:{id}"), format!("prefix:{id}")], true)
                .unwrap();
        assert!(ResourceLease::pending(&id).is_err());
        assert!(ResourceLease::acquire(
            vec![format!("agent:other-{id}"), format!("prefix:{id}")],
            false
        )
        .is_err());
        drop(maintenance);
        assert!(ResourceLease::pending(&id).is_ok());
    }

    #[tokio::test]
    async fn pending_delete_revokes_publish_and_waits_for_scope_drop() {
        let registry = Arc::new(LiveSessionRegistry::default());
        let lease = registry.reserve("conversation").unwrap();
        let retirement = registry.retire_conversation("conversation");
        tokio::pin!(retirement);
        assert!(futures::poll!(retirement.as_mut()).is_pending());
        assert!(lease.is_cancelled());
        let (session, _receiver) = make("codex", "/project");
        let wrote = AtomicBool::new(false);
        assert!(lease
            .publish(session, || {
                wrote.store(true, Ordering::Release);
                Ok(())
            })
            .is_err());
        assert!(!wrote.load(Ordering::Acquire));
        drop(lease);
        let retirement = retirement.await.unwrap();
        assert!(registry.reserve("conversation").is_err());
        drop(retirement);
        assert!(registry.reserve("conversation").is_ok());
    }

    #[tokio::test]
    async fn failed_delete_releases_its_retirement_guard() {
        let registry = LiveSessionRegistry::default();
        let deletion: Result<(), &str> = async {
            let _retirement = registry.retire_conversation("conversation").await.unwrap();
            assert!(registry.reserve("conversation").is_err());
            assert!(registry.retire_conversation("conversation").await.is_err());
            assert!(registry.reserve("conversation").is_err());
            Err("repository delete failed")
        }
        .await;
        assert!(deletion.is_err());
        assert!(registry.reserve("conversation").is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn retirement_timeout_releases_delete_but_keeps_pending_connection() {
        let registry = LiveSessionRegistry::default();
        let lease = registry.reserve("conversation").unwrap();
        assert!(registry.retire_conversation("conversation").await.is_err());
        assert!(lease.is_cancelled());
        assert!(registry.reserve("conversation").is_err());
        drop(lease);
        assert!(registry.reserve("conversation").is_ok());
    }

    #[tokio::test]
    async fn dropped_retirement_keeps_idle_actor_owned_until_receipt() {
        let registry = LiveSessionRegistry::default();
        let (session, receiver) = make("codex", "/project");
        registry.register("conversation".into(), session);
        let mut retirement = Box::pin(registry.retire_conversation("conversation"));
        assert!(futures::poll!(retirement.as_mut()).is_pending());
        drop(retirement);
        assert!(registry.reserve("conversation").is_err());
        drop(receiver);
        tokio::task::yield_now().await;
        assert!(registry.reserve("conversation").is_ok());
    }

    #[test]
    fn stale_completion_cannot_remove_or_claim_a_replacement() {
        let registry = LiveSessionRegistry::default();
        let (old, _old_receiver) = make("codex", "/project");
        let old_control = old.control.clone();
        registry.register("conversation".into(), old);
        let (new, _new_receiver) = make("codex", "/project");
        let new_control = new.control.clone();
        registry.register("conversation".into(), new);
        assert!(!registry.remove_exact("conversation", &old_control));
        let (_, busy, reusable) = registry
            .claim_control(
                "conversation",
                "codex",
                "/project",
                &LaunchConfig::default(),
            )
            .unwrap()
            .unwrap();
        assert!(reusable);
        assert!(registry
            .claim_control(
                "conversation",
                "codex",
                "/project",
                &LaunchConfig::default()
            )
            .is_err());
        assert!(registry.is_current("conversation", &new_control));
        drop(busy);
        assert!(registry
            .claim_control(
                "conversation",
                "codex",
                "/project",
                &LaunchConfig::default()
            )
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn full_queue_shutdown_waits_for_actor_resources_to_drop() {
        struct Resource(Arc<AtomicBool>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let registry = LiveSessionRegistry::default();
        let dropped = Arc::new(AtomicBool::new(false));
        let resource = Resource(dropped.clone());
        let (sender, receiver) = mpsc::channel(1);
        sender.send(SessionCommand::Cancel).await.unwrap();
        let control = SessionControl::spawn(sender, async move {
            let _resource = resource;
            let _receiver = receiver;
            std::future::pending::<()>().await;
        });
        let (mut session, _) = make("codex", "/project");
        session.control = control;
        registry.register("conversation".into(), session);
        registry.close_all().await;
        assert!(dropped.load(Ordering::Acquire));
        assert!(registry.reserve("after-shutdown").is_err());
    }

    #[tokio::test]
    async fn dropped_run_releases_its_exact_actor() {
        let registry = LiveSessionRegistry::default();
        let lease = registry.reserve("conversation").unwrap();
        let (sender, receiver) = mpsc::channel(1);
        let control = SessionControl::spawn(sender, async move {
            let _receiver = receiver;
            std::future::pending::<()>().await;
        });
        let (mut session, _) = make("codex", "/project");
        session.control = control.clone();
        lease.publish(session, || Ok(())).unwrap();
        drop(lease);
        tokio::time::timeout(Duration::from_secs(1), control.finished())
            .await
            .unwrap();
        assert!(registry.control_any("conversation").is_none());
        assert!(registry.reserve("conversation").is_ok());
    }

    #[test]
    fn reusable_when_agent_and_cwd_match_and_actor_alive() {
        let (session, _rx) = make("codex", "/proj");
        let any = LaunchConfig::default();
        assert!(session.is_reusable("codex", "/proj", &any));
        assert!(!session.is_reusable("codex", "/other", &any));
        assert!(!session.is_reusable("claude", "/proj", &any));
    }

    #[test]
    fn not_reusable_when_actor_dropped() {
        let (session, rx) = make("codex", "/proj");
        drop(rx); // actor gone → control channel closed
        assert!(!session.is_reusable("codex", "/proj", &LaunchConfig::default()));
    }

    #[test]
    fn live_injection_uses_the_active_providers_input_support() {
        use crate::external_agents::types::CapabilitySupport::{Fallback, Native, Unavailable};

        let registry = LiveSessionRegistry::default();
        for (id, steering, follow_up) in [
            ("pi", Native, Native),
            ("codex", Native, Fallback),
            ("opencode", Unavailable, Fallback),
        ] {
            let (session, _actor) = make(id, "/project");
            let sender = session.control.clone();
            registry.register("conversation".into(), session);
            let (active_control, active_definition) =
                registry.injection_control("conversation").unwrap();
            assert!(active_control.same_channel(&sender));
            assert_eq!(active_definition.steering, steering);
            assert_eq!(active_definition.follow_up, follow_up);
        }
        assert!(registry.injection_control("another-conversation").is_none());
    }

    // ---- B1: 启动配置变更必须换进程（spec 第 8 条）----

    fn cfg(flags: &str, instructions: Option<&str>) -> LaunchConfig {
        LaunchConfig {
            flags: flags.to_string(),
            instructions: instructions.map(str::to_string),
            identity: None,
        }
    }

    /// 换模型 / 换档位 / 换 sandbox：这些是启动 flag，常驻进程只能靠重连生效。
    #[test]
    fn a_flag_change_rejects_the_existing_session() {
        let established = cfg("opus|high|bypassPermissions", Some("h1"));
        assert!(established.accepts(&cfg("opus|high|bypassPermissions", Some("h1"))));
        assert!(!established.accepts(&cfg("sonnet|high|bypassPermissions", Some("h1"))));
        assert!(!established.accepts(&cfg("opus|low|bypassPermissions", Some("h1"))));
        assert!(!established.accepts(&cfg("opus|high|plan", Some("h1"))));
    }

    /// 改系统提示 / Memory：`--append-system-prompt-file` 的内容变了，而常驻进程只在启动时
    /// 读一遍那个文件 ⇒ 不重连就静默失效，无任何可观测信号。
    #[test]
    fn changed_instructions_reject_the_existing_session() {
        let established = cfg("opus||", Some("hash-old"));
        assert!(!established.accepts(&cfg("opus||", Some("hash-new"))));
    }

    /// 斜杠命令那一轮不注入指令 ⇒ 不据此重连（否则「斜杠 → 普通消息」会来回重启两次）。
    #[test]
    fn a_slash_turn_reuses_the_session_regardless_of_instructions() {
        let established = cfg("opus||", Some("hash-old"));
        assert!(established.accepts(&cfg("opus||", None)));
        // 但 flag 变了仍要重连——斜杠命令也是在这个进程里跑的。
        assert!(!established.accepts(&cfg("sonnet||", None)));
    }

    /// 会话是被一条斜杠命令拉起来的（注册时 instructions = None）：紧跟的普通消息必须重连，
    /// 否则用户配置的系统提示与 Memory 在这个会话里永远不生效。
    #[test]
    fn a_session_launched_without_instructions_cannot_serve_a_turn_that_needs_them() {
        let established = cfg("opus||", None);
        assert!(!established.accepts(&cfg("opus||", Some("hash-1"))));
        assert!(established.accepts(&cfg("opus||", None)));
    }

    /// 两边都是默认指纹时互相接受（ACP 仍走这条）。
    #[test]
    fn default_launch_config_always_accepts() {
        assert!(LaunchConfig::default().accepts(&LaunchConfig::default()));
    }

    /// 轮内重连之后，guard 必须重新挂到**新**会话上。
    ///
    /// `reconnect_fresh` 会往注册表塞一条全新的 `LiveSession`（`busy: false`），旧 guard 指着
    /// 旧会话的 Arc。不重挂的话，恰好是刚付过一次冷启动的那条路反而会被清扫器/LRU 拆掉。
    #[test]
    fn a_new_session_needs_its_own_busy_guard() {
        let (old_session, _rx1) = make("claude", "/proj");
        let (new_session, _rx2) = make("claude", "/proj");

        let old_guard = TurnBusyGuard::new(old_session.busy.clone());
        assert!(!old_session.is_idle(Duration::from_secs(0)), "旧会话在飞");
        // 新会话没被标记 ⇒ 立刻可回收。这正是重连后的真实状态。
        assert!(
            new_session.is_idle(Duration::from_secs(0)),
            "新会话默认不 busy"
        );

        // 重挂之后新会话受保护；旧 guard 在赋值时落地，旧会话回到可回收。
        let new_guard = TurnBusyGuard::new(new_session.busy.clone());
        drop(old_guard);
        assert!(
            old_session.is_idle(Duration::from_secs(0)),
            "旧 guard 落地即清"
        );
        assert!(
            !new_session.is_idle(Duration::from_secs(0)),
            "新会话现在在飞"
        );
        drop(new_guard);
        assert!(
            new_session.is_idle(Duration::from_secs(0)),
            "guard 落地即清"
        );
    }

    #[test]
    fn a_control_guard_cannot_clear_an_existing_turn_guard() {
        let (session, _rx) = make("pi", "/proj");
        let turn_guard = TurnBusyGuard::new(session.busy.clone());
        assert!(TurnBusyGuard::try_new(session.busy.clone()).is_none());
        assert!(session.busy.load(Ordering::Acquire));
        drop(turn_guard);
        let control_guard = TurnBusyGuard::try_new(session.busy.clone()).expect("idle session");
        assert!(session.busy.load(Ordering::Acquire));
        drop(control_guard);
        assert!(!session.busy.load(Ordering::Acquire));
    }

    #[test]
    fn registry_reuse_rejects_changed_configuration_and_keeps_retirement_owned() {
        let registry = LiveSessionRegistry::default();
        let (session, receiver) = make("codex", "/project");
        registry.register("conversation".into(), session);
        let (control, busy, reusable) = registry
            .claim_control("conversation", "codex", "/other", &LaunchConfig::default())
            .unwrap()
            .unwrap();
        assert!(!reusable);
        assert!(!receiver.is_closed());
        assert!(registry.is_current("conversation", &control));
        assert!(registry.remove_exact("conversation", &control));
        drop((control, busy));
        assert!(receiver.is_closed());
    }

    #[tokio::test]
    async fn closing_a_live_session_revokes_its_chat_credential() {
        let (credential, probe) = crate::app_mcp::issue_test_chat_credential().await.unwrap();
        let registry = LiveSessionRegistry::default();
        let (mut session, _receiver) = make("claude", "/project");
        session._chat_credential = Some(credential);
        registry.register("conversation".to_string(), session);
        assert!(probe.is_active());
        let control = registry
            .claim_control(
                "conversation",
                "claude",
                "/project",
                &LaunchConfig::default(),
            )
            .unwrap();
        assert!(probe.is_active());
        drop(control);
        assert!(registry.remove("conversation"));
        assert!(!probe.is_active());

        let (credential, probe) = crate::app_mcp::issue_test_chat_credential().await.unwrap();
        let (mut session, _receiver) = make("codex", "/project");
        session._chat_credential = Some(credential);
        registry.register("idle".to_string(), session);
        assert_eq!(registry.sweep_idle(Duration::ZERO), 1);
        assert!(!probe.is_active());

        let (credential, probe) = crate::app_mcp::issue_test_chat_credential().await.unwrap();
        let (mut session, mut receiver) = make("claude", "/project");
        session._chat_credential = Some(credential);
        registry.register("shutdown".to_string(), session);
        let actor = tokio::spawn(async move {
            assert!(matches!(receiver.recv().await, Some(SessionCommand::Close)));
        });
        registry.close_all().await;
        actor.await.unwrap();
        assert!(!probe.is_active());
    }

    #[test]
    fn registry_move_rejects_destination_collision_without_losing_source() {
        let registry = LiveSessionRegistry::default();
        let (source, _source_receiver) = make("codex", "/project");
        let (destination, _destination_receiver) = make("pi", "/project");
        registry.register("source".to_string(), source);
        registry.register("destination".to_string(), destination);
        assert!(registry.move_session("source", "destination").is_err());
        assert!(registry.control_any("source").is_some());
        assert!(registry.control_any("destination").is_some());
    }

    #[test]
    fn registry_busy_session_survives_lru_and_idle_sweep() {
        let registry = LiveSessionRegistry::default();
        let (busy, _busy_receiver) = make("codex", "/project");
        registry.register("busy".to_string(), busy);
        let control = registry.control_any("busy").unwrap();
        let guard = registry
            .mark_busy_exact("busy", &control)
            .expect("registered session");
        assert_eq!(registry.sweep_idle(Duration::ZERO), 0);
        for index in 0..7 {
            let (session, _receiver) = make("pi", "/project");
            registry.register(format!("idle-{index}"), session);
        }
        assert!(registry.control_any("busy").is_some());
        drop(guard);
        assert!(registry.sweep_idle(Duration::ZERO) >= 1);
        assert!(registry.control_any("busy").is_none());
    }

    #[tokio::test]
    async fn shutdown_drains_before_awaiting_actor_close() {
        let registry = Arc::new(LiveSessionRegistry::default());
        let (session, mut receiver) = make("codex", "/project");
        registry.register("one".to_string(), session);
        let actor = tokio::spawn(async move {
            assert!(matches!(receiver.recv().await, Some(SessionCommand::Close)));
        });
        registry.close_all().await;
        actor.await.expect("actor exits");
        assert!(registry.control_any("one").is_none());
    }

    #[tokio::test]
    async fn shutdown_bounds_a_full_control_channel_even_when_actor_is_stuck() {
        let registry = LiveSessionRegistry::default();
        let (session, _receiver) = make("codex", "/project");
        session
            .control
            .send(SessionCommand::Close)
            .await
            .expect("fill the one-slot control channel");
        registry.register("stuck".to_string(), session);

        tokio::time::timeout(Duration::from_secs(2), registry.close_all())
            .await
            .expect("the per-session timeout must cover sending Close too");
        assert!(registry.control_any("stuck").is_none());
    }

    #[test]
    fn is_idle_on_age_or_closed_channel() {
        // Fresh + open → not idle.
        let (session, _rx) = make("codex", "/proj");
        assert!(!session.is_idle(Duration::from_secs(600)));

        // Aged past ttl → idle.
        let (aged, _rx2) = make("codex", "/proj");
        std::thread::sleep(Duration::from_millis(2));
        assert!(aged.is_idle(Duration::from_millis(1)));
        assert!(!aged.is_idle(Duration::from_secs(3600)));

        // Closed actor → idle regardless of age.
        let (closed, rx3) = make("codex", "/proj");
        drop(rx3);
        assert!(closed.is_idle(Duration::from_secs(3600)));
    }
}
