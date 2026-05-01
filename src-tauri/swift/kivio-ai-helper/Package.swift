// swift-tools-version: 6.0
import PackageDescription

// 用字符串形式声明平台版本，避免依赖 PackageDescription 6.2 的 .v26 常量(老 CommandLineTools 拿不到)
let package = Package(
  name: "kivio-ai-helper",
  platforms: [.macOS("26.0")],
  targets: [
    .executableTarget(
      name: "kivio-ai-helper",
      path: "Sources"
    )
  ]
)
