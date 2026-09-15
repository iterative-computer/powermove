export const IMAGE_SEQUENCE_ACCEPT = '.png,.jpg,.jpeg,.webp,.bmp';
export const MAX_SEQUENCE_FRAMES = 20_000;

/** The last number before the extension is the frame index. Padding may vary. */
export function sequenceFrame(name: string) {
  const match = /^(.*?)(\d+)\.(png|jpe?g|webp|bmp)$/i.exec(name);
  if (!match) return null;
  const index = Number(match[2]);
  if (!Number.isSafeInteger(index)) return null;
  return { prefix: match[1]!, index, extension: match[3]!.toLowerCase() };
}

export function orderedSequence<T extends { name: string }>(files: T[]): T[] {
  if (files.length < 2 || files.length > MAX_SEQUENCE_FRAMES) {
    throw new Error(`Choose between 2 and ${MAX_SEQUENCE_FRAMES.toLocaleString('en-US')} numbered image frames`);
  }
  const first = sequenceFrame(files[0]!.name);
  if (!first || files.some(file => {
    const frame = sequenceFrame(file.name);
    return !frame || frame.prefix !== first.prefix || frame.extension !== first.extension;
  })) throw new Error('Choose images with the same name and extension, ending in frame numbers (for example frame_0001.png)');
  const sorted = [...files].sort((a, b) => sequenceFrame(a.name)!.index - sequenceFrame(b.name)!.index);
  for (let i = 1; i < sorted.length; i++) {
    const previous = sequenceFrame(sorted[i - 1]!.name)!.index;
    const current = sequenceFrame(sorted[i]!.name)!.index;
    if (current === previous) throw new Error(`Duplicate frame number ${current}`);
    if (current !== previous + 1) throw new Error(`Missing frame ${previous + 1} · select a continuous sequence`);
  }
  return sorted;
}

export function sequenceCandidate(files: Array<{ name: string }>): boolean {
  if (files.length < 2) return false;
  const first = sequenceFrame(files[0]!.name);
  return !!first && files.every(file => {
    const frame = sequenceFrame(file.name);
    return frame?.prefix === first.prefix && frame?.extension === first.extension;
  });
}

export function validSequenceFps(fps: number): boolean {
  return Number.isFinite(fps) && fps >= 1 && fps <= 240;
}

/** Seek inside the frame, avoiding WebM's millisecond timestamp rounding. */
export function sequencePlaybackTime(asset: any, time: number): number | undefined {
  const { fps, frames } = asset.imageSequence || {};
  if (!validSequenceFps(fps) || !Number.isSafeInteger(frames) || frames < 2) return undefined;
  const index = Math.max(0, Math.min(frames - 1, Math.floor((time + 1e-7) * fps)));
  return index / fps + .001;
}
