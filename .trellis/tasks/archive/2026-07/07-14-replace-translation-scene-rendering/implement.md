# 照片场景文字重绘实施计划

## Dependencies

- geometry-core 提供 `scene_patch` render slots。
- semantic-routing 提供 PhotoText 判定。
- stroke-erasure 提供 cleaned background 和精确 mask。
- visual-benchmark 提供照片/海报、旋转和长短变化数据集。

## Steps

1. 固化 PhotoText patch、style、glyph condition 和 affine fusion 接口。
2. 建立普通字体旋转 slot 基线，先保证内容完整与几何正确。
3. 评估候选模型的代码、权重、许可证、字符集、ONNX 转换和推理依赖。
4. 对合格候选实现本地原型，使用确定性 glyph condition 避免生成拼写错误。
5. 实现长度自适应、边距、旋转/透视和融合。
6. 接入统一离线包、懒加载和单区域失败回退。
7. 在 Windows/macOS 上测量冷/热延迟、峰值内存、模型体积和输出一致性。
8. 达到全部门禁后接入默认 PhotoText 路由；否则只保留普通字体新管线基线。

## Validation

```bash
cargo test --manifest-path src-tauri/Cargo.toml scene_text_rendering
cargo test --manifest-path src-tauri/Cargo.toml inpainting
npm test -- src/lens/replaceTextLayout.test.ts
npm run typecheck
```

人工/自动门禁：OCR 回读准确率、CER、几何误差、背景接缝、区域外像素一致性、1/2/9/10 字符长度变化。

## Rollback

候选模型未达标则删除其产品依赖和权重，保留普通字体 `scene_patch` 路径；不恢复旧 ReplaceLayoutRegion。

## 2026-07-14 Progress（确定性基线，审计后）

- [x] 锁定 `scene_patch` 的普通字体 fallback:`ReplaceTranslateOverlay` 中 `scene_patch` slot 现走 bounds-anchored 完整文本路径（换行+安全缩放，不截断不丢字），新增 colocated vitest 断言完整译文被绘制；overlay 加 `// ponytail:` 注释说明旋转与照片模型延迟、不投机实现。前端 overlay 测试 5/5 绿，typecheck 干净。
- **单测已验证**:PhotoText 区域在无模型时降级为普通字体、内容完整的确定性 fallback（Step 2 的"普通字体基线"最小切片 + Step 6 的"单区域失败回退"语义）。
- **未做（诚实标注,均属硬阻塞或需前置依赖）**:
  - Step 1/5 旋转/透视/仿射融合:`RenderSlot`/`TextAnchor` 目前无 rotation 字段,产出旋转 `scene_patch` 需扩 V2 协议(layout.rs+api/tauri.ts+overlay)——当前无产出触发器(PhotoText 路由未接线),按 ponytail 不投机 churn 协议,待 semantic-routing 路由接线后再加。
  - Step 3/4 候选模型(SRNet/MOSTEL 类)许可/字符集/ONNX 导出/推理评估:需下载评估 ML 模型,本会话无法进行。
  - Step 7/8 冷热延迟、峰值内存、模型体积双平台测量 + 照片/海报人工目检:需真机 + 双平台。
  - `build_replace_geometry` 尚未产出 `scene_patch`(依赖 PhotoText 路由判定);`cargo test scene_text_rendering` 测试模块未建(旋转/模型基线落地后再建)。
- 结论:scene-rendering 仅完成"普通字体 fallback 可测基线",**不可归档**;其头号价值(照片重绘质量)全部受 ML 模型评估与照片双平台目检硬阻塞,且依赖 semantic-routing 的 PhotoText 路由接线。
