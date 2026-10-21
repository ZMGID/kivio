#[path = "build_support/resources.rs"]
mod resources;

fn main() {
    // Match the effective Tauri identity for headless consumers as well as AppHandle paths.
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    let base: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string("tauri.conf.json").expect("Tauri config"))
            .expect("valid Tauri config");
    let overlay: serde_json::Value = std::env::var("TAURI_CONFIG")
        .map(|value| serde_json::from_str(&value).expect("valid Tauri config overlay"))
        .unwrap_or(serde_json::Value::Null);
    let identifier = overlay["identifier"]
        .as_str()
        .unwrap_or_else(|| base["identifier"].as_str().unwrap());
    assert!(
        matches!(identifier, "com.zmair.kivio" | "com.zmair.kivio.dev"),
        "unsupported Kivio identity"
    );
    println!("cargo:rustc-env=KIVIO_APP_IDENTIFIER={identifier}");

    // Share package discovery with Vite: every bundled manifest is included in the native catalog.
    println!("cargo:rerun-if-changed=../extensions");
    let mut ids = Vec::new();
    for entry in std::fs::read_dir("../extensions").expect("extension directory") {
        let manifest = entry.expect("extension entry").path().join("manifest.json");
        if !manifest.is_file() {
            continue;
        }
        let data: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap()).expect("extension manifest");
        let id = data["id"].as_str().expect("extension id");
        assert!(data["apiVersion"] == 1, "unsupported extension API: {id}");
        assert!(
            id.starts_with(|c: char| c.is_ascii_lowercase())
                && id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "invalid extension id"
        );
        assert!(
            !ids.contains(&id.to_owned()),
            "duplicate extension id: {id}"
        );
        ids.push(id.to_owned());
    }
    ids.sort();
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("extension_ids.json"),
        serde_json::to_vec(&ids).unwrap(),
    )
    .unwrap();

    // tauri-build does not track the ICO used by the Windows resource compiler.
    // Without this, incremental builds can keep the old icon inside the EXE.
    println!("cargo:rerun-if-changed=icons");

    // Tauri 构建脚本：在编译时生成 Tauri 应用所需的上下文和资源配置
    let attributes = tauri_build::Attributes::new();
    let attributes = if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // tauri-winres embeds its manifest only into bin targets, leaving the
        // libtest harness unable to resolve Common Controls v6 imports at startup
        // (STATUS_ENTRYPOINT_NOT_FOUND). Embed the same dependency through the
        // linker for every executable, and disable the duplicate resource manifest.
        // https://github.com/tauri-apps/tauri/issues/13419
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
        attributes.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
    } else {
        attributes
    };
    tauri_build::try_build(attributes).expect("Tauri build setup failed");

    // These directory mappings match bundle.resources in tauri.conf.json.
    // Watch the roots as well as Tauri's individual files so additions/removals
    // trigger a rebuild. OUT_DIR is <profile>/build/<package-hash>/out.
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let target_dir = out_dir.ancestors().nth(3).expect("Cargo output directory");
    for (source, target) in [
        ("resources/skills", "skills"),
        ("resources/word", "word"),
        ("../docs/licenses", "licenses"),
    ] {
        println!("cargo:rerun-if-changed={source}");
        resources::prune_stale(std::path::Path::new(source), &target_dir.join(target))
            .unwrap_or_else(|err| panic!("prune bundled {target}: {err}"));
    }

    // ScreenCaptureKit 桥接层依赖 Swift Concurrency runtime（libswift_Concurrency.dylib）。
    // 上游 screencapturekit crate 的 build.rs 只在装了完整 Xcode.app 的机器上能找到该 dylib
    // （rpath 写死到 `<xcode>/Toolchains/XcodeDefault.xctoolchain/usr/lib/...`）。
    // 仅装 CommandLineTools 的环境下原 rpath 找不到 → 启动时 dyld Library not loaded。
    //
    // 对策：补一条 `/usr/lib/swift` rpath。该路径在 macOS 12+ 由 dyld shared cache 提供，
    // 系统级 swift runtime 始终在那；不要再加 CommandLineTools 的 swift-5.5 路径，
    // 否则会同时加载两份 runtime，产生 "Class implemented in both" objc 警告。
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
