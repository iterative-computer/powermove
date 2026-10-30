# studio-gpui

Studio in a GPUI window: the native host prototype for Powermove's GPUI
rewrite (`docs/3d-studio-redesign.md`, section 4.1a).

```sh
cargo run --release                       # the window
cargo run --release -- --headless /tmp/o  # scripted orbit; writes live.png and clean.png
```

- **Drag** to orbit, **scroll** to dolly, pick a scene along the top.
- While the view moves Studio draws **Live** (half resolution, one sample,
  denoised: 3–8 ms on an M5); at rest it refines to 256 samples ("Clean").
- A worker thread owns the GPU (`pm_studio::gpu::Renderer`) and sends
  finished frames to the window as GPUI images, so the UI thread never waits
  on a trace. A later step hands GPUI the GPU texture directly instead of a
  CPU copy.

Its own Cargo workspace (GPUI is large), using the same `gpui-pre` release
as the other GPUI work on this machine.
