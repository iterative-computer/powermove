import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from './make-pm';

/* While the engine presents, a paused draw announces the frame on screen
   again (`PM.EngineRedraw`). A draw raised by a listener of that very
   announcement must not answer itself, or a `frame:rendered` listener that
   invalidates announces the same frame at every animation frame. */
describe('the paused engine redraw', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('answers a real draw once; a draw its own announcement raised redraws only the overlay', () => {
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
      return 'announced';
    };
    const overlays: number[] = [];
    let animating = false;
    // An overlay that animates by invalidating from its own draw.
    PM.bus.on('overlay', () => { overlays.push(1); if (animating) PM.bus.emit('draw'); });
    // An extension that redraws whenever a frame is rendered.
    PM.bus.on('frame:rendered', () => PM.bus.emit('draw'));
    PM.setTransportDriver({ active: () => true, play() {}, shuttle() {}, playAround() {}, setAudioScrub() {}, pause() {}, seek() {}, setLoop() {},
      now: () => null, onActiveChange: () => () => {} });
    const tick = (t: number) => { const next = frames.splice(0); for (const fn of next) fn(t); };
    tick(0);
    announced.length = 0; overlays.length = 0;
    PM.bus.emit('draw');
    for (let i = 1; i <= 10; i++) tick(i * 16);
    // The frame:rendered listener's draw redraws the overlay once, and
    // announces nothing again.
    expect(announced).toEqual(['presented']);
    expect(overlays).toEqual([1, 1]);
    // A later real draw is answered again.
    PM.bus.emit('draw');
    tick(200); tick(216);
    expect(announced).toEqual(['presented', 'presented']);
    // An animating overlay is drawn at every animation frame, as today's
    // frame loop draws it, without announcing the same frame again.
    announced.length = 0; overlays.length = 0; animating = true;
    PM.bus.emit('draw');
    for (let i = 1; i <= 10; i++) tick(300 + i * 16);
    expect(announced).toEqual(['presented']);
    expect(overlays.length).toBe(10);
    animating = false;
    tick(500); tick(516);
    const settled = overlays.length;
    tick(532); tick(548);
    expect(overlays.length).toBe(settled);
  });
  it('draws no overlay ahead of a frame on its way, and draws it once none landed in time', () => {
    const frames: Array<(t: number) => void> = [];
    const raf = (fn: (t: number) => void) => { frames.push(fn); return frames.length; };
    vi.stubGlobal('window', { performance, requestAnimationFrame: raf, cancelAnimationFrame: () => {}, addEventListener() {}, document: { addEventListener() {}, visibilityState: 'visible' } });
    vi.stubGlobal('requestAnimationFrame', raf);
    const PM: any = makePM('core/engine');
    PM.proj = { fps: 30, dur: 10, layers: [] };
    PM.assets.loading = new Set();
    PM.Audio = { pause() {}, start() {}, seek() {}, tick() {} };
    PM.EnginePresentedEvents = { announcing: false };
    // A scrub step: the engine presents the new time's frame later.
    let outcome = 'coming';
    PM.EngineRedraw = () => outcome;
    const overlays: number[] = [];
    PM.bus.on('overlay', () => overlays.push(1));
    PM.setTransportDriver({ active: () => true, play() {}, shuttle() {}, playAround() {}, setAudioScrub() {}, pause() {}, seek() {}, setLoop() {},
      now: () => null, onActiveChange: () => () => {} });
    const tick = (t: number) => { const next = frames.splice(0); for (const fn of next) fn(t); };
    tick(0);
    overlays.length = 0;
    PM.bus.emit('draw');
    for (let i = 1; i <= 5; i++) tick(i * 16);
    expect(overlays).toEqual([]);
    // The frame lands: the presentation announces it (`preview:presented`,
    // then `overlay`); the loop draws no second overlay for it.
    PM.bus.emit('preview:presented', {});
    PM.bus.emit('overlay');
    for (let i = 6; i <= 60; i++) tick(i * 16);
    expect(overlays).toEqual([1]);
    // A frame that never lands: the overlay is drawn once the hold is over.
    overlays.length = 0;
    PM.bus.emit('draw');
    tick(1000); tick(1016); tick(1400);
    expect(overlays).toEqual([]);
    tick(1520);
    expect(overlays).toEqual([1]);
    tick(1536); tick(2100);
    expect(overlays).toEqual([1]);
    // Nothing new coming: the overlay is drawn at the next animation frame.
    outcome = 'announced';
    PM.bus.emit('draw');
    tick(2116);
    expect(overlays).toEqual([1, 1]);
  });
});
