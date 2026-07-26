// List every window the window server knows about, with the owning process.
//
// The installed-observation collector needs this because a GPUI application
// publishes no accessibility tree, and an application that publishes no
// accessibility tree publishes no window list either: `System Events` reports
// `count of windows` = 0 for a window that is plainly on the screen. The
// window server still knows the window, so the collector reads the window from
// here and captures it by identifier.
//
// Build:
//     swiftc -O -o <dir>/omega-window-list script/omega-window-list.swift
//
// Output is one JSON array. `onscreen` is false for a window on a space that
// is not the active one, which is why the collector captures by identifier
// rather than by screen region.

import Foundation
import CoreGraphics

guard let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] else {
    FileHandle.standardError.write("cannot read the window list\n".data(using: .utf8)!)
    exit(1)
}

var out: [[String: Any]] = []
for window in list {
    out.append([
        "id": window[kCGWindowNumber as String] ?? 0,
        "pid": window[kCGWindowOwnerPID as String] ?? 0,
        "owner": window[kCGWindowOwnerName as String] ?? "",
        "name": window[kCGWindowName as String] ?? "",
        "layer": window[kCGWindowLayer as String] ?? 0,
        "onscreen": window[kCGWindowIsOnscreen as String] ?? false,
        "bounds": window[kCGWindowBounds as String] ?? [:],
    ])
}

let data = try JSONSerialization.data(withJSONObject: out, options: [.prettyPrinted, .sortedKeys])
FileHandle.standardOutput.write(data)
FileHandle.standardOutput.write("\n".data(using: .utf8)!)
