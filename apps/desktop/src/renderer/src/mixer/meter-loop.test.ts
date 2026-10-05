import { describe, expect, it, vi } from 'vitest';

import { MeterLoop, type LevelSource } from './meter-loop';

function canvas() {
  const calls: string[] = [];
  const context = {
    globalAlpha: 1, fillStyle: '',
    clearRect: () => calls.push('clear'),
    fillRect: (...args: number[]) => calls.push(`fill ${args.join(' ')}`),
    createLinearGradient: () => ({ addColorStop: vi.fn() })
  };
  const element = { width: 0, height: 0, getContext: () => context } as unknown as HTMLCanvasElement;
  return { element, calls };
}

function harness(level = { peak: 0.5, rms: 0.3 }) {
  let running = true;
  const source: LevelSource & { reads: number; releases: number } = {
    MASTER: '$master',
    reads: 0,
    releases: 0,
    read(_id, out) { this.reads++; out.peak[0] = out.peak[1] = level.peak; out.rms[0] = out.rms[1] = level.rms; return true; },
    release() { this.releases++; },
    running: () => running
  };
  const frames: Array<(now: number) => void> = [];
  const loop = new MeterLoop(source, (callback) => { frames.push(callback); return frames.length; }, () => { frames.length = 0; });
  let now = 0;
  const step = (count = 1) => {
    for (let index = 0; index < count; index++) {
      const callback = frames.shift();
      if (!callback) return false;
      now += 1000 / 60;
      callback(now);
    }
    return true;
  };
  return { loop, source, frames, step, setRunning: (value: boolean) => { running = value; }, level };
}

describe('meter loop', () => {
  it('draws nothing and holds no taps while hidden', () => {
    const h = harness();
    const view = canvas();
    h.loop.attach('music', view.element, () => {});
    expect(h.frames).toHaveLength(0);
    h.loop.setVisible(true);
    expect(h.frames).toHaveLength(1);
    h.loop.setVisible(false);
    expect(h.frames).toHaveLength(0);
    expect(h.source.releases).toBe(1);
  });

  it('reads and paints every frame while audio runs', () => {
    const h = harness();
    const view = canvas();
    h.loop.attach('music', view.element, () => {});
    h.loop.resize('music', 12, 100, 2);
    h.loop.setVisible(true);
    h.step(10);
    expect(h.source.reads).toBe(10);
    expect(view.element.width).toBe(24);
    expect(view.calls.some((call) => call.startsWith('fill'))).toBe(true);
    expect(h.loop.running).toBe(true);
  });

  it('lets meters fall after playback stops, then releases the taps and stops', () => {
    const h = harness();
    const view = canvas();
    h.loop.attach('music', view.element, () => {});
    h.loop.resize('music', 12, 100, 1);
    h.loop.setVisible(true);
    h.step(30);
    h.setRunning(false);
    const reads = h.source.reads;
    let frames = 0;
    while (h.step()) frames++;
    expect(h.source.reads).toBe(reads);
    expect(frames).toBeGreaterThan(30);
    expect(frames).toBeLessThan(60 * 8);
    expect(h.loop.running).toBe(false);
    expect(h.source.releases).toBe(1);
  });

  it('does not repaint an unchanged meter', () => {
    const h = harness({ peak: 0, rms: 0 });
    const view = canvas();
    h.loop.attach('music', view.element, () => {});
    h.loop.resize('music', 12, 100, 1);
    const painted = view.calls.length;
    h.loop.setVisible(true);
    h.step(5);
    expect(view.calls.length).toBe(painted);
  });

  it('reports a clip once and clears it on request', () => {
    const h = harness({ peak: 1, rms: 0.7 });
    const view = canvas();
    const onClip = vi.fn();
    h.loop.attach('music', view.element, onClip);
    h.loop.setVisible(true);
    h.step(3);
    expect(onClip.mock.calls).toEqual([[true]]);
    h.level.peak = 0.2;
    h.loop.clearClip('music');
    h.step(3);
    expect(onClip.mock.calls).toEqual([[true], [false]]);
  });

  it('follows automation only while audio runs', () => {
    const h = harness();
    const onFrame = vi.fn();
    h.loop.onFrame = onFrame;
    h.loop.attach('music', canvas().element, () => {});
    h.loop.setVisible(true);
    h.step(4);
    expect(onFrame).toHaveBeenCalledTimes(4);
    h.setRunning(false);
    h.step(4);
    expect(onFrame).toHaveBeenCalledTimes(4);
  });
});
