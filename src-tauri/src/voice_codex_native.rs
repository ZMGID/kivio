use std::{
    ffi::OsStr,
    io::{Error, ErrorKind, Read},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

use crate::proc::NoConsoleWindow;

const MAX_MANIFEST: usize = 1024 * 1024;
const MAX_FRAME: usize = 128 * 1024;
const MAX_SDP: usize = 64 * 1024;
const MAX_COMMIT: usize = 128;
const UNAVAILABLE: &str = "unavailable: Codex native voice runtime is unavailable";
const CODEX: &str = if cfg!(windows) { "codex.exe" } else { "codex" };
const HELPER: &str = if cfg!(windows) {
    "codex-voice-host.exe"
} else {
    "codex-voice-host"
};

// Only the release tuple and prerelease status affect the two version gates.
fn version_at_least(version: &str, minimum: (u64, u64, u64)) -> bool {
    fn identifiers(value: &str, prerelease: bool) -> bool {
        value.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !(prerelease
                    && part.len() > 1
                    && part.starts_with('0')
                    && part.bytes().all(|b| b.is_ascii_digit()))
        })
    }
    fn number(value: &str) -> Option<u64> {
        if value.is_empty()
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        value.parse().ok()
    }
    let (version, build) = match version.split_once('+') {
        Some((version, build)) => (version, Some(build)),
        None => (version, None),
    };
    if build.is_some_and(|value| !identifiers(value, false)) {
        return false;
    }
    let (core, prerelease) = match version.split_once('-') {
        Some((core, pre)) if identifiers(pre, true) => (core, true),
        Some(_) => return false,
        None => (version, false),
    };
    let mut parts = core.split('.');
    let Some((major, minor, patch)) = parts
        .next()
        .and_then(number)
        .zip(parts.next().and_then(number))
        .zip(parts.next().and_then(number))
        .map(|((major, minor), patch)| (major, minor, patch))
    else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    (major, minor, patch) > minimum || ((major, minor, patch) == minimum && !prerelease)
}

