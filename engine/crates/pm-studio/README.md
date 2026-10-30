# pm-studio

Studio, Powermove's product-shot renderer. Design and roadmap:
`docs/3d-studio-redesign.md`. This crate is **Phase 0**: one
GPU path tracer serving every quality tier, measured on product scenes.

## What it renders

- Layered material (OpenPBR subset): metal or dielectric base with GGX
  specular (visible-normal sampling), a clear coat, rough dielectric
  transmission, and emission — optionally the display's screen composition
  (a texture), so a product's screen shows real content.
- Rectangular softboxes with soft rims, light sampling with multiple
  importance sampling, and a dark studio dome so glass stays black.
- **Light by pointing**: `Softbox::for_highlight` places a softbox so its
  reflection lands on a chosen point as seen from the camera.
- A seamless backdrop that keeps its exact display color (`#F6F5F8` in the
  reference film) and a floor that only catches shadows and contact
  occlusion.
- A thin-lens camera (focal length, f-stop, focus point).
- Near-mirror coats (cover glass, lacquer) are traced on every sample, so
  their reflections never sparkle.
- Exposure → Khronos PBR Neutral → sRGB.

## Tiers

| Tier | How |
|---|---|
| Live | Half resolution, 1 sample, 2 bounces, cheaper floor shadows; edge-aware à-trous denoise of the noisy part only (direct screen emission is exact) |
| Refine | Accumulates 1-sample slices while nothing changes |
| Final | Fixed samples per frame, 8 bounces; optionally denoised by Open Image Denoise |

## Run

```sh
cargo run --release -p pm-studio --bin studio-spike -- --out /tmp/studio --scene all --size 1920x1080
# turntable frames:
cargo run --release -p pm-studio --bin studio-spike -- --out /tmp/studio --scene edge --frames 96 --frame-spp 48
```

### Final denoising (Open Image Denoise)

With the `oidn` feature, `Renderer::finish_with` runs Intel Open Image
Denoise (Apache-2.0) on Final frames, guided by first-hit albedo and normal
images traced with the same camera. The backdrop's shadow shading is
denoised too; the screen's emission is added afterwards, so it stays exact.

```sh
brew install open-image-denoise     # or set OIDN_DIR to an OIDN 2.x install
cargo run --release -p pm-studio --features oidn --bin studio-spike -- --out /tmp/studio --scene hero --final-spp 16
```

This writes `<scene>-final-oidn.png` beside `<scene>-final.png`. At 960×540
on an M5, 16 samples plus denoising (≈ 0.25 s) is cleaner than 256 samples
without it.

Scenes: `hero` (flat lay with stylus), `edge` (thin silhouette turning to
face the camera), `macro` (shallow focus across the display corner), `back`
(camera bump, lit by a painted highlight), `chrome` (reflection stress test),
`materials` (sheen, thin film, dispersion, glass, brushed metal, wax),
`caustics` (glass and gold under one hard light; `--no-caustics` to compare),
`wet` (droplets on the display).
The tablet and wallpaper are original procedural designs. `mesh::load_obj`
reads OBJ; a `.usdz` converts to OBJ with macOS ModelIO.

## Phase 0 measurements

Apple M5 (10-core GPU, 16 GB), wgpu 30 on Metal, 1920 × 1080 output,
2026-10-29. Live is GPU time per frame without readback; Refine is the time
to 64 samples; Final is 256 samples.

| Scene | Triangles | Live median | Refine (64 spp) | Final (256 spp) |
|---|---|---|---|---|
| hero | 4,196 | 5.1 ms | 0.94 s | 3.7 s |
| edge | 3,860 | 3.6 ms | 0.38 s | 1.4 s |
| macro | 3,860 | 7.2 ms | 1.27 s | 5.3 s |
| back | 3,860 | 8.1 ms | 2.28 s | 9.1 s |
| chrome | 77,228 | 11.0 ms | 3.83 s | 15.6 s |

Against the Phase 0 gate (60 fps Live at 1080p, i.e. ≤ 6 ms of GPU; Refine
clean in ≤ 2 s): Live passes on `hero` and `edge`, misses on the others;
Refine at 64 samples is under 2 s for three of five scenes. Not yet
measured: a base M1 Air, and a side-by-side against a reference renderer.

## Known gaps (next steps)

- Speed: hardware ray tracing (Metal, M3 and later), a compressed wide BVH,
  temporal reprojection for Live, and a GPU upscaler; Final at 256 samples
  should drop well under 2 s per 1080p frame.
- Caustics come from a photon grid (`photons` in `trace.wgsl`); very sharp
  focus lines (a lens's hot spot) are softened to the grid's cell size.
- No multiple-scattering energy compensation, specular anti-aliasing,
  motion blur or OIDN denoising for Final.
- Not wired into the editor: the viewer bridge, layers, the Studio UI and
  agent operations are Phases 1–2.
