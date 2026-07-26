// Read the text a person actually sees in a captured window.
//
// Vision returns one observation per recognised line together with its
// normalised bounding box, so a caller can ask not only whether a phrase is on
// screen but where it sits relative to another phrase. That ordering is the
// thing an accessibility-tree read cannot give when the application publishes
// no tree at all.
//
// Output is one JSON object: {"lines":[{"text":..,"confidence":..,"x":..,
// "y":..,"w":..,"h":..}]}. `y` is measured from the TOP of the image, so a
// smaller `y` means higher on screen.

import Foundation
import Vision
import CoreGraphics
import ImageIO

struct Line: Codable {
    let text: String
    let confidence: Double
    let x: Double
    let y: Double
    let w: Double
    let h: Double
}

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write("usage: ocr <image>\n".data(using: .utf8)!)
    exit(2)
}

let path = CommandLine.arguments[1]
guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
    FileHandle.standardError.write("cannot read image\n".data(using: .utf8)!)
    exit(3)
}

let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = false

let handler = VNImageRequestHandler(cgImage: image, options: [:])
do {
    try handler.perform([request])
} catch {
    FileHandle.standardError.write("vision failed: \(error)\n".data(using: .utf8)!)
    exit(4)
}

var lines: [Line] = []
for observation in (request.results ?? []) {
    guard let candidate = observation.topCandidates(1).first else { continue }
    let box = observation.boundingBox
    // Vision measures y from the bottom. Flip it so the caller can reason in
    // reading order without having to remember which way is up.
    lines.append(
        Line(
            text: candidate.string,
            confidence: Double(candidate.confidence),
            x: Double(box.minX),
            y: Double(1.0 - box.maxY),
            w: Double(box.width),
            h: Double(box.height)
        )
    )
}
lines.sort { $0.y < $1.y }

let encoder = JSONEncoder()
encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
let data = try encoder.encode(["lines": lines])
FileHandle.standardOutput.write(data)
FileHandle.standardOutput.write("\n".data(using: .utf8)!)