fn read_manifest(path: &Path) -> std::io::Result<Value> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take((MAX_MANIFEST + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_MANIFEST {
        return Err(Error::new(ErrorKind::InvalidData, "manifest too large"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| Error::new(ErrorKind::InvalidData, "invalid manifest"))
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
}

fn target_matches(target: &str) -> bool {
    let Some((arch, os)) = target.split_once('-') else {
        return false;
    };
    arch == std::env::consts::ARCH
        && match std::env::consts::OS {
            "windows" => os == "pc-windows-msvc",
            "macos" => os == "apple-darwin",
            "linux" => os == "unknown-linux-musl" || os == "unknown-linux-gnu",
            _ => false,
        }
}

fn installed(binary: &Path) -> Option<(PathBuf, PathBuf)> {
    let binary = binary.canonicalize().ok()?;
    if binary.file_name()? != OsStr::new(CODEX)
        || binary.parent()?.file_name()? != OsStr::new("bin")
        || !executable(&binary)
    {
        return None;
    }
    let root = binary.parent()?.parent()?;
    let manifest = read_manifest(&root.join("codex-package.json")).ok()?;
    if manifest.get("layoutVersion")?.as_u64()? != 1
        || !version_at_least(manifest.get("version")?.as_str()?, (0, 159, 0))
        || manifest.get("entrypoint").is_some_and(|value| {
            value.as_str()
                != Some(if cfg!(windows) {
                    "bin/codex.exe"
                } else {
                    "bin/codex"
                })
        })
        || manifest
            .get("variant")
            .is_some_and(|value| value.as_str() != Some("codex"))
        || manifest
            .get("resourcesDir")
            .is_some_and(|value| value.as_str() != Some("codex-resources"))
        || manifest
            .get("target")
            .is_some_and(|value| !value.as_str().is_some_and(target_matches))
    {
        return None;
    }
    let helper = root
        .join("codex-resources")
        .join("voice")
        .join("bin")
        .join(HELPER);
    if helper.canonicalize().ok().as_ref() != Some(&helper) || !executable(&helper) {
        return None;
    }
    Some((binary, helper))
}

fn discover_in_home(binary: &Path, home: Option<&Path>) -> Result<(PathBuf, PathBuf), String> {
    if !binary.as_os_str().is_empty() {
        if let Some(package) = installed(binary) {
            return Ok(package);
        }
    }
    if let Some(home) = home {
        for directory in [
            ".codex/packages/standalone/current/bin",
            ".local/bin",
            ".codex/bin",
        ] {
            if let Some(package) = installed(&home.join(directory).join(CODEX)) {
                return Ok(package);
            }
        }
    }
    Err(UNAVAILABLE.into())
}

pub(super) fn discover(binary: &Path) -> Result<(PathBuf, PathBuf), String> {
    let home = directories::UserDirs::new();
    discover_in_home(binary, home.as_ref().map(|dirs| dirs.home_dir()))
}

fn device_selection(helper: &Path) -> Result<bool, String> {
    let root = helper.parent().and_then(Path::parent).ok_or(UNAVAILABLE)?;
    let manifest = match read_manifest(&root.join("manifest.json")) {
        Ok(manifest) => manifest,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(UNAVAILABLE.into()),
    };
    let version = manifest
        .get("appVersion")
        .and_then(Value::as_str)
        .ok_or(UNAVAILABLE)?;
    if !version_at_least(version, (0, 0, 0)) {
        return Err(UNAVAILABLE.into());
    }
    Ok(version_at_least(version, (0, 161, 0)))
}

fn helper_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command.env_clear();
    for (key, value) in std::env::vars_os() {
        if matches!(
            key.to_string_lossy().to_ascii_uppercase().as_str(),
            "SYSTEMROOT"
                | "WINDIR"
                | "HOME"
                | "USERPROFILE"
                | "LOCALAPPDATA"
                | "APPDATA"
                | "TEMP"
                | "TMP"
                | "TMPDIR"
                | "XDG_RUNTIME_DIR"
                | "PULSE_SERVER"
                | "PULSE_COOKIE"
                | "PIPEWIRE_REMOTE"
                | "DBUS_SESSION_BUS_ADDRESS"
                | "HTTP_PROXY"
                | "HTTPS_PROXY"
                | "ALL_PROXY"
                | "NO_PROXY"
                | "SSL_CERT_FILE"
                | "SSL_CERT_DIR"
                | "REQUESTS_CA_BUNDLE"
                | "CURL_CA_BUNDLE"
        ) {
            command.env(key, value);
        }
    }
    for key in [
        "GST_PLUGIN_PATH",
        "GST_PLUGIN_PATH_1_0",
        "GST_PLUGIN_SYSTEM_PATH",
        "GST_PLUGIN_SYSTEM_PATH_1_0",
    ] {
        command.env(key, "");
    }
    command
        .env(
            "GST_REGISTRY",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GST_REGISTRY_UPDATE", "no")
        .env("GST_REGISTRY_FORK", "no");
    #[cfg(target_os = "linux")]
    for directory in [
        "/usr/lib/x86_64-linux-gnu/alsa-lib",
        "/usr/lib/aarch64-linux-gnu/alsa-lib",
        "/usr/lib64/alsa-lib",
        "/usr/lib/alsa-lib",
    ] {
        if Path::new(directory).is_dir() {
            command.env("ALSA_PLUGIN_DIR", directory);
            break;
        }
    }
    if let Some(parent) = path.parent() {
        command.current_dir(parent);
    }
    command
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .no_console_window();
    command
}

async fn build_commit(helper: &Path) -> Result<String, String> {
    let mut child = helper_command(helper)
        .arg("--build-commit")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| UNAVAILABLE)?;
    let stdout = child.stdout.take().ok_or(UNAVAILABLE)?;
    timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        stdout
            .take((MAX_COMMIT + 3) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| UNAVAILABLE)?;
        if bytes.len() > MAX_COMMIT + 2 {
            return Err(UNAVAILABLE);
        }
        let status = child.wait().await.map_err(|_| UNAVAILABLE)?;
        let commit = std::str::from_utf8(&bytes).map_err(|_| UNAVAILABLE)?.trim();
        if !status.success()
            || commit.is_empty()
            || commit.len() > MAX_COMMIT
            || !commit
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return Err(UNAVAILABLE);
        }
        Ok(commit.to_owned())
    })
    .await
    .map_err(|_| UNAVAILABLE.to_string())?
    .map_err(str::to_owned)
}

