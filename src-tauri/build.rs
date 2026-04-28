fn main() {
  // Tauri 构建脚本：在编译时生成 Tauri 应用所需的上下文和资源配置
  tauri_build::build();

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
