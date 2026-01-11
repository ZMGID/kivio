import Foundation
import Vision
import AppKit

func exitWithError(_ message: String) -> Never {
  FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
  exit(1)
}

guard CommandLine.arguments.count >= 2 else {
  exitWithError("Missing image path")
}

let imagePath = CommandLine.arguments[1]
guard let image = NSImage(contentsOfFile: imagePath),
      let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
  exitWithError("Error: Could not load image")
}

let request = VNRecognizeTextRequest { request, error in
  if let error = error {
    exitWithError("Vision error: \(error.localizedDescription)")
  }

  guard let observations = request.results as? [VNRecognizedTextObservation] else {
    print("")
    exit(0)
  }

  let recognizedStrings = observations.compactMap { observation in
    observation.topCandidates(1).first?.string
  }

  print(recognizedStrings.joined(separator: "\n"))
  exit(0)
}

request.recognitionLevel = .accurate
request.usesLanguageCorrection = true

let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
do {
  try handler.perform([request])
} catch {
  exitWithError("Vision perform error: \(error.localizedDescription)")
}

// The completion handler exits the process.
RunLoop.main.run()

