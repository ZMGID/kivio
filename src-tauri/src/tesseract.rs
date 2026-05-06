//! Tesseract 5 离线 OCR 集成(简化版)。
//!
//! 不 bundle 二进制 / 不管理 tessdata / 不下载语言包。
//! 用户责任:自己用 brew/choco/scoop/winget 装 tesseract,默认会带 eng 语言包。
//! Kivio 检测系统是否有 tesseract——找得到就直接 spawn,找不到就提供"一键安装"按钮
//! 调平台包管理器自动装。
//!
//! 进程模型:每次 OCR 单独 spawn,~100-300ms 启动开销,截图翻译场景完全可接受。
//!
//! PATH 兼容:macOS .app bundle 从 Finder/Dock 启动时,PATH 默认不包含 `/opt/homebrew/bin`
//! 也没 `/usr/local/bin`(Apple Silicon)。仅靠 `Command::new("tesseract")` 会探测不到
//! brew 装的可执行文件。所以这里同时检查标准 brew 路径作为 fallback。

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TesseractStatus {
  pub binary_available: bool,
  pub version: Option<String>,
  pub binary_path: Option<String>,
  /// 当前平台用于一键安装的包管理器名(brew/choco/scoop/winget),
  /// None 表示没探测到任何受支持的包管理器,前端据此 disable "一键安装"按钮。
  pub package_manager: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TesseractInstallResult {
  pub success: bool,
  pub message: String,
}

pub struct TesseractClient;

/// macOS 上 brew 装的 tesseract / brew 自身的标准位置。Apple Silicon 用 /opt/homebrew/bin,
/// Intel Mac 用 /usr/local/bin。MacPorts 用户用 /opt/local/bin。
#[cfg(target_os = "macos")]
const TESSERACT_FALLBACK_PATHS: &[&str] = &[
  "/opt/homebrew/bin/tesseract",
  "/usr/local/bin/tesseract",
  "/opt/local/bin/tesseract",
];

#[cfg(target_os = "macos")]
const BREW_FALLBACK_PATHS: &[&str] = &["/opt/homebrew/bin/brew", "/usr/local/bin/brew"];

#[cfg(not(target_os = "macos"))]
const TESSERACT_FALLBACK_PATHS: &[&str] = &[];

fn make_command(program: &str) -> std::process::Command {
  let mut cmd = std::process::Command::new(program);
  cmd.stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());
  #[cfg(target_os = "windows")]
  {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
  }
  cmd
}

fn make_async_command(program: &std::path::Path) -> tokio::process::Command {
  let mut cmd = tokio::process::Command::new(program);
  cmd.stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());
  #[cfg(target_os = "windows")]
  {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
  }
  cmd
}

/// 跑 `<program> --version`,成功返回 true。
fn version_probe_succeeds(program: &str) -> bool {
  make_command(program)
    .arg("--version")
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false)
}

