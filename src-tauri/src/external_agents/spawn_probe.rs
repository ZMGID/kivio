/// 探活单个候选的超时。只是启动一次 `--version`，正常在几十毫秒内返回；
/// 给到 2s 是为了容忍冷启动（Node/Bun 包装脚本首次加载）。
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// 已探活通过的候选路径缓存。
///
/// **为什么必须有**：探活要起一次 `--version` 子进程，实测本机开销差异极大——
/// grok 6.6ms、claude 40ms，但 kimi 328ms、pi 302ms（Node 包装脚本冷启动）。
/// 而 `resolve_binary` 是**回复热路径**上唯一允许的前置调用，`run.rs` 明确写着
/// 「把第 2+ 轮的前置开销压到 <500ms」。不缓存的话 kimi 单这一步就吃掉 538ms，
/// 直接击穿该预算。
///
/// **失效策略**：key 用「路径 + mtime + size」。用户换版本、重装、切版本管理器
/// 都会改变其中之一 ⇒ 自动失效重探；不需要 TTL，也就不会有「刚装好却要等 N 秒」
/// 或「删了还认为在」的窗口。文件读不到 metadata 时不写缓存（下次重探）。
///
/// **顺带存下版本号**：探活本来就跑了 `--version`，此前把 stdout 丢进 `/dev/null`。
/// 把它接住存进同一条缓存，列表阶段就能做版本门控（如「当前 CLI 太老，不提供新模型」）
/// 而**不新起任何进程** —— spec 第 9 条要求回复热路径零探测，另起一次 `--version`
/// 会违反它。缓存 key 已含文件身份，换版本自然失效。
static PROBE_CACHE: std::sync::LazyLock<std::sync::Mutex<HashMap<String, ProbeOutcome>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// 一次探活的结果：能否启动 + `--version` 首行（拿不到则 `None`）。
#[derive(Debug, Clone)]
struct ProbeOutcome {
    executable: bool,
    version: Option<String>,
}

/// 缓存 key：把文件身份（mtime + size）编进去，文件一变就自然 miss。
fn probe_cache_key(path: &Path) -> Option<String> {
    // `\\wsl$\...` 走 9P，metadata 又慢又会把发行版唤醒；用路径本身当身份即可。
    if crate::external_agents::wsl::is_wsl_target(path) {
        return Some(format!("wsl|{}", path.display()));
    }
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(format!("{}|{mtime}|{}", path.display(), meta.len()))
}

/// 解析 CLI 二进制：遍历 `bin` + `fallback_bins`，对每个名字取 PATH 上的**全部**同名候选，
/// 逐个探活，返回第一个真正能启动的。
///
/// 为什么不能只取 `which` 的第一行：PATH 上常有同名但坏掉的 shim（版本管理器切换后的残留、
/// 断掉的 symlink、丢了执行权限的文件）。选中它不会立刻失败，而是**等到真正跑轮次时**才炸，
/// 用户看到的是运行时报错而不是「未安装」，排查成本很高。
///
/// **热路径开销**（spec 第 9 条 + `run.rs` 的 <500ms 预算）：首次每个候选多一次
/// `--version`（实测 6.6ms ~ 328ms 不等），之后走 `PROBE_CACHE` ⇒ 稳态只剩
/// `which -a` 的 ~2-3ms，与改动前基本持平。
pub async fn resolve_binary_configured(def: &RuntimeAgentDef, custom_path: Option<PathBuf>) -> Option<PathBuf> {
    resolve_probe(def.into(), custom_path).await
}
pub async fn resolve_probe(def: crate::external_agents::management_catalog::Definition, custom_path: Option<PathBuf>) -> Option<PathBuf> {
    // 用户在设置页指定了路径就**只认它**：静默回退到 PATH 会让「我明明指了这个二进制」
    // 与实际跑的东西对不上，比诚实报「未安装」更难查。
    if let Some(path) = custom_path {
        let path = crate::external_agents::wsl::normalize_custom_path(path).await;
        return probe_executable_cached(&path, def.version_args)
            .await
            .then_some(path);
    }
    for candidate in std::iter::once(def.bin).chain(def.fallback_bins.iter().copied()) {
        for path in which_all(candidate).await {
            if probe_executable_cached(&path, def.version_args).await {
                return Some(path);
            }
        }
    }
    // The desktop process may still have the PATH from before the official agy install.
    if def.id == "antigravity" {
        let path = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|p| p.join("agy/bin/agy.exe"))
        } else {
            directories::UserDirs::new().map(|d| d.home_dir().join(".local/bin/agy"))
        };
        if let Some(path) = path.filter(|p| p.is_file()) {
            if probe_executable_cached(&path, def.version_args).await {
                return Some(path);
            }
        }
    }
    // Windows PATH 上没有能拉起来的 Win32 候选时，再看 WSL 里的 Linux 安装。
    // 不插到 PATH 候选前面：本机已经装了 Windows 版 CLI 的人不能被 WSL 里同名二进制抢走。
    for candidate in std::iter::once(def.bin).chain(def.fallback_bins.iter().copied()) {
        if let Some(path) = crate::external_agents::wsl::linux_bin_unc(candidate).await {
            if probe_executable_cached(&path, def.version_args).await {
                return Some(path);
            }
        }
    }
    None
}

