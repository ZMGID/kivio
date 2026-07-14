# 替换翻译：照片级原文擦除技术调研

## 结论摘要

推荐采用“分层擦除 + 区域级重排”方案：

1. OCR 继续负责识别文本与四点多边形。
2. 擦除掩膜由原始文本多边形小幅膨胀后合并，和译文排版区域分离。
3. 纯色、低纹理背景可走确定性的快速填充；照片、渐变、纹理背景走 MI-GAN ONNX。
4. 修复后的截图作为唯一背景，再按单元格、视觉行或段落区域绘制译文。
5. 不推荐将 LaMa 作为首选运行时；不推荐首期使用 ViTEraser/CTRNet++。

MI-GAN 是当前最适合 Kivio 的主候选：其公开 512 模型约 5.98M 参数，官方 ONNX 完整流水线约 28 MB，MIT 许可，支持任意尺寸输入并自带掩膜裁剪、缩放和合成。Apple M4 本机 CPU 原型实测单次约 0.18–0.24 秒，模型加载约 0.05 秒，视觉效果明显优于传统 OpenCV 修复，并接近 LaMa；代价是单独 Python ONNX Runtime 进程峰值内存约 0.9–1.0 GB，Rust/共享运行时仍需实测。

## 候选方案比较

| 方案 | 擦除质量 | 部署成本 | 公开模型/许可 | 本机实测或公开证据 | 结论 |
| --- | --- | --- | --- | --- | --- |
| Canvas 平均色矩形 | 仅纯色背景勉强可用 | 极低 | 无模型 | 当前实现会把深色字形混入均值，产生灰块 | 淘汰为主路径 |
| OpenCV Telea / Navier–Stokes | 细划痕和小孔较好；复杂语义/纹理弱 | 低 | OpenCV | 官方算法从边界传播颜色或等照度线；本机约 8–14 ms，但人物/复杂结构出现明显错误 | 仅作低纹理快速路径或失败兜底 |
| LaMa ONNX | 大掩膜、周期纹理较强 | 高 | Apache-2.0；社区 ONNX 约 208 MB | M4 CPU 约 3.0–3.4 s，峰值约 3.3 GB；CoreML 仅支持 2736/7956 节点，加载约 16 s 且运行仍约 2.6–3.0 s | 不适合作为默认产品路径 |
| MI-GAN ONNX | 照片、人物和复杂场景表现良好 | 中低 | MIT；官方 ONNX pipeline 约 28 MB | M4 CPU 约 0.18–0.24 s；论文 512 模型 5.98M 参数、15.69 GFLOPs，质量指标接近 LaMa | 推荐默认复杂背景路径 |
| ViTEraser | 专门的场景文字移除，公开论文指标强 | 高 | MIT；Tiny 约 65.39M 参数 | 官方实现基于 PyTorch/CUDA，公开速度为 RTX 3090；没有成熟跨平台 ONNX 产品链证据 | 作为后续质量上限研究，不进入首期 |
| CTRNet++ | 专门场景文字移除、结构恢复 | 很高 | Apache-2.0 | 依赖 LaMa、VGG 与额外结构生成器，训练/推理链复杂且 GPU 导向 | 不进入首期 |

## 本机原型

环境：Apple M4、16 GB 内存、512×512 合成照片，使用同一文字掩膜。

| 路径 | 模型加载 | 单次推理 | 峰值内存（独立 Python 进程） | 模型大小 |
| --- | ---: | ---: | ---: | ---: |
| OpenCV Telea | 无 | 13.5 ms | 未单独测量 | 无 |
| OpenCV Navier–Stokes | 无 | 7.9 ms | 未单独测量 | 无 |
| LaMa CPU | 2.08 s | 2.97–3.39 s | 约 3.32 GB RSS | 208 MB |
| LaMa CoreML + CPU fallback | 16.31 s | 2.58–2.98 s | 约 3.67 GB RSS | 208 MB |
| MI-GAN CPU | 0.05 s | 0.18–0.24 s（4 线程） | 约 0.97 GB RSS | 28 MB |

视觉对比：[08-comparison-migan.jpg](</Users/zmair/.codex/visualizations/2026/07/13/019f599f-b899-7b52-b2c4-ca5496053cb1/inpainting-research/08-comparison-migan.jpg>)。

