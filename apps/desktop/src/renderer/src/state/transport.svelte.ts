/* Transport + tool state — scalars only, safe as deep $state.
   `time` is written once per frame during playback; only leaf readouts and
   controls may read it (structural deriveds key on doc.tick instead). */
import { sameAudioHealth, type AudioHealth } from '../engine-connection/audio-diagnostics';

export const transport = $state({
  time: 0,
  playing: false,
  quality: 1,
  loop: true,
  tool: 'select' as string
});

/* Value fields need readable feedback during playback, not a DOM update for
   every display frame. Keep the actual playhead and edit timestamps live. */
const readout = $state({ time: 0 });
let lastReadoutAt = -Infinity;
export function updateControlTime(time: number, playing: boolean, now: number, force = false): void {
  if (force || !playing || now - lastReadoutAt >= 1000 / 15) {
    readout.time = time;
    lastReadoutAt = now;
  }
}
export function controlTime(): number {
  return transport.playing ? readout.time : transport.time;
}

/* ≤2 Hz status data mirrored from the engine's non-reactive stats. */
export const perf = $state({ native: false, ms: 0, fps: 0, draws: 0, passes: 0, progs: 0, raster: 0 });

/* The engine's audio health (engine-connection/audio-diagnostics.ts), null
   without engine audio. Replaced only when it changes, at most 2 Hz. */
export const audioStatus = $state({ health: null as AudioHealth | null });
export function updateAudioHealth(next: AudioHealth | null): void {
  if (!sameAudioHealth(audioStatus.health, next)) audioStatus.health = next;
}
