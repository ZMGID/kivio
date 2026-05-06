// swift-tools-version: 6.0
import PackageDescription

let package = Package(
  name: "kivio-ocr-helper",
  platforms: [.macOS("14.0")],
  targets: [
    .executableTarget(
      name: "kivio-ocr-helper",
      path: "Sources"
    )
  ]
)
