# Schema 引用链栈溢出诊断证据

这是一项隔离进程诊断，不是业务实现或完整回归套件。只调用现有 SDK 的公开 schema API。工作线程栈固定为 2 MiB，schema 用循环构造，JSON 容器深度为三层；引用链长度可以增长而容器深度不增长。

2026-11-07 的同产物结果见 `results.jsonl`、`metadata.json` 和四份日志。32 引用的 coercion、1024 引用的普通校验、1024 引用的 validator map 构造成功；1024 引用的 coercion 以 `0xC00000FD` 退出。后者的 schema 编码为 31,600 字节。这个数是具体样本，不是通用安全阈值。schema 编译对照直接使用 jsonschema 相同版本和功能；本例只有本地引用，不涉及外部 schema 获取。

构建命令如下。显式 target-dir 避免在文档目录生成构建文件。

```powershell
cargo build --locked --manifest-path docs/research/kivio-agent-stack-evidence-2026-11-07/Cargo.toml --target-dir .scratch/schema-evidence-target
```

逐项启动独立可执行进程，并由调用者检查退出码。最后一项有意验证原生栈溢出，不能把这段代码直接嵌入主应用或普通同进程测试。

```powershell
& .scratch/schema-evidence-target/debug/kivio-schema-stack-probe.exe 32 coerce
& .scratch/schema-evidence-target/debug/kivio-schema-stack-probe.exe 1024 validate
& .scratch/schema-evidence-target/debug/kivio-schema-stack-probe.exe 1024 build_map
& .scratch/schema-evidence-target/debug/kivio-schema-stack-probe.exe 1024 coerce
```

原记录使用 rustc 1.96.0、未优化 dev、源提交 `36655fa1`，原始构建路径在 `.scratch/stack-architecture/schema-probe`，复用了独立 consumer 的 target 缓存。复制到此目录后的重建会改变调试路径和产物哈希；对比应重新记录哈希，不能冒用原值。新编译器或修复后的 SDK 不必复现相同阈值。固定锁文件仅用于复查本次诊断，不改变应用或 SDK 的锁文件。