/// 解析 tesseract 路径:先试 PATH,再试平台标准 fallback 位置。
fn resolve_tesseract_path() -> Option<PathBuf> {
  if version_probe_succeeds("tesseract") {
    return Some(PathBuf::from("tesseract"));
  }
  TESSERACT_FALLBACK_PATHS
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

#[cfg(target_os = "macos")]
fn resolve_brew_path() -> Option<PathBuf> {
  if version_probe_succeeds("brew") {
    return Some(PathBuf::from("brew"));
  }
  BREW_FALLBACK_PATHS
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

/// Windows 包管理器探测:winget(系统自带,优先) > scoop(用户态,无 UAC) > choco(需 UAC)
#[cfg(target_os = "windows")]
fn detect_windows_pm() -> Option<&'static str> {
  for pm in &["winget", "scoop", "choco"] {
    if version_probe_succeeds(pm) {
      return Some(pm);
    }
  }
  None
}

/// 截取错误输出片段:stderr 优先,空就退到 stdout,前 500 字符够看了。
fn extract_error_snippet(output: &std::process::Output) -> String {
  let stderr = String::from_utf8_lossy(&output.stderr);
  let stdout = String::from_utf8_lossy(&output.stdout);
  let source = if !stderr.trim().is_empty() {
    stderr
  } else {
    stdout
  };
  source.trim().chars().take(500).collect()
}

/// 跑包管理器命令并把退出码转成 `TesseractInstallResult`。失败时取错误输出片段当 message。
async fn run_install_command(
  pm_path: &std::path::Path,
  args: &[&str],
  pm_name: &str,
) -> TesseractInstallResult {
  let mut cmd = make_async_command(pm_path);
  cmd.args(args);
  match cmd.output().await {
    Err(e) => TesseractInstallResult {
      success: false,
      message: format!("启动 {pm_name} 失败: {e}"),
    },
    Ok(output) if output.status.success() => TesseractInstallResult {
      success: true,
      message: format!("Tesseract 通过 {pm_name} 安装完成"),
    },
    Ok(output) => TesseractInstallResult {
      success: false,
      message: format!("{pm_name} 安装 tesseract 失败: {}", extract_error_snippet(&output)),
    },
  }
}

impl TesseractClient {
  pub fn new() -> Arc<Self> {
    Arc::new(Self)
  }

  pub fn status(&self) -> TesseractStatus {
    let Some(path) = resolve_tesseract_path() else {
      return TesseractStatus {
        binary_available: false,
        version: None,
        binary_path: None,
        package_manager: detect_package_manager(),
      };
    };

    let version = make_command(path.to_string_lossy().as_ref())
      .arg("--version")
      .output()
      .ok()
      .filter(|o| o.status.success())
      .and_then(|o| {
        let stderr = String::from_utf8_lossy(&o.stderr);
        let stdout = String::from_utf8_lossy(&o.stdout);
        let combined = if !stderr.trim().is_empty() {
          stderr.into_owned()
        } else {
          stdout.into_owned()
        };
        combined
          .lines()
          .next()
          .map(|s| s.trim().to_string())
          .filter(|s| !s.is_empty())
      });

    TesseractStatus {
      binary_available: true,
      version,
      binary_path: Some(path.to_string_lossy().into_owned()),
      package_manager: detect_package_manager(),
    }
  }

  pub async fn ocr_image(
    self: &Arc<Self>,
    image_path: &std::path::Path,
    lang: &str,
  ) -> Result<String, String> {
    let binary = resolve_tesseract_path()
      .ok_or_else(|| "tesseract_binary_missing".to_string())?;

    let output = make_async_command(&binary)
      .arg(image_path)
      .arg("stdout")
      .arg("-l")
      .arg(lang)
      .output()
      .await
      .map_err(|e| format!("启动 tesseract 失败: {e}"))?;

    if !output.status.success() {
      return Err({
        let snippet = extract_error_snippet(&output);
        if snippet.is_empty() {
          format!("tesseract 退出码 {:?}", output.status.code())
        } else {
          snippet
        }
      });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
      return Err("Tesseract 未识别到文字".to_string());
    }
    Ok(stdout)
  }

  /// 调平台包管理器一键安装 tesseract。阻塞到 install 完成或失败。
  /// macOS: `brew install tesseract` (无 sudo,1-3min 取决于 bottle 是否可用)
  /// Windows: 优先 winget(系统自带 + silent),退到 scoop / choco
  pub async fn install(self: &Arc<Self>) -> TesseractInstallResult {
    #[cfg(target_os = "macos")]
    {
      let Some(brew) = resolve_brew_path() else {
        return TesseractInstallResult {
          success: false,
          message: "未找到 Homebrew。请先访问 https://brew.sh 安装 Homebrew,再来一键装 Tesseract。".to_string(),
        };
      };
      run_install_command(&brew, &["install", "tesseract"], "brew").await
    }
    #[cfg(target_os = "windows")]
    {
      let Some(pm) = detect_windows_pm() else {
        return TesseractInstallResult {
          success: false,
          message: "未检测到 winget/scoop/choco。请先安装其中之一,或手动下载 Tesseract:https://github.com/UB-Mannheim/tesseract/wiki".to_string(),
        };
      };
      let pm_path = PathBuf::from(pm);
      let args: &[&str] = match pm {
        "winget" => &[
          "install",
          "--id",
          "UB-Mannheim.TesseractOCR",
          "-e",
          "--accept-source-agreements",
          "--accept-package-agreements",
          "--silent",
        ],
        "scoop" => &["install", "tesseract"],
        "choco" => &["install", "tesseract", "-y"],
        _ => unreachable!(),
      };
      run_install_command(&pm_path, args, pm).await
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
      TesseractInstallResult {
        success: false,
        message: "当前平台不支持一键安装。请用系统包管理器手动安装 tesseract。".to_string(),
      }
    }
  }
}

fn detect_package_manager() -> Option<String> {
  #[cfg(target_os = "macos")]
  return resolve_brew_path().map(|_| "brew".to_string());
  #[cfg(target_os = "windows")]
  return detect_windows_pm().map(|s| s.to_string());
  #[cfg(not(any(target_os = "macos", target_os = "windows")))]
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn status_does_not_panic_when_binary_missing() {
    let client = TesseractClient::new();
    let s = client.status();
    if !s.binary_available {
      assert!(s.version.is_none());
      assert!(s.binary_path.is_none());
    }
  }
}
