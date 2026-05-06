// Kivio AI Helper —— Tauri sidecar，把 Apple Foundation Models 暴露成 stdin/stdout JSON 行协议。
// 协议(每行一条 JSON):
//   启动:    { "type": "ready", "available": Bool, "reason": String? }
//   请求:    { "id": Int, "action": "text" | "stream",
//             "prompt": String? }
//   完成:    { "id": Int, "type": "done", "content": String? }   // text 模式带 content
//   增量:    { "id": Int, "type": "chunk", "delta": String }     // 仅 stream
//   错误:    { "id": Int, "type": "error", "message": String }
//
import Foundation

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

func dispatch(_ raw: String) {
  guard let data = raw.data(using: .utf8),
        let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
        let id = obj["id"] as? Int,
        let action = obj["action"] as? String else {
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
