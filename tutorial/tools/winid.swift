// Prints the window number and bounds of survey3d's main window, for
// `screencapture -l`.
import CoreGraphics
import Foundation

let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
let windows = list.filter { ($0[kCGWindowOwnerName as String] as? String) == "survey3d" && ($0[kCGWindowLayer as String] as? Int) == 0 }
let main = windows.max { a, b in
    let ra = a[kCGWindowBounds as String] as? [String: Double] ?? [:]
    let rb = b[kCGWindowBounds as String] as? [String: Double] ?? [:]
    return (ra["Width"] ?? 0) * (ra["Height"] ?? 0) < (rb["Width"] ?? 0) * (rb["Height"] ?? 0)
}
if let w = main, let n = w[kCGWindowNumber as String] as? Int {
    let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
    print("\(n) \(Int(b["Width"] ?? 0)) \(Int(b["Height"] ?? 0))")
} else {
    exit(1)
}
