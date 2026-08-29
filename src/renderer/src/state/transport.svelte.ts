/* Transport + tool state — scalars only, safe as deep $state.
   `time` is written once per frame during playback; only leaf readouts and
   controls may read it (structural deriveds key on doc.tick instead). */
export const transport = $state({
  time: 0,
  playing: false,
  quality: 1,
  loop: true,
  tool: 'select' as string
});

/* ≤2 Hz status data mirrored from the engine's non-reactive stats. */
export const perf = $state({ ms: 0, fps: 0, draws: 0, passes: 0, progs: 0, raster: 0 });
