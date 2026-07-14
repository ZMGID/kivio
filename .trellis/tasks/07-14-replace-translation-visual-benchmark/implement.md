# 替换翻译视觉基准实施计划

## Dependencies

- 本任务最先开始，先提供 fixture schema 与核心几何/擦除基线。
- 随 geometry、routing、erasure、scene 子任务逐步扩展产物和门禁。
- 最终归档晚于其他所有子任务的质量检查。

## Steps

1. 建立 fixture 目录、`case.json` schema、结果目录和隐私规则。
2. 把已知文档、仓库列表、表格、代码徽章和照片失败样例转为本地/合成夹具。
3. 实现离线 runner：固定 OCR/翻译输入时可只跑布局、mask 和 Canvas。
4. 输出 geometry/mask/cleaned/final 阶段文件和 contact sheet。
5. 实现几何、重叠、保护像素、残影、完整性和性能指标。
6. 建立 macOS/Windows 平台 fixture 执行清单和结果合并格式。
7. 为每个子任务固化初始阈值与 required buckets。
8. 集成到本地质量命令；适合 CI 的无模型测试进入 CI，真实模型/实机测试作为发布前门禁。
9. 最终生成一份场景分桶报告，所有必需桶通过后允许父任务收尾。

## Validation

```bash
npm test -- src/lens
cargo test --manifest-path src-tauri/Cargo.toml replace_translation
npm run typecheck
npm run build:ui
```

新增命令目标：

```bash
npm run test:replace-visual
cargo test --manifest-path src-tauri/Cargo.toml replace_translation_visual_fixtures
```

## Rollback

基准工具只读源图并写结果目录，不改变生产状态。指标不稳定时先降级为报告项，不能删除原始夹具或以人工“看起来可以”替代。

## 2026-07-14 Progress

- [x] 新增 V1 fixture schema 与 5 组确定性合成夹具：菜单、文档段落、表格、代码徽章、渐变照片。
- [x] macOS/Windows 平台、Retina/DPI 和场景标签写入每个 `case.json`。
- [x] 新增几何锚点、IoU、跨区/跨列、保护像素、mask 外像素和译文完整性指标。
- [x] 新增按场景分桶的发布门禁，缺少 required bucket 或照片桶失败时不能被平均分掩盖。
- [x] 新增 `npm run fixtures:replace-visual` 与 `npm run test:replace-visual`。
- [ ] 后续子任务接入真实 leaves/groups/slots/mask/cleaned/final 阶段产物与 contact sheet。
- [ ] 完成 macOS/Windows 实机结果合并和最终发布报告。
