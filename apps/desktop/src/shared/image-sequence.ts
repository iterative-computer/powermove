export const IMAGE_SEQUENCE_ACCEPT = '.png,.jpg,.jpeg,.webp,.bmp';

/** The last number before the extension is the frame index. Padding may vary. */
export function sequenceFrame(name: string) {
  const match = /^(.*?)(\d+)\.(png|jpe?g|webp|bmp)$/i.exec(name);
  if (!match) return null;
  const index = Number(match[2]);
  if (!Number.isSafeInteger(index)) return null;
  return { prefix: match[1]!, index, extension: match[3]!.toLowerCase() };
}

export function orderedSequence<T extends { name: string }>(files: T[]): T[] {
  if (files.length < 2) {
    throw new Error('Choose at least 2 numbered image frames');
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
  }
  return sorted;
}

/** Report gaps as ranges so a sparse sequence never allocates every missing index. */
export function sequenceGaps(files: Array<{ name: string }>): Array<{ start: number; end: number }> {
  const sorted = orderedSequence(files);
  const gaps: Array<{ start: number; end: number }> = [];
  for (let i = 1; i < sorted.length; i++) {
    const previous = sequenceFrame(sorted[i - 1]!.name)!.index;
    const current = sequenceFrame(sorted[i]!.name)!.index;
    if (current > previous + 1) gaps.push({ start: previous + 1, end: current - 1 });
  }
  return gaps;
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