async fn framed_exchange<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    input: &mut W,
    output: &mut R,
    message: &Value,
) -> Result<Value, String> {
    let bytes = serde_json::to_vec(message).map_err(|_| "could not encode request")?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err("invalid request size".into());
    }
    input
        .write_u32(bytes.len() as u32)
        .await
        .map_err(|_| "could not write frame header")?;
    input
        .write_all(&bytes)
        .await
        .map_err(|_| "could not write frame payload")?;
    input.flush().await.map_err(|_| "could not flush frame")?;
    let length = output
        .read_u32()
        .await
        .map_err(|_| "could not read frame header")? as usize;
    if length == 0 || length > MAX_FRAME {
        return Err("invalid response size".into());
    }
    let mut bytes = vec![0; length];
    output
        .read_exact(&mut bytes)
        .await
        .map_err(|_| "could not read frame payload")?;
    serde_json::from_slice(&bytes).map_err(|_| "invalid response JSON".into())
}

fn operation(message: &Value) -> &'static str {
    match message.get("type").and_then(Value::as_str) {
        Some("hello") => "handshake",
        Some("initializeRuntime") => "runtime initialization",
        Some("startTransport") => "transport offer",
        Some("applyAnswer") => "transport answer",
        Some("openDevices") => "opening audio devices",
        Some("setAudioControls") => "audio controls",
        Some("inspectAudio") => "audio inspection",
        _ => "request",
    }
}

fn kill(child: &mut Option<Child>, retiring: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        let _ = child.start_kill();
        *retiring = Some(child);
    }
}

struct KillOnCancel<'a> {
    child: &'a mut Option<Child>,
    retiring: &'a mut Option<Child>,
    armed: bool,
}

impl Drop for KillOnCancel<'_> {
    fn drop(&mut self) {
        if self.armed {
            kill(self.child, self.retiring);
        }
    }
}

pub(super) struct NativeHost {
    child: Option<Child>,
    retiring: Option<Child>,
    input: ChildStdin,
    output: ChildStdout,
    device_selection: bool,
}

impl NativeHost {
    pub(super) async fn open(helper: &Path) -> Result<Self, String> {
        let device_selection = device_selection(helper)?;
        let commit = build_commit(helper).await?;
        let mut child = helper_command(helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|_| UNAVAILABLE)?;
        let input = child.stdin.take().ok_or(UNAVAILABLE)?;
        let output = child.stdout.take().ok_or(UNAVAILABLE)?;
        let mut host = Self {
            child: Some(child),
            retiring: None,
            input,
            output,
            device_selection,
        };
        host.expect(
            json!({"type":"hello","protocol":1,"buildCommit":commit}),
            "ready",
            30,
        )
        .await?;
        host.expect(json!({"type":"initializeRuntime"}), "runtimeReady", 30)
            .await?;
        Ok(host)
    }