额外验证：MI-GAN 使用整行矩形掩膜时仍能在测试照片上恢复连续背景，说明 RapidOCR 的四点多边形足以作为首版输入；但实际实现仍应使用多边形而不是轴对齐矩形，以减少对旋转文字周围内容的破坏。

## 关键架构判断

### 1. 擦除掩膜与排版区域必须分离

- `erase polygon`：来自每个 OCR 检测框的四点多边形，仅用于生成修复掩膜；按文字高度自适应膨胀，覆盖抗锯齿边缘和描边。
- `layout region`：由多个 OCR 叶子节点聚合而成，可对应表格单元格、视觉行或段落；仅用于翻译和译文排版。
- 不能用 `layout region` 直接擦除，否则会抹掉表格线、图标、人物和大量本不需要重建的背景。

### 2. MI-GAN 采用本地按需模型

- 模型和 RapidOCR 类似，首次使用时按需下载并做 SHA-256/大小校验。
- 复用当前动态 ONNX Runtime，但 MI-GAN 会创建独立 session；需要验证与 RapidOCR session 同进程时的实际峰值内存。
- 首期以 CPU Execution Provider 为可靠基线。当前公开 MI-GAN pipeline 的 CoreML 编译会因部分动态预处理节点失败；DirectML 也不是首期前提。
- 后续可把官方 pipeline 中的裁剪/缩放/合成移到 Rust，并单独转换固定 512 生成器，以改善内存和硬件 EP 兼容性。

### 3. 背景分类用于性能和确定性

- 低方差、近似纯色或简单线性渐变区域：使用外围环采样、边缘保护和确定性填充，避免生成模型轻微改动 UI 表格线。
- 照片、纹理、人物、复杂渐变或快速路径质量置信度不足：整张截图的联合文字掩膜交给 MI-GAN 一次修复。
- 生成模型失败时保留快速路径结果或原图，不得让整张翻译消失。

### 4. OCR 几何能力现状

- `oar-ocr` 当前结果已经包含每个区域的四点 `bounding_box.points`，但 Kivio 的 `RapidOcrLine` 只降维成 `x/y/width/height`。
- 依赖库没有向 Kivio 暴露 DBNet 概率图，因此首期无法直接获得像素级字符笔画掩膜。
- 首期方案是四点多边形 + 自适应膨胀；后续若真实样本出现大面积误擦，再评估导出检测概率图或增加专门文字分割模型。

## 来源与证据

- LaMa 论文：<https://ar5iv.labs.arxiv.org/html/2109.07161>
- LaMa 官方仓库及 Apache-2.0：<https://github.com/advimman/lama>
- LaMa 社区 ONNX（固定 512，约 208 MB）：<https://huggingface.co/Carve/LaMa-ONNX>
- OpenCV inpainting 官方说明：<https://docs.opencv.org/4.x/df/d3d/tutorial_py_inpainting.html>
- MI-GAN 论文：<https://openaccess.thecvf.com/content/ICCV2023/papers/Sargsyan_MI-GAN_A_Simple_Baseline_for_Image_Inpainting_on_Mobile_Devices_ICCV_2023_paper.pdf>
- MI-GAN 官方仓库、ONNX pipeline 和 MIT 许可：<https://github.com/Picsart-AI-Research/MI-GAN>
- MI-GAN 模型文件清单：<https://huggingface.co/api/models/andraniksargsyan/migan/tree/main?recursive=true&expand=true>
- ViTEraser 官方仓库：<https://github.com/shannanyinxiang/ViTEraser>
- CTRNet++ 官方仓库：<https://github.com/lcy0604/CTRNet-plus>
- ONNX Runtime CoreML EP：<https://onnxruntime.ai/docs/execution-providers/CoreML-ExecutionProvider.html>
- ONNX Runtime DirectML EP：<https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html>

## 调研命令

```bash
smart-search deep "为 Tauri Rust 桌面应用实现接近 QQ 截图翻译的照片级原文擦除与译文重绘……" --budget deep --format json
smart-search exa-search "MI-GAN mobile image inpainting ONNX GitHub model parameters CPU" --num-results 8 --include-highlights --include-text --format json
smart-search fetch "https://openaccess.thecvf.com/content/ICCV2023/papers/Sargsyan_MI-GAN_A_Simple_Baseline_for_Image_Inpainting_on_Mobile_Devices_ICCV_2023_paper.pdf" --format markdown
smart-search fetch "https://huggingface.co/api/models/andraniksargsyan/migan/tree/main?recursive=true&expand=true" --format markdown
```
