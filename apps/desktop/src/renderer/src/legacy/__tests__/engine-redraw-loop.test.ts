import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from './make-pm';

/* While the engine presents, a paused draw announces the frame on screen
   again (`PM.EngineRedraw`). A draw raised by a listener of that very
   announcement must not answer itself, or a `frame:rendered` listener that
   invalidates announces the same frame at every animation frame. */
describe('the paused engine redraw', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('answers a real draw once, and not the draws its own announcement raised', () => {
    const frames: Array<(t: number) => void> = [];
    const raf = (fn: (t: number) => void) => { frames.push(fn); return frames.length; };
    vi.stubGlobal('window', { performance, requestAnimationFrame: raf, cancelAnimationFrame: () => {}, addEventListener() {}, document: { addEventListener() {}, visibilityState: 'visible' } });
    vi.stubGlobal('requestAnimationFrame', raf);
    const PM: any = makePM('core/engine');
    PM.proj = { fps: 30, dur: 10, layers: [] };
    PM.assets.loading = new Set();
    PM.Audio = { pause() {}, start() {}, seek() {}, tick() {} };
    let announcing = false;
    const announced: string[] = [];
    PM.EnginePresentedEvents = { get announcing() { return announcing; } };
    PM.EngineRedraw = () => {
      announcing = true;
      try { announced.push('presented'); PM.bus.emit('frame:rendered'); } finally { announcing = false; }
      return true;
    };
    const overlays: number[] = [];
    PM.bus.on('overlay', () => { overlays.push(1); PM.bus.emit('draw'); });
    // An extension that redraws whenever a frame is rendered.
    PM.bus.on('frame:rendered', () => PM.bus.emit('draw'));
    PM.setTransportDriver({ active: () => true, play() {}, shuttle() {}, playAround() {}, setAudioScrub() {}, pause() {}, seek() {}, setLoop() {},
      now: () => null, onActiveChange: () => () => {} });
    const tick = (t: number) => { const next = frames.splice(0); for (const fn of next) fn(t); };
    tick(0);
    announced.length = 0; overlays.length = 0;
    PM.bus.emit('draw');
    for (let i = 1; i <= 10; i++) tick(i * 16);
    expect(announced).toEqual(['presented']);
    expect(overlays).toEqual([1]);
    // A later real draw is answered again.
    PM.bus.emit('draw');
    tick(200); tick(216);
    expect(announced).toEqual(['presented', 'presented']);
  });
});
