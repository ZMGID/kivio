// Kivio AI Helper —— Tauri sidecar，把 Apple Foundation Models 暴露成 stdin/stdout JSON 行协议。
// 协议(每行一条 JSON):
//   启动:    { "type": "ready", "available": Bool, "reason": String? }
//   请求:    { "id": Int, "action": "text" | "stream" | "ocr",
//             "prompt": String?,        // text/stream 用
//             "imagePath": String? }    // ocr 用,本地图像绝对路径
//   完成:    { "id": Int, "type": "done", "content": String? }   // text/ocr 模式带 content
//   增量:    { "id": Int, "type": "chunk", "delta": String }     // 仅 stream
//   错误:    { "id": Int, "type": "error", "message": String }
//
// ocr 走 Apple Vision framework(VNRecognizeTextRequest),与 FoundationModels 各自独立 — 一个识别图像文本,
// 另一个翻译/对话,组合起来在外部就是"全本地的截图翻译"。

import Foundation
import Vision
import AppKit

#if canImport(FoundationModels)
import FoundationModels
#endif

// stdout 多个 Task 并发写,加锁保证整行原子
let stdoutLock = NSLock()

func emit(_ obj: [String: Any]) {
  guard let data = try? JSONSerialization.data(withJSONObject: obj, options: []),
        var line = String(data: data, encoding: .utf8) else { return }
  line.append("\n")
  stdoutLock.lock()
  FileHandle.standardOutput.write(Data(line.utf8))
  stdoutLock.unlock()
}

func sendReady() {
  if #available(macOS 26.0, *) {
    let model = SystemLanguageModel.default
    switch model.availability {
    case .available:
      emit(["type": "ready", "available": true])
    case .unavailable(let reason):
      emit(["type": "ready", "available": false, "reason": "\(reason)"])
    @unknown default:
      emit(["type": "ready", "available": false, "reason": "unknown availability state"])
    }
  } else {
    emit(["type": "ready", "available": false, "reason": "macOS 26.0+ required"])
  }
}

@available(macOS 26.0, *)
func handleText(id: Int, prompt: String) async {
  do {
    let session = LanguageModelSession()
    let response = try await session.respond(to: prompt)
    emit(["id": id, "type": "done", "content": response.content])
  } catch {
    emit(["id": id, "type": "error", "message": "\(error)"])
  }
}

@available(macOS 26.0, *)
func handleStream(id: Int, prompt: String) async {
  do {
    let session = LanguageModelSession()
    let stream = session.streamResponse(to: prompt)
    // streamResponse 每次 yield 的是累计快照,需要手动算 delta
    var emittedCount = 0
    for try await partial in stream {
      let full = partial.content
      if full.count > emittedCount {
        let startIdx = full.index(full.startIndex, offsetBy: emittedCount)
        let delta = String(full[startIdx...])
        emit(["id": id, "type": "chunk", "delta": delta])
        emittedCount = full.count
      }
    }
    emit(["id": id, "type": "done"])
  } catch {
    emit(["id": id, "type": "error", "message": "\(error)"])
  }
}

/// Apple Vision 端上 OCR：识别多语言文字,按行拼接返回。不依赖 FoundationModels,所以 macOS 10.15+ 即可。
func handleOCR(id: Int, imagePath: String) async {
  guard let image = NSImage(contentsOfFile: imagePath),
        let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
    emit(["id": id, "type": "error", "message": "无法读取图像: \(imagePath)"])
    return
  }
  let request = VNRecognizeTextRequest()
  request.recognitionLevel = .accurate
  // 多语言识别（Vision 自动选最匹配的）— 覆盖 Kivio 的主要使用场景
  request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US", "ja-JP", "ko-KR", "fr-FR", "de-DE"]
  request.usesLanguageCorrection = true
  let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
  do {
    try handler.perform([request])
    let observations = request.results ?? []
    let lines = observations.compactMap { $0.topCandidates(1).first?.string }
    emit(["id": id, "type": "done", "content": lines.joined(separator: "\n")])
  } catch {
    emit(["id": id, "type": "error", "message": "OCR 失败: \(error)"])
  }
}

func dispatch(_ raw: String) {
  guard let data = raw.data(using: .utf8),
        let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
        let id = obj["id"] as? Int,
        let action = obj["action"] as? String else {
    return
  }
  // OCR 不依赖 FoundationModels,可在不支持 Apple Intelligence 的 macOS 26 环境也跑（极少见,Vision 框架是基础组件）
  if action == "ocr" {
    if let imagePath = obj["imagePath"] as? String {
      Task { await handleOCR(id: id, imagePath: imagePath) }
    } else {
      emit(["id": id, "type": "error", "message": "ocr action 缺 imagePath 字段"])
    }
    return
  }
  guard let prompt = obj["prompt"] as? String else {
    emit(["id": id, "type": "error", "message": "缺 prompt 字段"])
    return
  }
  if #available(macOS 26.0, *) {
    switch action {
    case "text":
      Task { await handleText(id: id, prompt: prompt) }
    case "stream":
      Task { await handleStream(id: id, prompt: prompt) }
    default:
      emit(["id": id, "type": "error", "message": "unknown action: \(action)"])
    }
  } else {
    emit(["id": id, "type": "error", "message": "macOS 26.0+ required"])
  }
}

@main
struct KivioAIHelper {
  static func main() async {
    sendReady()
    while let line = readLine(strippingNewline: true) {
      if line.isEmpty { continue }
      dispatch(line)
    }
  }
}
