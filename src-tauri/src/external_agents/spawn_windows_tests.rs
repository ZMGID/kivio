use super::*;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

#[tokio::test]
async fn resolves_dsh_desktop_without_path_registration() {
    const TEST: &str =
        "external_agents::spawn::windows_tests::resolves_dsh_desktop_without_path_registration";
    if let Some(root) = std::env::var_os("KIVIO_DSH_DESKTOP_FIXTURE") {
        let root = PathBuf::from(root);
        // Windows can restore ProgramFiles at process startup; set fixture roots inside the child.
        std::env::set_var("ProgramFiles", root.join("system"));
        std::env::set_var("ProgramFiles(x86)", root.join("system-x86"));
        let desktop = root.join("Programs/DeepSeek Harness/resources/runtime/cli/bin/dsh.cmd");
        let def = &super::super::providers::dsh::DSH_AGENT_DEF;
        assert!(which_all("dsh").await.is_empty());
        let resolved = resolve_binary(def).await.expect("desktop CLI");
        assert_eq!(resolved, desktop);
        assert_eq!(cached_cli_version(&resolved).as_deref(), Some("0.2.2"));

        let system = root.join("system/DeepSeek Harness/resources/runtime/cli/bin/dsh.cmd");
        std::fs::create_dir_all(system.parent().unwrap()).unwrap();
        std::fs::copy(&desktop, &system).unwrap();
        std::fs::remove_file(&desktop).unwrap();
        assert_eq!(resolve_binary(def).await, Some(system));

        let path_bin = root.join("path/dsh.cmd");
        std::fs::create_dir_all(path_bin.parent().unwrap()).unwrap();
        std::fs::write(&path_bin, "@echo off\r\necho 0.2.3\r\n").unwrap();
        let original = std::env::var_os("PATH").unwrap();
        let paths = std::iter::once(path_bin.parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&original));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        assert_eq!(
            resolve_binary(def).await.unwrap().canonicalize().unwrap(),
            path_bin.canonicalize().unwrap()
        );

        let mut settings = crate::settings::Settings::default();
        let config = settings
            .chat
            .external_cli_agents
            .entry("dsh".into())
            .or_default();
        config.path = root.join("missing/dsh.exe").to_string_lossy().into_owned();
        super::super::overrides::sync_from_settings(&settings);
        assert_eq!(
            resolve_binary(def).await,
            None,
            "explicit paths must not fall back"
        );
        settings
            .chat
            .external_cli_agents
            .get_mut("dsh")
            .unwrap()
            .path = path_bin.to_string_lossy().into_owned();
        super::super::overrides::sync_from_settings(&settings);
        assert_eq!(resolve_binary(def).await, Some(path_bin));
        return;
    }

    // Isolate PATH and installation roots from other tests and real installations.
    let directory = tempfile::tempdir().unwrap();
    let bin = directory
        .path()
        .join("Programs/DeepSeek Harness/resources/runtime/cli/bin/dsh.cmd");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::fs::write(&bin, "@echo off\r\necho 0.2.2\r\n").unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env("KIVIO_DSH_DESKTOP_FIXTURE", directory.path())
        .env("LOCALAPPDATA", directory.path())
        .env(
            "PATH",
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32"),
        )
        .no_console_window()
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) const FIXTURE: &str = "external_agents::spawn::windows_tests::process_fixture";

pub(crate) struct FixtureProcess(HANDLE);

impl FixtureProcess {
    fn open(pid: u32) -> Self {
        Self(unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) }.unwrap())
    }

    pub(crate) async fn stopped(&self) -> bool {
        timeout(Duration::from_secs(3), async {
            while unsafe { WaitForSingleObject(self.0, 0) } != WAIT_OBJECT_0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok()
    }
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        unsafe {
            let _ = TerminateProcess(self.0, 1);
            let _ = CloseHandle(self.0);
        }
    }
}

fn fixture_command(directory: &Path, role: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
        .env("KIVIO_PROCESS_FIXTURE", role)
        .env("KIVIO_PROCESS_FIXTURE_DIR", directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

pub(crate) async fn fixture_process(directory: &Path, role: &str) -> FixtureProcess {
    timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(directory.join(role)) {
                if let Some(pid) = text.strip_suffix('\n').and_then(|pid| pid.parse().ok()) {
                    break FixtureProcess::open(pid);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("fixture must publish its {role} PID"))
}

#[test]
#[ignore = "invoked only by the isolated process ownership tests"]
fn process_fixture() {
    let role = std::env::var("KIVIO_PROCESS_FIXTURE").unwrap();
    if role == "leaf" {
        std::thread::sleep(Duration::from_secs(20));
        return;
    }
    let directory = PathBuf::from(std::env::var_os("KIVIO_PROCESS_FIXTURE_DIR").unwrap());
    if role == "host" {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let mut command = fixture_command(&directory, "root");
            let mut child = AgentChild::spawn(&mut command).unwrap();
            std::fs::write(directory.join("root"), format!("{}\n", child.id().unwrap())).unwrap();
            child.wait().await.unwrap();
        });
        return;
    }
    let leaf = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
        .env("KIVIO_PROCESS_FIXTURE", "leaf")
        .spawn()
        .unwrap();
    println!("fixture-output");
    std::fs::write(directory.join("leaf"), format!("{}\n", leaf.id())).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !directory.join("release").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    eprintln!("fixture-final-output");
}

#[tokio::test]
async fn leader_exit_reclaims_descendant_holding_output() {
    let directory = tempfile::tempdir().unwrap();
    let mut command = fixture_command(directory.path(), "root");
    let child = AgentChild::spawn(&mut command).unwrap();
    let leaf = fixture_process(directory.path(), "leaf").await;
    std::fs::write(directory.path().join("release"), b"").unwrap();
    let output = timeout(Duration::from_secs(3), child.wait_with_output()).await;
    assert!(
        leaf.stopped().await,
        "leader exit must reclaim its descendant"
    );
    let output = output
        .expect("inherited output pipes must reach EOF")
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("fixture-output"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("fixture-final-output"));
}

#[tokio::test]
async fn cancellation_and_drop_reclaim_only_owned_processes() {
    let unrelated_directory = tempfile::tempdir().unwrap();
    let mut unrelated =
        AgentChild::spawn(&mut fixture_command(unrelated_directory.path(), "leaf")).unwrap();
    for cancel in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut child = AgentChild::spawn(&mut fixture_command(directory.path(), "root")).unwrap();
        let root = FixtureProcess::open(child.id().unwrap());
        let leaf = fixture_process(directory.path(), "leaf").await;
        if cancel {
            child.start_kill().unwrap();
            timeout(Duration::from_secs(3), child.wait())
                .await
                .unwrap()
                .unwrap();
        }
        drop(child);
        assert!(root.stopped().await, "root must stop on cancel/drop");
        assert!(leaf.stopped().await, "descendant must stop on cancel/drop");
        assert!(
            unrelated.try_wait().unwrap().is_none(),
            "another owner must stay live"
        );
    }
}

#[tokio::test]
async fn host_death_closes_job_and_reclaims_descendants() {
    let directory = tempfile::tempdir().unwrap();
    let mut host = fixture_command(directory.path(), "host").spawn().unwrap();
    let leaf = fixture_process(directory.path(), "leaf").await;
    let root = fixture_process(directory.path(), "root").await;
    host.start_kill().unwrap();
    timeout(Duration::from_secs(3), host.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(root.stopped().await, "host death must close the root Job");
    assert!(
        leaf.stopped().await,
        "host death must reclaim Job descendants"
    );
}
