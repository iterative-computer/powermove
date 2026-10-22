import type { FrameDescriptor, FrameRelease } from './protocol';

type Held = { index: number; seq: number; generation: number; depthGeneration: number; released: boolean; drawn: boolean; retired: boolean; colorDone: boolean; readers: number };
/** Color and chrome share native slot ownership. A displayed depth pair may
 * still be read after the color painter finished, including by another GPU
 * context. Keep it until replacement and every chrome fence has completed. */
export class ChromeLeases {
  private frames = new Map<number, Held>();
  private drawn: number | null = null;
  private shown: number | null = null;
  private retirements = new Map<number, () => number>();
  private retiredGenerations = new Set<number>();
  constructor(private readonly releaseNative: (release: FrameRelease) => void) {}

  track(frame: FrameDescriptor): void {
    if (frame.depthId?.encoding !== 'chromePlanesV1' || this.frames.has(frame.seq) || this.retiredGenerations.has(frame.poolGeneration) || this.retiredGenerations.has(frame.depthId.poolGeneration)) return;
    this.frames.set(frame.seq, { index: frame.poolIndex, seq: frame.seq, generation: frame.poolGeneration, depthGeneration: frame.depthId.poolGeneration, released: false, drawn: false, retired: false, colorDone: false, readers: 0 });
  }
  colorDone(release: FrameRelease): void {
    const frame = this.frames.get(release.seq);
    if (!frame || frame.index !== release.poolIndex) { this.releaseNative(release); return; }
    frame.colorDone = true;
    // An undrawn descriptor was dropped by the presenter.
    if (!frame.drawn) frame.retired = true;
    this.flush(frame);
  }
  drawing(seq: number): void {
    this.drawn = seq;
    const frame = this.frames.get(seq);
    if (frame) frame.drawn = true;
    this.retireUnused();
  }
  showing(seq: number): void { this.shown = seq; this.retireUnused(); }
  private retireUnused(): void {
    for (const frame of this.frames.values()) if (frame.drawn && frame.seq !== this.drawn && frame.seq !== this.shown) {
      frame.retired = true;
      this.flush(frame);
    }
  }
  acquire(seq: number): ((done?: Promise<unknown>) => void) | null {
    const frame = this.frames.get(seq);
    if (!frame || !frame.drawn || frame.retired) return null;
    frame.readers++;
    let finished = false;
    return (done = Promise.resolve()) => {
      if (finished) return;
      finished = true;
      const finish = () => { frame.readers--; this.flush(frame); };
      void done.then(finish, finish);
    };
  }
  retire(generation: number, run: () => number): number {
    this.retiredGenerations.add(generation);
    if ([...this.frames.values()].some(frame => frame.generation === generation || frame.depthGeneration === generation)) {
      this.retirements.set(generation, run);
      return 0;
    }
    return run();
  }
  clear(): void {
    this.drawn = this.shown = null;
    for (const frame of this.frames.values()) { frame.retired = true; this.flush(frame); }
  }
  private flush(frame: Held): void {
    if (frame.released || !frame.retired || !frame.colorDone || frame.readers) return;
    frame.released = true;
    if (this.frames.get(frame.seq) === frame) this.frames.delete(frame.seq);
    this.releaseNative({ poolIndex: frame.index, seq: frame.seq });
    for (const [generation, run] of this.retirements) {
      if ([...this.frames.values()].some(frame => frame.generation === generation || frame.depthGeneration === generation)) continue;
      this.retirements.delete(generation);
      run();
    }
  }
}
