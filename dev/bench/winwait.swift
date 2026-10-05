// Launches an app binary and prints "<pid> <seconds>" once it shows an on-screen window of at
// least <minW>x<minH> points (so splash screens don't count). The app is left running.
// Window owner and bounds are readable without Screen Recording permission.
//
//   winwait <executable> [minW] [minH]
import CoreGraphics
import Foundation

let args = CommandLine.arguments
let minW = Double(args.count > 2 ? args[2] : "700")!
let minH = Double(args.count > 3 ? args[3] : "440")!
let app = Process()
app.executableURL = URL(fileURLWithPath: args[1])
app.standardOutput = FileHandle.nullDevice
app.standardError = FileHandle.nullDevice
let start = Date()
try! app.run()
let pid = app.processIdentifier
while Date().timeIntervalSince(start) < 120 {
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
    for w in list where (w[kCGWindowOwnerPID as String] as? Int32) == pid && (w[kCGWindowLayer as String] as? Int) == 0 {
        let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
        if (b["Width"] ?? 0) >= minW && (b["Height"] ?? 0) >= minH && ((w[kCGWindowAlpha as String] as? Double) ?? 0) > 0 {
            print(pid, String(format: "%.3f", Date().timeIntervalSince(start)))
            exit(0)
        }
    }
    usleep(5_000)
}
print(pid, "timeout")
exit(1)
