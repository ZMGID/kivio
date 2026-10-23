#[path = "../../src-tauri/src/app_identity.rs"]
mod app_identity;
const IDENTITY: app_identity::AppIdentity = app_identity::AppIdentity::NativePreview;

fn main() {
    zeron_ui::run_app(zeron_ui::UiConfig {
        data_dir: catalog_directory()
            .parent()
            .expect("application data root")
            .to_path_buf(),
        ipc_port: 0,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: zeron_ui::HarnessId::Codex,
        initial_url: None,
    });
}

fn catalog_directory() -> std::path::PathBuf {
    // An explicit development identity; never resolves the production/Dev store.
    #[cfg(target_os = "macos")]
    let base = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME is required"))
        .join("Library/Application Support");
    #[cfg(target_os = "windows")]
    let base = std::path::PathBuf::from(std::env::var_os("APPDATA").expect("APPDATA is required"));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME is required"))
                .join(".local/share")
        });
    base.join(IDENTITY.identifier()).join("conversations")
}