/// `probe_executable` + `PROBE_CACHE`（key 含 mtime/size，换版本自动失效）。
async fn probe_executable_cached(path: &Path, version_args: &[&str]) -> bool {
    let key = probe_cache_key(path);
    if let Some(key) = key.as_deref() {
        if let Some(cached) = PROBE_CACHE.lock().ok().and_then(|c| c.get(key).cloned()) {
            return cached.executable;
        }
    }
    let (ok, version) = probe_executable(path, version_args).await;
    if let (Some(key), Ok(mut cache)) = (key, PROBE_CACHE.lock()) {
        cache.insert(
            key,
            ProbeOutcome {
                executable: ok,
                version,
            },
        );
    }
    ok
}

/// 探活时顺手记下的 CLI 版本号（`--version` 首行），**不起任何新进程**。
///
/// 由 `resolve_binary` 的探活填充；缓存 key 含文件身份（路径 + mtime + size），
/// 换版本/重装自动失效。返回 `None` 的情况：这个路径还没被探活过、`--version`
/// 没有可用输出、或者拿不到文件 metadata（不写缓存那一路）。
///
/// 调用方拿 `None` 时**不得**因此禁用功能（spec 第 9 条：不许为补版本号再起进程）；
/// 语义应是「未知 ⇒ 不做版本门控」。
pub fn cached_cli_version(path: &Path) -> Option<String> {
    let key = probe_cache_key(path)?;
    PROBE_CACHE
        .lock()
        .ok()?
        .get(&key)
        .and_then(|entry| entry.version.clone())
}

/// 安装后 `--version` 从失败变成能跑，但 shim 的 mtime/size 往往不变，探活缓存
/// 会继续返回「能启动、无版本」。装完立刻丢掉这条，下次探测才会重新跑。
pub fn invalidate_probe_cache(path: &Path) {
    let prefix = format!("{}|", path.display());
    let wsl_key = format!("wsl|{}", path.display());
    if let Ok(mut cache) = PROBE_CACHE.lock() {
        cache.retain(|key, _| !key.starts_with(&prefix) && key.as_str() != wsl_key);
    }
}

pub fn clear_probe_cache() {
    if let Ok(mut cache) = PROBE_CACHE.lock() {
        cache.clear();
    }
}

