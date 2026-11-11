# Visual parity sheets

v1.2.0 and V2 frames of the same project at the same instants, side by side
with a difference heat map, ranked by visible difference. Outputs never go in
the repository.

```sh
# 1. Clone the v1 profile (never use a live one); strip locks, MCP descriptors and secrets.
cp -cR "$HOME/Library/Application Support/powermove" "$S/v1-clone"   # then remove Singleton*, mcp, cloud, env, Cookies*, "Local State"
# 2. v1 frames: hidden background-test app (Node only; Bun cannot host Playwright's Electron driver).
node tools/visual-parity/v1-frames.mjs --profile "$S/v1-clone" --project Paper.pmv --out "$S/paper" --step 2 --times 13:11,12:08
# 3. V2 frames: the lane app (pm-gui launch with --electron-profile <another clone> to migrate it,
#    profile folder named com.zellzoi.powermove.native-beta) has the project open; the app's MCP renders.
node tools/visual-parity/v2-frames.mjs --app <lane>/Powermove.app --profile <V2 profile> --project Paper.pmv --out "$S/paper"
# 4. Optional: the paused viewer as a person sees it (the lane app shows the project; --rect is
#    the viewer canvas in the native-resolution window screenshot). Capture v1 at the viewer's
#    render size (--width) into its own directory to compare viewer with viewer.
node tools/visual-parity/viewer-frames.mjs --pid <pid> --window <id> --rect x,y,w,h --out "$S/paper"
# 5. Optional: a delivered video of the project (what was posted) as the reference column;
#    project time t is video frame round((t + offset) × rate). Find the offset by matching frames.
node tools/visual-parity/ref-frames.mjs --video posted.mp4 --out "$S/paper" [--offset 0]
# 6. The sheet.
node tools/visual-parity/sheet.mjs --dir "$S/paper" --title Paper [--notes notes.json] [--summary summary.txt]
```

- v1 frames are the exporter's frames (`renderToPixels` with the export
  options, Full quality) after `prepareFrame` and three identical renders.
- V2 frames (`--via export`, default) come from the app's `export_video`:
  full-size ProRes from the export job on the open project, with the migrated
  profile's extensions, decoded by frame index. An export refuses an effect
  whose extension isn't installed; for such a profile use `--via stills`
  (`render_frames`, at most 1280 wide, missing effects bypassed as the viewer
  does) and capture v1 with `--width 1280`.
- v1 waits until the project's user-extension effects have registered; any
  still missing are logged and listed in the manifest.
- v1's viewer renders at composition × min(1, zoom × backing scale) ×
  quality, and effects see that render's pixels; V2's viewer does the same.
  Compare a viewer capture only with v1 at that size, never with the export.
- Times: every `--step` seconds, the last frame, and `--times` (seconds, or
  `s:ff` timecode at the project rate). V2 reuses v1's list.
- With `ref/`, the heat map is V2 against the delivered video and v1 is
  scored against it too, so a difference both apps share (a font missing on
  this Mac, compression) is told apart from one V2 alone has.
- `notes.json` maps a sample label (`t13_458`) to its cause or the reason a
  difference is accepted; nothing is accepted without one.
