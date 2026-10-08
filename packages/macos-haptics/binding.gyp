{
  "targets": [
    {
      "target_name": "macos_haptics",
      "conditions": [
        ["OS=='mac'", {
          "sources": ["src/addon.mm"],
          "xcode_settings": {
            "MACOSX_DEPLOYMENT_TARGET": "10.15",
            "OTHER_LDFLAGS": ["-framework AppKit", "-framework CoreText"]
          }
        }, {
          "type": "none"
        }]
      ]
    }
  ]
}