/// 一个候选是否**可执行**（不是「是否可用」）。
///
/// 判据刻意宽松——只要进程**起来了**就算存在，不看退出码：
/// 一个装了但**没登录**的 CLI，`--version` 完全可能非零退出（本机实测
/// `printf 'exit 1' > x` 这类脚本 spawn 成功、exit=1）。只认零退出码会把这类 CLI
/// 误判成「未安装」，比原来的 bug 更糟。
///
/// 真正的「不存在」只有 spawn 阶段的失败。本机实测（macOS，`/tmp/probe_t` 造的样本）：
/// - 丢了执行权限     → `EACCES`
/// - 空文件但挂了执行位 → **spawn 成功**（内核回退 /bin/sh，exit 0），视为存在
/// - 断掉的 symlink / 文件不存在 → `ENOENT`
/// - 装了但未登录（exit 1）→ **spawn 成功**，视为存在
///
/// 超时也算存在：进程都起来了，说明文件确实是可执行的，只是这个 CLI 的 `--version` 慢。
///
/// 返回 `(能否启动, --version 首行)`。第二项**只是搭便车**：这次子进程本来就要跑，
/// 顺手把 stdout 接住给版本门控用（此前是 `Stdio::null()` 直接丢掉）。它拿不到时一律
/// `None`，绝不影响第一项的判定 —— 未登录的 CLI 完全可能既非零退出又没有版本输出。
async fn probe_executable(path: &Path, version_args: &[&str]) -> (bool, Option<String>) {
    let mut command = cli_probe_command(path);
    command
        .args(version_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .no_console_window()
        .kill_on_drop(true);
    let child = match command.spawn().map(AgentChild::from) {
        Ok(child) => child,
        // spawn 失败 = 这个候选不是个能跑的可执行文件（EACCES / ENOEXEC / ENOENT）。
        Err(_) => return (false, None),
    };
    // 超时时这个 future 被丢弃 ⇒ 连带丢弃 Child ⇒ `kill_on_drop(true)` 收尾，
    // 与改动前的显式 `start_kill()` 等价。
    let is_wsl = crate::external_agents::wsl::is_wsl_target(path);
    match timeout(PROBE_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            if is_wsl && crate::external_agents::wsl::wsl_stdout_is_system_error(&output.stdout) {
                return (false, None);
            }
            let text = if is_wsl {
                crate::external_agents::wsl::decode_wsl_output(&output.stdout)
            } else {
                String::from_utf8_lossy(&output.stdout).into_owned()
            };
            (true, first_version_line(&text))
        }
        // Win32：进程能起来就算在。WSL：真正启动的是 wsl.exe 垫片——没装组件时它
        // 会卡在 MSI 几十秒（本机 61s / REGDB_E_CLASSNOTREG），超时绝不等于 CLI 在。
        Ok(Err(_)) => (!is_wsl, None),
        Err(_) => (!is_wsl, None),
    }
}

/// `--version` 输出里的版本行：首个非空行，裁到 `VERSION_LINE_CAP` 字符。
///
/// 刻意不解析结构（不同 CLI 形态差异极大：claude 是 `2.1.220 (Claude Code)`、
/// codex 是 `codex-cli 0.145.0`）——比较语义留给使用方，这里只负责「原样记一行」。
fn first_version_line(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| tail_chars_head(line, VERSION_LINE_CAP))
}

const VERSION_LINE_CAP: usize = 200;

/// 取前 `max_chars` 个字符（char 边界安全）。与 `tail_chars` 对称，用于给版本行封顶：
/// 坏 shim 可能把一整篇日志打在一行里。
fn tail_chars_head(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

/// PATH 上某个名字的**全部**同名候选，按 PATH 顺序。
/// unix 用 `which -a`；Windows 的 `where` 本身就多行输出。
async fn which_all(name: &str) -> Vec<PathBuf> {
    let mut command = Command::new(if cfg!(windows) { "where" } else { "which" });
    if !cfg!(windows) {
        command.arg("-a");
    }
    command.env("PATH", crate::path_env::command_path());
    let output = match command.arg(name).no_console_window().output().await {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| seen.insert(line.to_string()))
        .map(PathBuf::from)
        .collect()
}
