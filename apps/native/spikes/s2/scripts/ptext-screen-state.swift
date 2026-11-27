import AppKit
import CoreGraphics
// IOConsoleLocked is authoritative. CGSSessionScreenIsLocked can stay true
// after the console unlocks on this macOS version.
let check = Process()
check.executableURL = URL(fileURLWithPath: "/usr/sbin/ioreg")
check.arguments = ["-n", "Root", "-d1", "-a"]
let output = Pipe()
check.standardOutput = output
try check.run()
let data = output.fileHandleForReading.readDataToEndOfFile()
check.waitUntilExit()
let archive = try PropertyListSerialization.propertyList(from: data, format: nil)
let root = archive as? [String:Any] ?? (archive as? [[String:Any]])?.first ?? [:]
let locked = check.terminationStatus != 0 || (root["IOConsoleLocked"] as? Bool ?? true)
let rate = NSScreen.main?.maximumFramesPerSecond ?? 0
let result:[String:Any] = ["locked":locked,"maximum_hz":rate]
let resultData = try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys])
print(String(data:resultData,encoding:.utf8)!)
