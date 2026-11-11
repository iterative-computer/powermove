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
# 4. The sheet.
node tools/visual-parity/sheet.mjs --dir "$S/paper" --title Paper [--notes notes.json]
```

- v1 frames are the exporter's frames (`renderToPixels` with the export
  options, Full quality) after `prepareFrame` and three identical renders.
- V2 frames (`--via export`, default) come from the app's `export_video`:
  full-size ProRes from the export job on the open project, with the migrated
  profile's extensions, decoded by frame index. An export refuses an effect
  whose extension isn't installed; for such a profile use `--via stills`
  (`render_frames`, at most 1280 wide, missing effects bypassed as the viewer
  does) and capture v1 with `--width 1280`.
- Times: every `--step` seconds, the last frame, and `--times` (seconds, or
  `s:ff` timecode at the project rate). V2 reuses v1's list.
- `notes.json` maps a sample label (`t13_458`) to its cause or the reason a
  difference is accepted; nothing is accepted without one.
