// Read-only session check. Never attempts to unlock or dismiss authentication.
import AppKit
import CoreGraphics
import Foundation
let session = CGSessionCopyCurrentDictionary() as? [String: Any]
if session?["CGSSessionScreenIsLocked"] as? Bool == true {
    fputs("S3 foreground refused: Mac session is locked; unlock it and rerun.\n", stderr)
    exit(77)
}

for screen in NSScreen.screens {
    print("Foreground display: max_hz=\(screen.maximumFramesPerSecond) current_headroom=\(screen.maximumExtendedDynamicRangeColorComponentValue) potential_headroom=\(screen.maximumPotentialExtendedDynamicRangeColorComponentValue)")
}
