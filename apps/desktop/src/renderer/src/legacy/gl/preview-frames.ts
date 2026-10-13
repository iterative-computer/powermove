/** Owns completed GPU frames separately from reusable working render targets. */
export class PreviewFrames<T extends { bytes: number }> {
  private frames = new Map<string, T>();
  private usedBytes = 0;
  constructor(private dispose: (frame: T) => void) {}
  get bytes() { return this.usedBytes; }
  get count() { return this.frames.size; }
  has(key: string): boolean { return this.frames.has(key); }

  get(key: string): T | undefined {
    const frame = this.frames.get(key);
    if (frame) { this.frames.delete(key); this.frames.set(key, frame); }
    return frame;
  }

  put(key: string, frame: T, budget: number): boolean {
    if (frame.bytes <= 0 || frame.bytes > budget) return false;
    const previous = this.frames.get(key);
    if (previous === frame) return true;
    if (previous) this.remove(key, previous);
    this.trim(budget - frame.bytes);
    this.frames.set(key, frame);
    this.usedBytes += frame.bytes;
    return true;
  }

  trim(target: number, protectedFrames?: ReadonlySet<T>): void {
    for (const [key, frame] of this.frames) {
      if (this.usedBytes <= Math.max(0, target)) break;
      if (protectedFrames?.has(frame)) continue;
      this.remove(key, frame);
    }
  }

  clear(): void { this.trim(0); }

  private remove(key: string, frame: T): void {
    this.frames.delete(key);
    this.usedBytes -= frame.bytes;
    this.dispose(frame);
  }
}