    async fn exchange(&mut self, message: Value, seconds: u64) -> Result<Value, String> {
        if self.child.is_none() {
            return Err("Codex voice helper is closed".into());
        }
        let operation = operation(&message);
        // Dropping a partially completed exchange must not leave the microphone live.
        let mut guard = KillOnCancel {
            child: &mut self.child,
            retiring: &mut self.retiring,
            armed: true,
        };
        let error = match timeout(
            Duration::from_secs(seconds),
            framed_exchange(&mut self.input, &mut self.output, &message),
        )
        .await
        {
            Ok(Ok(response)) => {
                guard.armed = false;
                return Ok(response);
            }
            Ok(Err(error)) => error,
            Err(_) => return Err(format!("Codex voice helper {operation} timed out")),
        };
        // The helper closes stdout before its fixed, non-sensitive exit code is observable.
        let code = if let Some(child) = guard.child.as_mut() {
            timeout(Duration::from_millis(250), child.wait())
                .await
                .ok()
                .and_then(Result::ok)
                .and_then(|status| status.code())
        } else {
            None
        };
        match code {
            Some(code) => Err(format!(
                "Codex voice helper {operation}: {error} (exit code {code})"
            )),
            None => Err(format!("Codex voice helper {operation}: {error}")),
        }
    }

    async fn expect(&mut self, message: Value, expected: &str, seconds: u64) -> Result<(), String> {
        let operation = operation(&message);
        let response = self.exchange(message, seconds).await?;
        if operation == "transport answer"
            && response.get("type").and_then(Value::as_str) == Some("transportTimedOut")
        {
            self.close();
            return Err("Codex voice helper transport answer timed out".into());
        }
        if response.get("type").and_then(Value::as_str) != Some(expected) {
            self.close();
            return Err(format!(
                "Codex voice helper {operation}: unexpected response"
            ));
        }
        Ok(())
    }

    pub(super) async fn offer(&mut self) -> Result<String, String> {
        let response = self.exchange(json!({"type":"startTransport"}), 20).await?;
        let sdp = response
            .get("sdp")
            .and_then(Value::as_str)
            .filter(|sdp| valid_sdp(sdp));
        if response.get("type").and_then(Value::as_str) != Some("offer") || sdp.is_none() {
            self.close();
            return Err("Codex voice helper transport offer: invalid response".into());
        }
        Ok(sdp.unwrap().to_owned())
    }

    pub(super) async fn answer(
        &mut self,
        sdp: &str,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        if !valid_sdp(sdp) {
            self.close();
            return Err("Invalid Codex voice SDP".into());
        }
        self.expect(
            json!({"type":"applyAnswer","sdp":sdp}),
            "transportReady",
            20,
        )
        .await?;
        if cancel.is_cancelled() {
            self.close();
            return Err(super::CANCELLED.into());
        }
        let message = if self.device_selection {
            json!({"type":"openDevices","selection":{}})
        } else {
            json!({"type":"openDevices"})
        };
        self.expect(message, "devicesOpened", 5).await
    }

    pub(super) async fn controls(&mut self, muted: bool) -> Result<(), String> {
        self.expect(
            json!({"type":"setAudioControls","controls":{"microphoneMuted":muted,"speakerSuppressed":false}}),
            "audioControlsApplied",
            5,
        ).await
    }

    pub(super) async fn levels(&mut self) -> Result<(f64, f64), String> {
        let response = self.exchange(json!({"type":"inspectAudio"}), 5).await?;
        let peak = |name| {
            response
                .get("state")
                .and_then(|state| state.get(name))
                .and_then(Value::as_u64)
                .and_then(|value| u16::try_from(value).ok())
        };
        match (
            response.get("type").and_then(Value::as_str),
            peak("microphonePeak"),
            peak("speakerPeak"),
        ) {
            (Some("audioState"), Some(mic), Some(speaker)) => Ok((
                f64::from(mic) / f64::from(u16::MAX),
                f64::from(speaker) / f64::from(u16::MAX),
            )),
            _ => {
                self.close();
                Err("Codex voice helper audio inspection: invalid response".into())
            }
        }
    }

