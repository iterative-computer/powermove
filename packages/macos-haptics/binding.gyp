{
  "targets": [
    {
      "target_name": "macos_haptics",
      "sources": ["src/addon.mm"],
      "xcode_settings": {
        "MACOSX_DEPLOYMENT_TARGET": "10.15",
        "OTHER_LDFLAGS": ["-framework AppKit", "-framework CoreText"]
      }
    },
    {
      "target_name": "powermove-hdr-proxy",
      "type": "executable",
      "sources": ["src/hdr-proxy.m"],
      "xcode_settings": {
        "MACOSX_DEPLOYMENT_TARGET": "12.0",
        "CLANG_ENABLE_OBJC_ARC": "YES",
        "OTHER_LDFLAGS": [
          "-framework Foundation", "-framework AVFoundation", "-framework CoreMedia",
          "-framework CoreVideo", "-framework Metal", "-framework VideoToolbox"
        ]
      }
    }
  ]
}
