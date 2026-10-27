/** Where an export spends its time, so a slow export can be attributed to a
 * stage instead of guessed at. Per-frame stage times are summed per frame and
 * summarised as totals and p50/p95; waits for the encoder count under `encode`. */
export type ExportStage = 'preparing' | 'decoding' | 'rendering' | 'encoding' | 'mixing audio' | 'finalizing';
type FrameStage = 'decode' | 'render' | 'encode';

export type ExportStatsSummary = {
  frames: number;
  repeatedFrames: number;
  seconds: number;
  fps: number;
  encoder?: string;
  stages: Record<FrameStage, { totalMs: number; p50Ms: number; p95Ms: number }>;
  audioMs: number;
  finalizeMs: number;
  video?: { decoded: number; seeked: number; fallbacks: string[] };
};

const round = (value: number) => Math.round(value * 10) / 10;
function percentile(sorted: Float64Array, fraction: number) {
  return sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))]! : 0;
}

export function createExportStats(total: number) {
  const started = performance.now();
  const frames: Record<FrameStage, Float64Array> = {
    decode: new Float64Array(total), render: new Float64Array(total), encode: new Float64Array(total),
  };
  const touched = new Uint8Array(total);
  const stats = {
    stage: 'preparing' as ExportStage,
    repeated: 0,
    audioMs: 0,
    finalizeMs: 0,
    encoder: undefined as string | undefined,
    add(stage: FrameStage, frame: number, ms: number) {
      if (frame < 0 || frame >= total || !(ms >= 0)) return;
      frames[stage][frame]! += ms;
      touched[frame] = 1;
    },
    /** Time one stage of one frame, recording it even when the step throws. */
    async time<T>(stage: FrameStage, frame: number, run: () => Promise<T> | T): Promise<T> {
      const at = performance.now();
      try { return await run(); } finally { stats.add(stage, frame, performance.now() - at); }
    },
    summary(video?: ExportStatsSummary['video']): ExportStatsSummary {
      const seconds = (performance.now() - started) / 1000;
      const count = touched.reduce((sum, value) => sum + value, 0);
      const stage = (name: FrameStage) => {
        const values = Float64Array.from(frames[name].filter((_, i) => touched[i])).sort();
        return { totalMs: round(values.reduce((sum, value) => sum + value, 0)), p50Ms: round(percentile(values, .5)), p95Ms: round(percentile(values, .95)) };
      };
      const exported = count + stats.repeated;
      return {
        frames: exported, repeatedFrames: stats.repeated, seconds: round(seconds), fps: round(exported / Math.max(seconds, .001)),
        ...(stats.encoder ? { encoder: stats.encoder } : {}),
        stages: { decode: stage('decode'), render: stage('render'), encode: stage('encode') },
        audioMs: round(stats.audioMs), finalizeMs: round(stats.finalizeMs),
        ...(video ? { video: { decoded: video.decoded, seeked: video.seeked, fallbacks: video.fallbacks.slice(0, 5) } } : {}),
      };
    },
  };
  return stats;
}
export type ExportStats = ReturnType<typeof createExportStats>;