    pub(super) fn close(&mut self) {
        kill(&mut self.child, &mut self.retiring);
    }

    pub(super) async fn shutdown(&mut self) -> Result<(), String> {
        self.close();
        if let Some(child) = self.retiring.as_mut() {
            child
                .wait()
                .await
                .map_err(|_| "Could not confirm Codex voice helper cleanup".to_string())?;
        }
        self.retiring.take();
        Ok(())
    }
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        self.close();
    }
}

fn valid_sdp(sdp: &str) -> bool {
    !sdp.trim().is_empty() && sdp.len() <= MAX_SDP
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(root: &Path, version: &str) -> (PathBuf, PathBuf) {
        let binary = root.join("bin").join(CODEX);
        let helper = root.join("codex-resources/voice/bin").join(HELPER);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"fixture").unwrap();
        std::fs::write(&helper, b"fixture").unwrap();
        std::fs::write(
            root.join("codex-package.json"),
            json!({"layoutVersion":1,"version":version}).to_string(),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (binary, helper)
    }

    #[test]
    fn release_gates_validate_semver() {
        for version in ["0.159.0", "0.159.0+vendor.1", "0.160.0-rc.1", "1.0.0"] {
            assert!(version_at_least(version, (0, 159, 0)), "{version}");
        }
        for version in [
            "0.158.9",
            "0.159.0-rc.1",
            "0.159",
            "0.159.0-",
            "0.159.0+",
            "0.159.0+one+two",
            "00.159.0",
            "0.159.0-01",
            "garbage",
        ] {
            assert!(!version_at_least(version, (0, 159, 0)), "{version}");
        }
        assert!(!version_at_least("0.161.0-rc.1", (0, 161, 0)));
        assert!(version_at_least("0.161.0+build.1", (0, 161, 0)));
    }

    #[test]
    fn discovers_only_usable_standalone_packages_and_known_fallbacks() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".codex/packages/standalone/current");
        let (binary, helper) = package(&root, "0.161.0");
        let expected = (
            binary.canonicalize().unwrap(),
            helper.canonicalize().unwrap(),
        );
        assert_eq!(discover_in_home(&binary, None).unwrap(), expected);
        assert_eq!(
            discover_in_home(Path::new(""), Some(temp.path())).unwrap(),
            expected
        );
        let shim = temp.path().join("npm/codex");
        std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
        std::fs::write(&shim, "npm shim").unwrap();
        assert_eq!(
            discover_in_home(&shim, Some(temp.path())).unwrap(),
            expected
        );
        std::fs::write(
            root.join("codex-package.json"),
            r#"{"layoutVersion":1,"version":"0.158.9"}"#,
        )
        .unwrap();
        assert!(discover_in_home(&binary, None)
            .unwrap_err()
            .starts_with("unavailable: "));
        std::fs::write(
            root.join("codex-package.json"),
            r#"{"layoutVersion":2,"version":"0.161.0"}"#,
        )
        .unwrap();
        assert!(installed(&binary).is_none());
        std::fs::write(
            root.join("codex-package.json"),
            r#"{"layoutVersion":1,"version":"0.161.0","entrypoint":"../codex"}"#,
        )
        .unwrap();
        assert!(installed(&binary).is_none());
        std::fs::write(
            root.join("codex-package.json"),
            r#"{"layoutVersion":1,"version":"0.161.0"}"#,
        )
        .unwrap();
        std::fs::remove_file(helper).unwrap();
        assert!(installed(&binary).is_none());
    }

    #[test]
    fn voice_manifest_selects_the_right_device_protocol() {
        let temp = tempfile::tempdir().unwrap();
        let (_, helper) = package(temp.path(), "0.161.0");
        let manifest = helper
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("manifest.json");
        assert!(!device_selection(&helper).unwrap());
        for (version, selected) in [
            ("0.159.0", false),
            ("0.160.0", false),
            ("0.161.0-rc.1", false),
            ("0.161.0", true),
            ("1.0.0", true),
        ] {
            std::fs::write(&manifest, json!({"appVersion":version}).to_string()).unwrap();
            assert_eq!(device_selection(&helper).unwrap(), selected);
        }
        for contents in ["{", "{}", r#"{"appVersion":"bogus"}"#] {
            std::fs::write(&manifest, contents).unwrap();
            assert!(device_selection(&helper)
                .unwrap_err()
                .starts_with("unavailable: "));
        }
    }

    #[test]
    fn sdp_is_bounded_and_helper_environment_is_allowlisted() {
        assert!(!valid_sdp("  "));
        assert!(valid_sdp(&"x".repeat(MAX_SDP)));
        assert!(!valid_sdp(&"x".repeat(MAX_SDP + 1)));
        let command = helper_command(Path::new("/tmp/codex-voice-host"));
        let keys: Vec<_> = command
            .as_std()
            .get_envs()
            .map(|(key, _)| key.to_string_lossy().to_ascii_uppercase())
            .collect();
        for secret in [
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
        ] {
            assert!(!keys.iter().any(|key| key == secret));
        }
        assert!(keys.iter().any(|key| key == "GST_PLUGIN_PATH"));
    }

    #[tokio::test]
    async fn framing_is_big_endian_bounded_and_redacted() {
        let (client, mut server) = tokio::io::duplex(1024);
        let (mut reader, mut writer) = tokio::io::split(client);
        let peer = tokio::spawn(async move {
            let length = server.read_u32().await.unwrap() as usize;
            let mut request = vec![0; length];
            server.read_exact(&mut request).await.unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&request).unwrap(),
                json!({"type":"inspectAudio"})
            );
            let reply = br#"{"type":"audioState"}"#;
            server.write_u32(reply.len() as u32).await.unwrap();
            server.write_all(reply).await.unwrap();
        });
        assert_eq!(
            framed_exchange(&mut writer, &mut reader, &json!({"type":"inspectAudio"}))
                .await
                .unwrap()["type"],
            "audioState"
        );
        peer.await.unwrap();

        let (client, mut server) = tokio::io::duplex(1024);
        let (mut reader, mut writer) = tokio::io::split(client);
        let peer = tokio::spawn(async move {
            let length = server.read_u32().await.unwrap() as usize;
            let mut request = vec![0; length];
            server.read_exact(&mut request).await.unwrap();
            server.write_u32((MAX_FRAME + 1) as u32).await.unwrap();
        });
        let error = framed_exchange(&mut writer, &mut reader, &json!({"type":"secret"}))
            .await
            .unwrap_err();
        assert!(!error.contains("secret"));
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn fragmented_helper_response_survives_other_ready_events() {
        let (client, mut server) = tokio::io::duplex(1);
        let (mut reader, mut writer) = tokio::io::split(client);
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(1);
        let (ack_tx, mut ack_rx) = tokio::sync::mpsc::channel(1);
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel::<()>();
        let peer = tokio::spawn(async move {
            let length = server.read_u32().await.unwrap() as usize;
            let mut request = vec![0; length];
            server.read_exact(&mut request).await.unwrap();
            let payload = br#"{"type":"audioState"}"#;
            let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
            frame.extend_from_slice(payload);
            for byte in &frame[..frame.len() - 1] {
                server.write_all(&[*byte]).await.unwrap();
                progress_tx.send(()).await.unwrap();
                ack_rx.recv().await.unwrap();
            }
            server.write_all(&frame[frame.len() - 1..]).await.unwrap();
            let _ = finish_rx.await;
        });
        let request = json!({"type":"inspectAudio"});
        let exchange = framed_exchange(&mut writer, &mut reader, &request);
        tokio::pin!(exchange);
        let mut events = 0;
        let response = loop {
            tokio::select! {
                biased;
                result = &mut exchange => break result.unwrap(),
                event = progress_rx.recv() => {
                    event.unwrap();
                    events += 1;
                    ack_tx.send(()).await.unwrap();
                }
            }
        };
        assert!(events > 4);
        assert_eq!(response["type"], "audioState");
        let _ = finish_tx.send(());
        peer.await.unwrap();
    }

    #[cfg(unix)]
    const FAKE_HELPER: &str = r##"
