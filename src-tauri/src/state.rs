use std::{
  collections::{HashMap, HashSet},
  path::PathBuf,
  sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc, Mutex, RwLock,
  },
  time::{Duration, Instant},
};

use reqwest::Client;

use crate::apple_intelligence::AppleIntelligenceClient;
use crate::settings::Settings;

/// 应用全局状态
/// 使用 RwLock 保护 settings，允许多读单写；
/// Mutex 用于 explain_images 等需要独占访问的数据；
/// AtomicBool 标记 lens 是否正在进行，防止并发热键触发。
pub struct AppState {
  pub settings: RwLock<Settings>,
  pub explain_images: Mutex<HashMap<String, PathBuf>>,
  pub current_explain_image_id: Mutex<Option<String>>,
  pub lens_busy: AtomicBool,
  /// 流式取消代号：每开新的流就 +1，跑流的循环检测到代号变了就立即结束。
  pub explain_stream_generation: AtomicU64,
  /// Lens 启动前抓到的选中文本：放在这里等前端 enterSelect 来取走。
  /// 取一次清一次（take 语义）。无选中 / 取过 / translate 模式 = None。
  pub pending_selection: Mutex<Option<String>>,
  /// API Key 多 key failover 状态：(provider_id, key_idx) → 冷却到期时间。
  /// 某个 key 触发 quota/rate-limit/auth 失败时进入冷却，KEY_COOLDOWN 秒内不再选用。
  pub key_cooldowns: Mutex<HashMap<(String, usize), Instant>>,
  /// 每个 provider 当前活跃 key idx：上一次成功的 key 优先继续用。
  pub active_key_idx: Mutex<HashMap<String, usize>>,
  pub http: Client,
  /// Apple Intelligence sidecar 客户端。app 启动时 spawn 一次，所有走 Apple provider 的请求复用。
  /// 不可用时 client.available()=false，路由层立即报错。
  pub apple_intelligence: Arc<AppleIntelligenceClient>,
}

/// 单个 key 触发 failover 后的冷却时长。
pub const KEY_COOLDOWN: Duration = Duration::from_secs(60);

