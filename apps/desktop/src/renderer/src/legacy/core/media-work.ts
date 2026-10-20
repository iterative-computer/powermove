/** A task boundary lets input, painting and cancellation run between CPU slices.
 * Promise.resolve() only drains microtasks and would still starve the UI. */
export async function yieldMediaWork(): Promise<void> {
  const scheduler = (globalThis as any).scheduler;
  // A boosted scheduler.yield continuation can outrun ordinary timer tasks
  // across hundreds of PCM slices. Queue fresh background work so input,
  // rendering and other application tasks all get a chance to run.
  if (scheduler?.postTask) await scheduler.postTask(() => {}, { priority: 'background' });
  else if (scheduler?.yield) await scheduler.yield();
  else await new Promise<void>(resolve => setTimeout(resolve, 0));
}

/** Keep the exact source-rate envelope, with bounded work even for a single
 * enormous bucket or many audio channels. No second PCM copy is allocated. */
export async function buildWaveform(buffer: any, {
  rate = 100, assertCurrent = () => {}, onProgress, yieldTask = yieldMediaWork,
}: { rate?: number; assertCurrent?: () => void; onProgress?: (completed: number, total: number) => void;
  yieldTask?: () => Promise<void> } = {}): Promise<Float32Array> {
  const frames = Math.max(1, buffer.length || 1);
  const duration = Math.max(.001, Number(buffer.duration) || frames / Math.max(1, Number(buffer.sampleRate) || 1));
  const length = Math.max(1, Math.min(frames, Math.ceil(duration * rate)));
  const peaks = new Float32Array(length);
  const channels = Math.max(1, buffer.numberOfChannels || 1);
  // getChannelData can materialize a whole channel on the renderer thread.
  // Read native AudioBuffers through bounded scratch blocks instead.
  const nativeCopy = typeof buffer.copyFromChannel === 'function';
  const data = Array.from({ length: channels }, (_, channel) => nativeCopy
    ? new Float32Array(Math.min(frames, 32768)) : buffer.getChannelData(channel));
  const offsets = new Array(channels).fill(-32768);
  const total = frames * channels;
  let completed = 0, slice = 0;
  assertCurrent();
  await yieldTask();
  assertCurrent();
  for (let bucket = 0; bucket < length; bucket++) {
    const start = Math.floor(bucket * frames / length);
    const end = bucket === length - 1 ? frames : Math.max(start + 1, Math.floor((bucket + 1) * frames / length));
    let peak = 0;
    for (let channel = 0; channel < channels; channel++) {
      const samples = data[channel];
      for (let index = start; index < end; index++) {
        if (nativeCopy && index >= offsets[channel] + samples.length) {
          offsets[channel] = index;
          buffer.copyFromChannel(samples, channel, index);
        }
        peak = Math.max(peak, Math.abs(samples[nativeCopy ? index - offsets[channel] : index] || 0));
        completed++;
        if (++slice === 32768) {
          slice = 0;
          onProgress?.(completed, total);
          await yieldTask();
          assertCurrent();
        }
      }
    }
    peaks[bucket] = peak;
  }
  onProgress?.(total, total);
  return peaks;
}
