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