import json, os, pathlib, struct, sys, time
root = pathlib.Path(__file__).resolve().parents[3]
while True:
    header = sys.stdin.buffer.read(4)
    if not header: break
    length = struct.unpack('>I', header)[0]
    request = json.loads(sys.stdin.buffer.read(length))
    kind = request['type']
    if kind == 'hello':
        assert request == {'type':'hello','protocol':1,'buildCommit':'fixture-build'}
        response = {'type':'ready'}
    elif kind == 'initializeRuntime':
        if (root/'fail-runtime').exists(): sys.exit(23)
        response = {'type':'runtimeReady'}
    elif kind == 'startTransport': response = {'type':'offer','sdp':'fixture-offer'}
    elif kind == 'applyAnswer':
        assert request == {'type':'applyAnswer','sdp':'fixture-answer'}
        if (root/'stall-answer').exists():
            (root/'answer-received').write_text('received')
            while (root/'stall-answer').exists(): time.sleep(0.01)
        response = {'type':'transportTimedOut' if (root/'timeout-answer').exists() else 'transportReady'}
    elif kind == 'openDevices':
        version = json.loads((pathlib.Path(__file__).resolve().parents[1]/'manifest.json').read_text())['appVersion']
        expected = {'type':'openDevices'}
        if tuple(map(int, version.split('.'))) >= (0,161,0): expected['selection'] = {}
        assert request == expected
        (root/'devices-opened').write_text('opened')
        response = {'type':'devicesOpened'}
    elif kind == 'setAudioControls':
        assert request['controls']['speakerSuppressed'] is False
        if (root/'fail-controls').exists(): sys.exit(29)
        response = {'type':'audioControlsApplied'}
        if (root/'stall-controls').exists():
            payload = json.dumps(response).encode()
            frame = struct.pack('>I', len(payload)) + payload
            count = int((root/'stall-controls').read_text())
            sys.stdout.buffer.write(frame[:count])
            sys.stdout.buffer.flush()
            (root/'partial-control-sent').write_text('sent')
            time.sleep(60)
    elif kind == 'inspectAudio':
        if (root/'fail-inspection').exists(): sys.exit(36)
        response = {'type':'audioState','state':{'microphonePeak':65535,'speakerPeak':32768}}
    else: raise ValueError('unexpected request')
    payload = json.dumps(response).encode()
    sys.stdout.buffer.write(struct.pack('>I', len(payload)) + payload)
    sys.stdout.buffer.flush()
