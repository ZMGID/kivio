<#
.SYNOPSIS
  Windows 上运行 Kivio 的 Rust 测试（绕开 comctl32 v6 清单缺失导致的 0xC0000139）。

.DESCRIPTION
  cargo test 构建的测试二进制没有 Common-Controls v6 应用清单，而依赖静态导入了
  comctl32!TaskDialogIndirect（仅 v6 导出）→ 测试 exe 加载即 STATUS_ENTRYPOINT_NOT_FOUND。
  本脚本：1) 先只构建测试二进制；2) 给 target/debug/deps 下每个测试 exe 旁放一份外部
  .manifest（声明 v6 依赖，Windows 对无嵌入清单的 exe 会读取同名 .manifest）；3) 再运行测试。
  详见 src-tauri/tests-common-controls.manifest。

.EXAMPLE
  ./scripts/win-cargo-test.ps1
  ./scripts/win-cargo-test.ps1 --lib
  ./scripts/win-cargo-test.ps1 build_error_arm_message
  ./scripts/win-cargo-test.ps1 --lib chat::agent::loop_tests
#>
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repo 'src-tauri\Cargo.toml'
$ccManifest = Join-Path $repo 'src-tauri\tests-common-controls.manifest'
$depsDir = Join-Path $repo 'src-tauri\target\debug\deps'

Write-Host '[win-cargo-test] 1/3 构建测试二进制 (--no-run)...' -ForegroundColor Cyan
cargo test --manifest-path $manifestPath @args --no-run
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host '[win-cargo-test] 2/3 放置 Common-Controls v6 外部清单...' -ForegroundColor Cyan
Get-ChildItem "$depsDir\*.exe" -ErrorAction SilentlyContinue | ForEach-Object {
  Copy-Item -LiteralPath $ccManifest -Destination "$($_.FullName).manifest" -Force
}

Write-Host '[win-cargo-test] 3/3 运行测试...' -ForegroundColor Cyan
cargo test --manifest-path $manifestPath @args
exit $LASTEXITCODE
