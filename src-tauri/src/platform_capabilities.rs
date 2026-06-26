use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    Macos,
    Windows,
    Linux,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopSession {
    X11,
    Wayland,
    Headless,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Supported,
    Unsupported,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityInfo {
    pub status: CapabilityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub smoke_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformCapabilities {
    pub platform: PlatformKind,
    pub session_type: DesktopSession,
    pub window_capture: CapabilityInfo,
    pub region_capture: CapabilityInfo,
    pub system_ocr: CapabilityInfo,
    pub rapid_ocr: CapabilityInfo,
    pub global_shortcuts: CapabilityInfo,
    pub tray: CapabilityInfo,
    pub autostart: CapabilityInfo,
    pub transparent_overlay: CapabilityInfo,
}

pub fn current_platform_capabilities() -> PlatformCapabilities {
    platform_capabilities_for(current_platform(), current_desktop_session())
}

fn current_platform() -> PlatformKind {
    if cfg!(target_os = "macos") {
        PlatformKind::Macos
    } else if cfg!(target_os = "windows") {
        PlatformKind::Windows
    } else if cfg!(target_os = "linux") {
        PlatformKind::Linux
    } else {
        PlatformKind::Other
    }
}

fn current_desktop_session() -> DesktopSession {
    session_type_from_env(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("DISPLAY").ok().as_deref(),
    )
}

fn session_type_from_env(
    xdg_session_type: Option<&str>,
    wayland_display: Option<&str>,
    display: Option<&str>,
) -> DesktopSession {
    match xdg_session_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("x11") => DesktopSession::X11,
        Some("wayland") => DesktopSession::Wayland,
        Some("tty") => DesktopSession::Headless,
        Some(_) => DesktopSession::Unknown,
        None => {
            if wayland_display.is_some_and(|value| !value.trim().is_empty()) {
                DesktopSession::Wayland
            } else if display.is_some_and(|value| !value.trim().is_empty()) {
                DesktopSession::X11
            } else {
                DesktopSession::Headless
            }
        }
    }
}

pub fn platform_capabilities_for(
    platform: PlatformKind,
    session_type: DesktopSession,
) -> PlatformCapabilities {
    match platform {
        PlatformKind::Macos => PlatformCapabilities {
            platform,
            session_type,
            window_capture: supported("ScreenCaptureKit window capture", true),
            region_capture: supported("ScreenCaptureKit region capture", true),
            system_ocr: supported("Apple Vision OCR", true),
            rapid_ocr: supported("RapidOCR local ONNX runtime", true),
            global_shortcuts: supported("Tauri global shortcut plugin", true),
            tray: supported("Tauri tray icon", true),
            autostart: supported("Tauri autostart plugin", true),
            transparent_overlay: supported("native overlay panel", true),
        },
        PlatformKind::Windows => PlatformCapabilities {
            platform,
            session_type,
            window_capture: unsupported("window capture is currently macOS-only"),
            region_capture: supported("xcap region capture", true),
            system_ocr: supported("Windows.Media.Ocr", true),
            rapid_ocr: supported("RapidOCR local ONNX runtime", true),
            global_shortcuts: supported("Tauri global shortcut plugin", true),
            tray: supported("Tauri tray icon", true),
            autostart: supported("Tauri autostart plugin", true),
            transparent_overlay: degraded("transparent window behavior needs desktop smoke"),
        },
        PlatformKind::Linux => linux_capabilities(session_type),
        PlatformKind::Other => PlatformCapabilities {
            platform,
            session_type,
            window_capture: unsupported("unsupported platform"),
            region_capture: unsupported("unsupported platform"),
            system_ocr: unsupported("unsupported platform"),
            rapid_ocr: unsupported("unsupported platform"),
            global_shortcuts: unsupported("unsupported platform"),
            tray: unsupported("unsupported platform"),
            autostart: unsupported("unsupported platform"),
            transparent_overlay: unsupported("unsupported platform"),
        },
    }
}

fn linux_capabilities(session_type: DesktopSession) -> PlatformCapabilities {
    // Linux 先暴露真实边界：可编译/可打包不等于桌面能力已可用。
    let desktop_reason = match session_type {
        DesktopSession::X11 => "Linux X11 desktop behavior requires smoke test",
        DesktopSession::Wayland => "Linux Wayland desktop behavior is compositor-restricted",
        DesktopSession::Headless => "no graphical desktop session detected",
        DesktopSession::Unknown => "unknown Linux desktop session",
    };

    PlatformCapabilities {
        platform: PlatformKind::Linux,
        session_type,
        window_capture: unsupported("window capture is not implemented on Linux"),
        region_capture: unsupported("region capture is not implemented on Linux"),
        system_ocr: unsupported("system OCR is not available on Linux"),
        rapid_ocr: supported("RapidOCR local ONNX runtime", true),
        global_shortcuts: degraded(desktop_reason),
        tray: degraded(desktop_reason),
        autostart: degraded("desktop-file autostart requires smoke test"),
        transparent_overlay: degraded(desktop_reason),
    }
}

fn supported(reason: &str, smoke_required: bool) -> CapabilityInfo {
    CapabilityInfo {
        status: CapabilityStatus::Supported,
        reason: Some(reason.to_string()),
        smoke_required,
    }
}

fn unsupported(reason: &str) -> CapabilityInfo {
    CapabilityInfo {
        status: CapabilityStatus::Unsupported,
        reason: Some(reason.to_string()),
        smoke_required: false,
    }
}

fn degraded(reason: &str) -> CapabilityInfo {
    CapabilityInfo {
        status: CapabilityStatus::Degraded,
        reason: Some(reason.to_string()),
        smoke_required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_desktop_session_from_xdg_or_display_fallback() {
        assert_eq!(
            session_type_from_env(Some("x11"), Some("wayland-0"), Some(":1")),
            DesktopSession::X11
        );
        assert_eq!(
            session_type_from_env(Some("wayland"), None, Some(":1")),
            DesktopSession::Wayland
        );
        assert_eq!(
            session_type_from_env(None, Some("wayland-0"), None),
            DesktopSession::Wayland
        );
        assert_eq!(
            session_type_from_env(None, None, Some(":1")),
            DesktopSession::X11
        );
        assert_eq!(
            session_type_from_env(None, None, None),
            DesktopSession::Headless
        );
    }

    #[test]
    fn linux_contract_keeps_capture_explicitly_unsupported_and_rapidocr_supported() {
        let caps = platform_capabilities_for(PlatformKind::Linux, DesktopSession::X11);
        assert_eq!(caps.window_capture.status, CapabilityStatus::Unsupported);
        assert_eq!(caps.region_capture.status, CapabilityStatus::Unsupported);
        assert_eq!(caps.system_ocr.status, CapabilityStatus::Unsupported);
        assert_eq!(caps.rapid_ocr.status, CapabilityStatus::Supported);
        assert!(caps.rapid_ocr.smoke_required);
        assert_eq!(caps.global_shortcuts.status, CapabilityStatus::Degraded);
    }

    #[test]
    fn windows_contract_does_not_claim_window_capture() {
        let caps = platform_capabilities_for(PlatformKind::Windows, DesktopSession::Unknown);
        assert_eq!(caps.window_capture.status, CapabilityStatus::Unsupported);
        assert_eq!(caps.region_capture.status, CapabilityStatus::Supported);
        assert_eq!(caps.system_ocr.status, CapabilityStatus::Supported);
    }
}
