// Kivio OCR Helper —— 独立的 Apple Vision OCR sidecar。
// 协议(每行一条 JSON):
//   启动: { "type": "ready", "available": Bool }
//   请求: { "id": Int, "action": "ocr", "imagePath": String }
//   完成: { "id": Int, "type": "done", "content": String }
//   错误: { "id": Int, "type": "error", "message": String }

import AppKit
import Foundation
import Vision

let stdoutLock = NSLock()

func emit(_ obj: [String: Any]) {
  guard let data = try? JSONSerialization.data(withJSONObject: obj, options: []),
        var line = String(data: data, encoding: .utf8) else { return }
  line.append("\n")
  stdoutLock.lock()
  FileHandle.standardOutput.write(Data(line.utf8))
  stdoutLock.unlock()
}

func handleOCR(id: Int, imagePath: String) async {
  guard let image = NSImage(contentsOfFile: imagePath),
        let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
    emit(["id": id, "type": "error", "message": "无法读取图像: \(imagePath)"])
    return
  }

  let request = VNRecognizeTextRequest()
  request.recognitionLevel = .accurate
  request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US", "ja-JP", "ko-KR", "fr-FR", "de-DE"]
  request.usesLanguageCorrection = true

  let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
  do {
    try handler.perform([request])
    let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
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

  guard action == "ocr" else {
    emit(["id": id, "type": "error", "message": "unknown action: \(action)"])
    return
  }
  guard let imagePath = obj["imagePath"] as? String else {
    emit(["id": id, "type": "error", "message": "ocr action 缺 imagePath 字段"])
    return
  }

  Task { await handleOCR(id: id, imagePath: imagePath) }
}

@main
struct KivioOCRHelper {
  static func main() async {
    emit(["type": "ready", "available": true])
    while let line = readLine(strippingNewline: true) {
      if line.isEmpty { continue }
      dispatch(line)
    }
  }
}
