// P-TEXT capability probe. Restores selection and all temporarily enabled sources.
import AppKit
import Carbon
func property(_ source:TISInputSource,_ key:CFString)->String {
    guard let pointer=TISGetInputSourceProperty(source,key) else {return ""}
    return Unmanaged<CFString>.fromOpaque(pointer).takeUnretainedValue() as String
}
let sources=TISCreateInputSourceList(nil,true).takeRetainedValue() as! [TISInputSource]
let original=TISCopyCurrentKeyboardInputSource().takeRetainedValue()
var enabledHere:[TISInputSource]=[]
defer {
    print("RESTORE",TISSelectInputSource(original))
    for source in enabledHere {print("DISABLE_TEMPORARY",TISDisableInputSource(source))}
}
print("ORIGINAL",property(original,kTISPropertyInputSourceID))
var selected=false
for source in sources {
    let id=property(source,kTISPropertyInputSourceID)
    guard id.contains("Japanese") || id.contains("SCIM") else {continue}
    guard let pointer=TISGetInputSourceProperty(source,kTISPropertyInputSourceIsEnabled) else {continue}
    let wasEnabled=CFBooleanGetValue(Unmanaged<CFBoolean>.fromOpaque(pointer).takeUnretainedValue())
    let enabled=TISEnableInputSource(source)
    if !wasEnabled && enabled==0 {enabledHere.append(source)}
    let result=TISSelectInputSource(source)
    print("IME",id,"enable",enabled,"select",result)
    if result==0 {selected=true;break}
}
print(selected ? "TIS selection supported; actual composition still needs the manual check" : "TIS selection unavailable; use the input menu for the manual check")
