# Vendored wgpu 30.0.1 (front crate only)

Upstream: crates.io `wgpu` 30.0.1 (git `40f4a34ebaf56f9a046231f54125ad046239d3f3`,
`wgpu/`), MIT OR Apache-2.0 (`LICENSE.MIT`, `LICENSE.APACHE`). Patched in through
`[patch.crates-io]` in `engine/Cargo.toml`; `wgpu-core`, `wgpu-hal` and
`wgpu-types` are unchanged from crates.io.

The only change: deterministic work counters (`pm-perf`, `docs/engine/perfbench.md`).

- `src/pm_counters.rs` (new): re-exports `pm_perf`'s counters and computes a
  texture descriptor's bytes.
- `src/lib.rs`: `mod pm_counters;`.
- `Cargo.toml`: the `pm-perf` path dependency.
- `src/api/{device,texture,queue,command_encoder,render_pass,compute_pass,render_bundle_encoder,buffer}.rs`:
  one counter line (marked `// Powermove: pm-perf counters.`) at the top of each
  resource creation, `begin_*_pass`, copy, draw, dispatch, `set_pipeline`,
  `set_bind_group`, `write_*`, `submit`, and `BufferSlice::map_async`.

Each line calls `pm_perf::add`: an increment of a const-initialized
thread-local cell, plus (only while a bench has turned on
`pm_perf::count_globally`, never in the app) a relaxed atomic add to a
process-wide total. Behavior is otherwise upstream's.

Not counted: `create_texture_from_hal` and `create_buffer_from_hal` (they wrap
memory made elsewhere, such as IOSurface imports), and anything done below
wgpu (`wgpu-hal` or Metal directly, as `pm-gpu`'s `decode_in`, `bridge_out`
and `device.rs` do). To move to a new wgpu: copy the new release's crate here, re-apply
the marked lines (`grep -rn "Powermove: pm-perf" src`), and keep `pm_counters.rs`.