impl AppState {
  /// 安全读取设置（锁中毒时返回内部数据，不 panic）
  pub fn settings_read(&self) -> std::sync::RwLockReadGuard<'_, Settings> {
    self.settings.read().unwrap_or_else(|e| e.into_inner())
  }
  /// 安全写入设置（锁中毒时返回内部数据，不 panic）
  pub fn settings_write(&self) -> std::sync::RwLockWriteGuard<'_, Settings> {
    self.settings.write().unwrap_or_else(|e| e.into_inner())
  }
  /// 安全获取解释图片映射锁
  pub fn images_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, PathBuf>> {
    self.explain_images.lock().unwrap_or_else(|e| e.into_inner())
  }
  /// 安全获取当前解释图片 ID 锁
  pub fn current_id_lock(&self) -> std::sync::MutexGuard<'_, Option<String>> {
    self.current_explain_image_id.lock().unwrap_or_else(|e| e.into_inner())
  }

  /// 选择一个可用的 API Key 索引：
  /// 优先返回 active_key_idx 记录的 idx；若它在冷却中或已被试过，退回到下一个非冷却 idx；
  /// 全部冷却或 tried 已穷举时返回 None（调用方决定是否报错）。
  pub fn pick_active_key(
    &self,
    provider_id: &str,
    total: usize,
    tried: &HashSet<usize>,
  ) -> Option<usize> {
    if total == 0 {
      return None;
    }
    let now = Instant::now();
    let cooldowns = self
      .key_cooldowns
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    let active = self
      .active_key_idx
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .get(provider_id)
      .copied()
      .unwrap_or(0)
      .min(total.saturating_sub(1));

    let in_cooldown = |idx: usize| {
      cooldowns
        .get(&(provider_id.to_string(), idx))
        .map(|until| *until > now)
        .unwrap_or(false)
    };

    // 1) 优先 active idx（未试过 + 未冷却）
    if !tried.contains(&active) && !in_cooldown(active) {
      return Some(active);
    }
    // 2) 从 active+1 开始环绕扫描
    for offset in 1..total {
      let idx = (active + offset) % total;
      if !tried.contains(&idx) && !in_cooldown(idx) {
        return Some(idx);
      }
    }
    // 3) 全部冷却 → 兜底找一个未试过的（无视冷却，避免完全无 key 可用）
    for offset in 0..total {
      let idx = (active + offset) % total;
      if !tried.contains(&idx) {
        return Some(idx);
      }
    }
    None
  }

  /// 标记某个 key 失败：进入冷却 + 不变更 active_key_idx
  pub fn mark_key_failed(&self, provider_id: &str, idx: usize) {
    let mut cooldowns = self
      .key_cooldowns
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    cooldowns.insert((provider_id.to_string(), idx), Instant::now() + KEY_COOLDOWN);
  }

  /// 标记某个 key 成功：清除该 idx 的冷却 + 设为 active
  pub fn mark_key_ok(&self, provider_id: &str, idx: usize) {
    let mut cooldowns = self
      .key_cooldowns
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    cooldowns.remove(&(provider_id.to_string(), idx));
    drop(cooldowns);
    let mut active = self
      .active_key_idx
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    active.insert(provider_id.to_string(), idx);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use reqwest::Client;

  /// 构造一个最小可用的 AppState 用于测试 cooldown / pick_active_key 逻辑。
  /// 不涉及网络，Client::new() 即可（不会发请求）。
  fn test_state() -> AppState {
    AppState {
      settings: RwLock::new(Settings::default()),
      explain_images: Mutex::new(HashMap::new()),
      current_explain_image_id: Mutex::new(None),
      lens_busy: AtomicBool::new(false),
      explain_stream_generation: AtomicU64::new(0),
      pending_selection: Mutex::new(None),
      key_cooldowns: Mutex::new(HashMap::new()),
      active_key_idx: Mutex::new(HashMap::new()),
      http: Client::new(),
      apple_intelligence: AppleIntelligenceClient::disabled(),
    }
  }

  #[test]
  fn pick_active_key_returns_none_when_total_zero() {
    let st = test_state();
    assert_eq!(st.pick_active_key("p", 0, &HashSet::new()), None);
  }

  #[test]
  fn pick_active_key_starts_at_idx_zero_when_no_active_recorded() {
    let st = test_state();
    assert_eq!(st.pick_active_key("p", 3, &HashSet::new()), Some(0));
  }

  #[test]
  fn pick_active_key_prefers_last_known_good_idx() {
    let st = test_state();
    st.mark_key_ok("p", 2);
    assert_eq!(st.pick_active_key("p", 3, &HashSet::new()), Some(2));
  }

  #[test]
  fn pick_active_key_skips_tried_indices() {
    let st = test_state();
    let mut tried = HashSet::new();
    tried.insert(0);
    // active 是 0（没记录过 ok），但 0 已 tried → 应返回 1（环绕扫描下一个）
    assert_eq!(st.pick_active_key("p", 3, &tried), Some(1));
  }

  #[test]
  fn pick_active_key_skips_cooled_down_indices() {
    let st = test_state();
    st.mark_key_failed("p", 0); // 0 进入冷却
    // active 默认 0；0 在冷却 → 应跳到 1
    assert_eq!(st.pick_active_key("p", 3, &HashSet::new()), Some(1));
  }

  #[test]
  fn pick_active_key_falls_back_to_cooled_when_all_cooled_but_untried() {
    let st = test_state();
    // 三个 key 全部冷却
    st.mark_key_failed("p", 0);
    st.mark_key_failed("p", 1);
    st.mark_key_failed("p", 2);
    // 但都没试过 → 兜底返回某个 idx（不是 None），让用户至少有 key 用
    assert!(st.pick_active_key("p", 3, &HashSet::new()).is_some());
  }

  #[test]
  fn pick_active_key_returns_none_when_all_tried() {
    let st = test_state();
    let mut tried = HashSet::new();
    tried.insert(0);
    tried.insert(1);
    tried.insert(2);
    assert_eq!(st.pick_active_key("p", 3, &tried), None);
  }

  #[test]
  fn mark_key_ok_clears_cooldown() {
    let st = test_state();
    st.mark_key_failed("p", 0);
    // 此时 0 在冷却
    assert_ne!(st.pick_active_key("p", 2, &HashSet::new()), Some(0));
    // 标记成功后冷却被清除 + active 设为 0
    st.mark_key_ok("p", 0);
    assert_eq!(st.pick_active_key("p", 2, &HashSet::new()), Some(0));
  }

  #[test]
  fn cooldowns_are_per_provider() {
    let st = test_state();
    st.mark_key_failed("p1", 0);
    // p1 idx 0 冷却不影响 p2 idx 0
    assert_eq!(st.pick_active_key("p2", 2, &HashSet::new()), Some(0));
  }

  #[test]
  fn pick_active_key_handles_active_idx_out_of_bounds() {
    // 用户原来有 5 个 key，active=4；删了 3 个，现在 total=2
    // pick_active_key 应该 clamp 到 total-1，不 panic
    let st = test_state();
    st.mark_key_ok("p", 4);
    let result = st.pick_active_key("p", 2, &HashSet::new());
    assert!(result.is_some());
    assert!(result.unwrap() < 2);
  }
}