"##;

    #[cfg(unix)]
    fn write_fake_helper(helper: &Path) {
        // Build identity must not wait for a cold Python startup on CI.
        std::fs::write(helper, "#!/bin/sh\nif [ \"$1\" = \"--build-commit\" ]; then\n  printf 'fixture-build\\n'\nelse\n  exec /usr/bin/python3 \"$0.py\"\nfi\n").unwrap();
        std::fs::write(helper.with_extension("py"), FAKE_HELPER).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_before_answer_completion_never_opens_devices() {
        if !Path::new("/usr/bin/python3").is_file() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let (_, helper) = package(temp.path(), "0.161.0");
        write_fake_helper(&helper);
        let manifest = helper
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("manifest.json");
        std::fs::write(manifest, json!({"appVersion":"0.161.0"}).to_string()).unwrap();
        std::fs::write(temp.path().join("stall-answer"), "stall").unwrap();
        let mut host = NativeHost::open(&helper).await.unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let answer = host.answer("fixture-answer", &cancel);
        tokio::pin!(answer);
        timeout(Duration::from_secs(5), async {
            loop {
                tokio::select! {
                    result = &mut answer => panic!("answer completed before cancellation: {result:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {
                        if temp.path().join("answer-received").is_file() { break; }
                    }
                }
            }
        }).await.unwrap();
        cancel.cancel();
        std::fs::remove_file(temp.path().join("stall-answer")).unwrap();
        assert!(answer.await.is_err());
        assert!(!temp.path().join("devices-opened").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn host_speaks_reference_protocol_and_cancellation_kills_it() {
        if !Path::new("/usr/bin/python3").is_file() {
            return;
        }
        for (version, partial_bytes) in [("0.159.0", "2"), ("0.161.0", "6")] {
            let temp = tempfile::tempdir().unwrap();
            let (_, helper) = package(temp.path(), version);
            write_fake_helper(&helper);
            let manifest = helper
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("manifest.json");
            std::fs::write(manifest, json!({"appVersion":version}).to_string()).unwrap();
            let mut host = NativeHost::open(&helper).await.unwrap();
            assert_eq!(host.offer().await.unwrap(), "fixture-offer");
            host.answer("fixture-answer", &CancellationToken::new())
                .await
                .unwrap();
            host.controls(false).await.unwrap();
            assert_eq!(host.levels().await.unwrap(), (1.0, 32768.0 / 65535.0));
            std::fs::write(temp.path().join("stall-controls"), partial_bytes).unwrap();
            let mut operation = Box::pin(host.controls(true));
            timeout(Duration::from_secs(5), async {
                loop {
                    tokio::select! {
                        result = &mut operation => panic!("unexpected response: {result:?}"),
                        _ = tokio::time::sleep(Duration::from_millis(10)) => {
                            if temp.path().join("partial-control-sent").is_file() { break; }
                        }
                    }
                }
            })
            .await
            .unwrap();
            drop(operation);
            assert!(host.child.is_none());
            assert!(host.controls(false).await.is_err());
            assert!(host.retiring.is_some());
            host.shutdown().await.unwrap();
            assert!(host.retiring.is_none());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn helper_failures_report_operation_and_exit_code() {
        if !Path::new("/usr/bin/python3").is_file() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let (_, helper) = package(temp.path(), "0.161.0");
        write_fake_helper(&helper);
        std::fs::write(temp.path().join("fail-runtime"), "fail").unwrap();
        let error = NativeHost::open(&helper).await.err().unwrap();
        assert!(error.contains("runtime initialization"));
        assert!(error.contains("exit code 23"));
        std::fs::remove_file(temp.path().join("fail-runtime")).unwrap();
        let mut host = NativeHost::open(&helper).await.unwrap();
        std::fs::write(temp.path().join("fail-controls"), "fail").unwrap();
        let error = host.controls(true).await.unwrap_err();
        assert!(error.contains("audio controls"));
        assert!(error.contains("exit code 29"));
        assert!(host.child.is_none());
        std::fs::remove_file(temp.path().join("fail-controls")).unwrap();
        let mut host = NativeHost::open(&helper).await.unwrap();
        std::fs::write(temp.path().join("timeout-answer"), "timeout").unwrap();
        assert_eq!(
            host.answer("fixture-answer", &CancellationToken::new())
                .await
                .unwrap_err(),
            "Codex voice helper transport answer timed out"
        );
        assert!(host.child.is_none());
        std::fs::remove_file(temp.path().join("timeout-answer")).unwrap();
        let mut host = NativeHost::open(&helper).await.unwrap();
        std::fs::write(temp.path().join("fail-inspection"), "fail").unwrap();
        let error = host.levels().await.unwrap_err();
        assert!(error.contains("audio inspection"));
        assert!(error.contains("exit code 36"));
        assert!(host.child.is_none());
    }
}
