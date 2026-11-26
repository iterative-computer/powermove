# Distortion waves

A native port of **distortion-waves 0.1.0**, MIT, copyright 2026 Jakub Antalik.
Source: <https://github.com/Jakubantalik/Libraries.dev>, revision
`371b324a253b8ee71b9ce1bc8a1ecc5a59e1a16a` (`packages/distortion-waves`).
Docs: <https://libraries.dev/waves>. The documented npm package returned HTTP 404
when this port was made; the reference uses that exact public source revision.
No JavaScript or browser runtime is linked into the native app.

`pm-wave` contains typed style/palette parameters, the deterministic timeline,
interruptible `WaveClock`, and an offscreen `WaveRenderer`. The GPUI element is
`pm_shell::wave::DistortionWave`, with an `Entity<WaveState>` handle:

```rust,ignore
let state = cx.new(|_| WaveState::new(
    WaveStyle::new(WaveType::Vertical),
    WaveRect { x: 235., y: 330., w: 150., h: 44. },
).unwrap());
DistortionWave::new(state.clone(), from_view)
    .revealed_content(to_view) // omit for a wave into the same view
    .overlay(live_controls)
    .w(px(620.)).h(px(400.))
```

The state exposes `set_revealed`, `play`, `reverse`, `reset`, `prepare`, `scrub`,
`on_finish`, `set_focus`, `set_source`, `set_style`, `set_corner_radius`, and
`set_reduced_motion`. Controls accept the current window to request a repaint;
`error()` reports capture/render errors. `WaveSource::track(control)` measures an
element source in the overlay. A source click supplies the lean automatically.
The shader radius also follows a radius set through GPUI's `Styled` helpers.
Rectangles and focus points are local window points before auto scale.

Both children share a grid cell; the larger child determines intrinsic size.
Give children opaque backgrounds. Pictures stay frozen throughout a run.
AppKit child windows and deferred popovers cannot be captured by GPUI's scene
renderer; keep them outside the pictured children. Preparation requires the
component bounds to fit inside the window's offscreen render target. The overlay remains live.
The native boundary rejects nonfinite values, negative physical parameters,
zero divisors, more than 256 particles, and orientation/spread outside 0–1.
Scrub clamps to 0–1. Cubic-bezier X controls clamp to 0–1 as upstream does.
Defaults, palettes, Gaussian levels, OKLab interpolation, point sprites, forward
clock and collapse easing follow the package source. Native `play` restarts;
`set_revealed` and `reverse` retarget from the current progress.

One full-surface Metal pass plus a small point-sprite pass runs per display
refresh. Prepare uploads the two screenshots and caches near/far Gaussian blur
levels and three leased outputs. No captures/readbacks or texture creation occur
in the frame loop. Idle states request no animation frames. Completion/reset
release screenshots and output-pool ownership; GPUI retains an in-flight output
until its scene/GPU consumer lets go. Reduced Motion follows macOS by default
and renders a plain 300 ms crossfade. GPU input/output bytes are display-encoded
sRGB (`BGRA8Unorm`), like the browser's WebGL canvas. GPUI's Metal layer explicitly
declares sRGB for ColorSync conversion on a P3 display; no P3 reinterpretation or
extra transfer-function decoding is applied.

The app uses no wave. To open the **debug-only** developer example later, from
the native workspace, use the project's native environment and build settings:

```sh
cargo +1.96.1 run --locked -p pm-shell --example distortion_wave_gallery
```

It shows all three types with sample views and Play/Reverse/Reset controls.
The gallery code is excluded when `debug_assertions` is off. It was compiled,
not launched, during implementation.

## Headless verification

```sh
cargo +1.96.1 test --locked -p pm-wave -- --nocapture
cargo +1.96.1 test --locked -p pm-shell --lib wave::tests:: -- --nocapture
cargo +1.96.1 clippy --locked -p pm-wave --all-targets -- -D warnings
```

Use the project's required native environment, target directory and job limit.
The renderer test compiles the production Metal shader at runtime. It renders
620×400 images and compares **all 198 frames** (three types × six palettes ×
eleven scrub positions). The synthetic inputs are `tests/fixtures/from.png`
and `to.png`. Committed reference strips store each frame at 155×100 using exact
4×4 box averaging; native output uses that same averaging. This keeps the
fixtures around 1 MB. Numbers printed per frame are global RGB SSIM and mean
absolute RGB difference normalized to 0–1, after compositing the rounded canvas
onto the test page's `#101822` background. Thresholds: SSIM > 0.9998 and MAE < 0.001.
Set `PM_WAVE_FRAMES` to a scratch output directory to retain full native PNGs.

The tests also cover exact endpoints, one-view mode, reverse interruption and
its collapse curve, reduced-motion pixels/timing, invalid parameters, click
lean/centre hold, no frame-loop texture creation, retained consumer leases,
zero textures after release, and GPUI snapshot/overlay/automatic finish cleanup.
The GPUI test creates only a `HeadlessAppContext`; no app/window is presented.

To regenerate browser reference frames, use an **external scratch directory**:

1. Clone the upstream repository there and check out the revision above.
2. Copy `reference/package.json` into a separate scratch harness directory and
   run `npm install` there (never in this crate). Install headless Chromium with
   Playwright if needed.
3. Run `node reference/generate.mjs UPSTREAM_PACKAGE SCRATCH_HARNESS [CHROMIUM]`.
   `UPSTREAM_PACKAGE` names the checked-out `packages/distortion-waves` folder.
4. Run `python3 reference/pack_frames.py SCRATCH_HARNESS/frames tests/fixtures`.
   Packing needs Pillow. Remove scratch `node_modules` when finished.

The browser runs headless at DPR 1 with WebGL2/Metal; no server or visible tab
is used. The harness remounts each type/palette case because the upstream
controller deliberately retains the source while scrubbing. Its custom capture
returns the exact test PNGs, avoiding DOM/font rasterization differences.

Minor differences remain in Gaussian/mipmap rounding, point-sprite coverage,
dither quantization and rounded clip antialiasing between Chromium WebGL and
native Metal. The formulas and kernels are ported rather than approximated.
Metrics below measure these differences at fixture resolution; they are not a
claim of bit-identical full-resolution GPU output. P3 output follows ColorSync;
the offscreen numerical comparison is sRGB and does not measure a physical panel.

| Type | Palette | Minimum SSIM | Maximum MAE |
| --- | --- | ---: | ---: |
| vertical | colorful | 0.999971 | 0.000105 |
| vertical | green | 0.999975 | 0.000129 |
| vertical | ocean | 0.999978 | 0.000097 |
| vertical | sunset | 0.999971 | 0.000115 |
| vertical | mono | 0.999968 | 0.000108 |
| vertical | 528dff | 0.999978 | 0.000096 |
| horizontal | colorful | 0.999957 | 0.000149 |
| horizontal | green | 0.999950 | 0.000173 |
| horizontal | ocean | 0.999963 | 0.000151 |
| horizontal | sunset | 0.999955 | 0.000153 |
| horizontal | mono | 0.999954 | 0.000157 |
| horizontal | 528dff | 0.999962 | 0.000149 |
| center | colorful | 0.999954 | 0.000163 |
| center | green | 0.999949 | 0.000168 |
| center | ocean | 0.999958 | 0.000162 |
| center | sunset | 0.999952 | 0.000165 |
| center | mono | 0.999952 | 0.000162 |
| center | 528dff | 0.999958 | 0.000160 |
