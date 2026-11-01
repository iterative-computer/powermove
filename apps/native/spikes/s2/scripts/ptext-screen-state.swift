import AppKit
import CoreGraphics
let session = CGSessionCopyCurrentDictionary() as? [String:Any] ?? [:]
let locked = session["CGSSessionScreenIsLocked"] as? Bool ?? session.isEmpty
let rate = NSScreen.main?.maximumFramesPerSecond ?? 0
let result:[String:Any] = ["locked":locked,"maximum_hz":rate]
let data = try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys])
print(String(data:data,encoding:.utf8)!)
