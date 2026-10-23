// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { COMPAT_ATTRIBUTES, CompatContextHost, compatContext, sourceRect } from './compat-context';

/** A WebGL2 stand-in recording the calls that matter (constants as WebGL2's). */
function fakeGl(canvas: unknown) {
  const calls: Array<[string, ...unknown[]]> = [];
  const params = new Map<number, unknown>([[0x8069, 'extension-texture'], [0x88EF, null], [0x9240, false], [0x9241, false], [0x9243, 0x9244], [0x0CF5, 4], [0x0CF2, 0], [0x0CF4, 0], [0x0CF3, 0]]);
  const enabled = new Set<number>([0x0C11]);
  const record = (name: string) => (...args: unknown[]) => { calls.push([name, ...args]); };
  const gl: any = {
    FRAMEBUFFER: 0x8D40, READ_FRAMEBUFFER: 0x8CA8, DRAW_FRAMEBUFFER: 0x8CA9, TEXTURE_2D: 0x0DE1, TEXTURE_BINDING_2D: 0x8069,
    PIXEL_UNPACK_BUFFER: 0x88EC, PIXEL_UNPACK_BUFFER_BINDING: 0x88EF, UNPACK_FLIP_Y_WEBGL: 0x9240, UNPACK_PREMULTIPLY_ALPHA_WEBGL: 0x9241,
    UNPACK_COLORSPACE_CONVERSION_WEBGL: 0x9243, BROWSER_DEFAULT_WEBGL: 0x9244, UNPACK_ALIGNMENT: 0x0CF5, UNPACK_ROW_LENGTH: 0x0CF2,
    UNPACK_SKIP_PIXELS: 0x0CF4, UNPACK_SKIP_ROWS: 0x0CF3, SCISSOR_TEST: 0x0C11, RASTERIZER_DISCARD: 0x8C89, RGBA8: 0x8058, RGBA: 0x1908,
    UNSIGNED_BYTE: 0x1401, COLOR_ATTACHMENT0: 0x8CE0, COLOR_BUFFER_BIT: 0x4000, NEAREST: 0x2600,
    canvas,
    getParameter: (p: number) => params.get(p),
    isEnabled: (p: number) => enabled.has(p),
    enable: (p: number) => { enabled.add(p); calls.push(['enable', p]); },
    disable: (p: number) => { enabled.delete(p); calls.push(['disable', p]); },
    createTexture: () => 'upload-texture', createFramebuffer: () => 'upload-fbo',
    bindBuffer: record('bindBuffer'), bindTexture: record('bindTexture'), pixelStorei: record('pixelStorei'),
    texImage2D: record('texImage2D'), framebufferTexture2D: record('framebufferTexture2D'),
    bindFramebuffer: record('bindFramebuffer'), deleteFramebuffer: record('deleteFramebuffer'),
    blitFramebuffer: record('blitFramebuffer'), readPixels: record('readPixels'), clear: record('clear'),
    drawArrays: record('drawArrays'), drawElements: record('drawElements'), copyTexImage2D: record('copyTexImage2D')
  };
  return { gl, calls, enabled };
}

function rig(options: { width?: number; height?: number } = {}) {
  const stage = document.createElement('div');
  document.body.append(stage);
  const bound = document.createElement('canvas');
  bound.width = options.width ?? 200; bound.height = options.height ?? 100;
  stage.append(bound);
  const presentation = document.createElement('canvas');
  presentation.id = 'engine-present';
  presentation.width = 400; presentation.height = 200;
  presentation.style.visibility = 'visible';
  stage.append(presentation);
  const overlay = document.createElement('canvas');
  overlay.id = 'overlay';
  stage.append(overlay);
  const made: any[] = [];
  const drawn: unknown[][] = [];
  const twoD = () => ({
    setTransform() {}, fillRect: (...a: unknown[]) => drawn.push(['fillRect', ...a]),
    drawImage: (...a: unknown[]) => drawn.push(['drawImage', ...a]),
    putImageData: (...a: unknown[]) => drawn.push(['putImageData', ...a]),
    set fillStyle(v: string) { drawn.push(['fillStyle', v]); }, globalCompositeOperation: 'source-over', imageSmoothingEnabled: true
  });
  let gl: ReturnType<typeof fakeGl> | null = null;
  const makeCanvas = (width: number, height: number) => {
    const canvas: any = { width, height, addEventListener: vi.fn() };
    canvas.getContext = vi.fn((type: string, attributes?: unknown) => {
      if (type === 'webgl2') { gl = fakeGl(canvas); canvas.attributes = attributes; return gl.gl; }
      return twoD();
    });
    made.push(canvas);
    return canvas;
  };
  const deferred: Array<() => void> = [];
  const env = {
    bound: vi.fn((): HTMLCanvasElement | null => null),
    engine: vi.fn(() => true),
    legacy: vi.fn(() => null),
    makeCanvas,
    presentation: (b: HTMLCanvasElement) => (b === bound && presentation.style.visibility === 'visible' ? presentation : null),
    defer: (fn: () => void) => deferred.push(fn)
  };
  const host = new CompatContextHost(env);
  // The layer's own 2D context (happy-dom has none).
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(((type: string) => (type === '2d' ? twoD() : null)) as any);
  // happy-dom has no layout: the presentation covers the stage at twice the bound canvas's density.
  presentation.getBoundingClientRect = () => ({ left: 0, top: 0, width: 200, height: 100, right: 200, bottom: 100, x: 0, y: 0, toJSON() {} }) as DOMRect;
  bound.getBoundingClientRect = () => ({ left: 0, top: 0, width: 200, height: 100, right: 200, bottom: 100, x: 0, y: 0, toJSON() {} }) as DOMRect;
  return { host, env, bound, presentation, overlay, stage, made, drawn, deferred, gl: () => gl!, flush: () => { for (const fn of deferred.splice(0)) fn(); } };
}

describe('render.gl.context, the compatibility context', () => {
  it('reserves a deferred compositor context before creating a mirror, only on explicit context access', () => {
    const r = rig();
    const order: string[] = [];
    r.env.bound.mockReturnValue(r.bound);
    const host = new CompatContextHost({
      ...r.env,
      beforeContext: (bound) => { expect(bound).toBe(r.bound); order.push('reserve WebGL'); },
      makeCanvas: (...args) => { order.push('compatibility canvas'); return r.env.makeCanvas(...args); }
    });
    expect(order).toEqual([]);
    expect(host.context()).not.toBeNull();
    expect(order[0]).toBe('reserve WebGL');
    host.context();
    expect(order.filter(stage => stage === 'reserve WebGL')).toHaveLength(1);
  });

  it('is the compositor\'s context with the engine off, as today', () => {
    const gl = { canvas: {} };
    expect(compatContext({ GL: { gl } })).toBe(gl);
    expect(compatContext({ GL: {} })).toBeNull();
    const host = new CompatContextHost({ bound: () => null, engine: () => false, legacy: () => gl as any });
    expect(host.context()).toBe(gl);
  });

  it('selects the preserved engine surface by the presented frame tag, not the newest arrival', () => {
    const r = rig(); r.env.bound.mockReturnValue(r.bound); r.host.context();
    const first = { seq: 1, revision: 7, viewSeq: 3 };
    const second = { seq: 2, revision: 7, viewSeq: 3 };
    r.host.captureFrame(first, { displayWidth: 400, displayHeight: 200 } as VideoFrame);
    const saved = r.made.at(-1);
    r.host.captureFrame(second, { displayWidth: 400, displayHeight: 200 } as VideoFrame);
    r.drawn.length = 0;
    r.host.selectFrame(first); r.host.presented();
    expect(r.drawn).toContainEqual(['drawImage', saved, 0, 0, 400, 200, 0, 0, 200, 100]);
  });

  it('preserves presented pixels for reads after the WebGPU swapchain has been cleared', () => {
    const r = rig();
    r.env.bound.mockReturnValue(r.bound);
    const gl: any = r.host.context();
    r.host.presented();
    const captured = r.made.find((canvas) => canvas !== r.made[0]);
    expect(captured).toBeDefined();
    r.drawn.length = 0;
    gl.readPixels(0, 0, 1, 1, 0x1908, 0x1401, new Uint8Array(4));
    expect(r.drawn.some((d) => d[0] === 'drawImage' && d[1] === r.presentation)).toBe(false);
    expect(r.drawn).toContainEqual(['drawImage', captured, 0, 0]);
  });

  it('is null until a viewer binds a canvas, then one context on an offscreen canvas with today\'s attributes', () => {
    const r = rig();
    expect(r.host.context()).toBeNull();
    r.env.bound.mockReturnValue(r.bound);
    const gl = r.host.context()!;
    expect(gl).not.toBeNull();
    expect(r.host.context()).toBe(gl);
    // `canvas` is the bound canvas (frozen viewers find their stage from it).
    expect(gl.canvas).toBe(r.bound);
    expect((gl as any).compatCanvas).toBe(r.made[0]);
    expect(r.made[0].attributes).toEqual(COMPAT_ATTRIBUTES);
    expect(r.made[0].attributes).toMatchObject({ alpha: false, preserveDrawingBuffer: true, antialias: false, premultipliedAlpha: true });
    expect([r.made[0].width, r.made[0].height]).toEqual([200, 100]);
    // A new canvas bound: a new context, as GL.init made one per canvas.
    const other = document.createElement('canvas');
    other.width = 64; other.height = 32;
    r.env.bound.mockReturnValue(other);
    const next = r.host.context()!;
    expect(next).not.toBe(gl);
    expect(next.canvas).toBe(other);
  });

  it('follows the bound canvas\'s size, as GL.resize resized the compositor\'s canvas', () => {
    const r = rig();
    r.env.bound.mockReturnValue(r.bound);
    r.host.context();
    r.bound.width = 300; r.bound.height = 150;
    r.host.context();
    expect([r.made[0].width, r.made[0].height]).toEqual([300, 150]);
  });

  it('reads back the presented frame: the engine\'s canvas under the bound canvas is uploaded over black before the first read', () => {
    const r = rig();
    r.env.bound.mockReturnValue(r.bound);
    const gl: any = r.host.context();
    r.host.presented();
    const out = new Uint8Array(4);
    gl.readPixels(0, 0, 1, 1, 0x1908, 0x1401, out);
    const { calls } = r.gl();
    // Black, then the presentation's region (its own pixels: twice the density) scaled to the backing store.
    expect(r.drawn).toContainEqual(['fillStyle', '#000']);
    expect(r.drawn).toContainEqual(['fillRect', 0, 0, 200, 100]);
    expect(r.drawn).toContainEqual(['drawImage', r.presentation, 0, 0, 400, 200, 0, 0, 200, 100]);
    const names = calls.map((c) => c[0]);
    expect(names.indexOf('texImage2D')).toBeLessThan(names.indexOf('readPixels'));
    expect(calls).toContainEqual(['blitFramebuffer', 0, 0, 200, 100, 0, 0, 200, 100, 0x4000, 0x2600]);
    expect(calls).toContainEqual(['pixelStorei', 0x9240, true]);
    // The extension's state is back: framebuffers, texture, unpack flags, scissor.
    const binds = calls.filter((c) => c[0] === 'bindFramebuffer');
    expect(binds.slice(-2)).toEqual([['bindFramebuffer', 0x8CA8, null], ['bindFramebuffer', 0x8CA9, null]]);
    expect(calls.filter((c) => c[0] === 'bindTexture').at(-1)).toEqual(['bindTexture', 0x0DE1, 'extension-texture']);
    expect(calls.filter((c) => c[0] === 'pixelStorei' && c[1] === 0x9240).at(-1)).toEqual(['pixelStorei', 0x9240, false]);
    expect(r.gl().enabled.has(0x0C11)).toBe(true);
    // Once per presented frame.
    gl.readPixels(0, 0, 1, 1, 0x1908, 0x1401, out);
    expect(r.host.stats.uploads).toBe(1);
    r.host.presented();
    gl.readPixels(0, 0, 1, 1, 0x1908, 0x1401, out);
    expect(r.host.stats.uploads).toBe(2);
  });

  it('does not upload for reads and draws of the extension\'s own framebuffers', () => {
    const r = rig();
    r.env.bound.mockReturnValue(r.bound);
    const gl: any = r.host.context();
    gl.bindFramebuffer(gl.FRAMEBUFFER, 'own');
    gl.clear(0x4000);
    gl.drawArrays(4, 0, 3);
    gl.readPixels(0, 0, 1, 1, 0x1908, 0x1401, new Uint8Array(4));
    r.flush();
    expect(r.host.stats.uploads).toBe(0);
    expect(r.host.shownLayer).toBeNull();
    // Deleting the bound framebuffer makes the default one current again, as in WebGL.
    gl.deleteFramebuffer('own');
    gl.readPixels(0, 0, 1, 1, 0x1908, 0x1401, new Uint8Array(4));
    expect(r.host.stats.uploads).toBe(1);
  });

  it('shows drawing into the default framebuffer over the engine\'s canvas, under the overlay, until the next frame', () => {
    const r = rig();
    r.env.bound.mockReturnValue(r.bound);
    const gl: any = r.host.context();
    r.host.presented();
    gl.clear(0x4000);
    gl.drawArrays(4, 0, 3);
    // Drawn on the frame (uploaded first), shown after the task.
    expect(r.host.stats.uploads).toBe(1);
    expect(r.host.shownLayer).toBeNull();
    r.flush();
    const layer = r.host.shownLayer!;
    expect(layer).not.toBeNull();
    expect(layer.previousSibling).toBe(r.presentation);
    expect(layer.nextSibling).toBe(r.overlay);
    expect([layer.width, layer.height]).toEqual([200, 100]);
    expect(layer.style.pointerEvents).toBe('none');
    // The next presented frame replaces the drawing, as today's next present.
    r.host.presented();
    expect(r.host.shownLayer).toBeNull();
    expect(r.host.stats).toMatchObject({ shown: 1, hidden: 1 });
  });

  it('reads black where no engine frame is on screen (a cleared frame)', () => {
    const r = rig();
    r.presentation.style.visibility = 'hidden';
    r.env.bound.mockReturnValue(r.bound);
    const gl: any = r.host.context();
    gl.readPixels(0, 0, 1, 1, 0x1908, 0x1401, new Uint8Array(4));
    expect(r.drawn).toContainEqual(['fillRect', 0, 0, 200, 100]);
    expect(r.drawn.some((d) => d[0] === 'drawImage')).toBe(false);
    expect(r.host.stats.uploads).toBe(1);
  });

  it('maps the bound canvas to the engine canvas\'s pixels', () => {
    const source = document.createElement('canvas');
    source.width = 800; source.height = 400;
    source.getBoundingClientRect = () => ({ left: 10, top: 20, width: 400, height: 200 }) as DOMRect;
    const bound = document.createElement('canvas');
    bound.getBoundingClientRect = () => ({ left: 60, top: 40, width: 100, height: 50 }) as DOMRect;
    expect(sourceRect(source, bound)).toEqual({ sx: 100, sy: 40, sw: 200, sh: 100 });
  });
});

describe('render.gl.init before the engine connection is up', () => {
  it('binds the canvas the viewer passed once the engine\'s binding exists', async () => {
    const { bindLate } = await import('./install');
    const canvas = document.createElement('canvas');
    document.body.append(canvas);
    const binding = { bound: null as HTMLCanvasElement | null, bind: vi.fn((c: HTMLCanvasElement) => { binding.bound = c; return true; }) };
    expect(bindLate({}, canvas)).toBe(false);
    expect(bindLate({ EngineCanvas: binding }, null)).toBe(false);
    expect(bindLate({ EngineCanvas: binding }, canvas)).toBe(true);
    expect(binding.bound).toBe(canvas);
    // Bound already, or a canvas that left the document: nothing to do.
    expect(bindLate({ EngineCanvas: binding }, canvas)).toBe(false);
    canvas.remove();
    expect(bindLate({ EngineCanvas: { bound: null, bind: binding.bind } }, canvas)).toBe(false);
  });

  it('copies each presented frame, then the drawing over it, into the bound canvas: context.canvas reads what today\'s canvas held', () => {
    const r = rig();
    r.env.bound.mockReturnValue(r.bound);
    const gl = r.host.context()!;
    r.drawn.length = 0;
    r.host.presented();
    // The frame under the bound canvas (the presentation at twice the density), over black.
    expect(r.drawn).toContainEqual(['drawImage', r.presentation, 0, 0, 400, 200, 0, 0, 200, 100]);
    expect(r.host.stats.mirrored).toBe(1);
    // A drawing over it lands in the bound canvas too.
    gl.clear(0x4000);
    r.flush();
    expect(r.drawn.at(-1)).toEqual(['drawImage', r.made[0], 0, 0, 200, 100]);
    expect(r.host.stats.mirrored).toBe(2);
  });

  it('runs a WebM export frame\'s handlers over that frame at the export\'s size and returns what they drew, top-down', () => {
    const r = rig();
    r.env.bound.mockReturnValue(r.bound);
    const gl: any = r.host.context()!;
    (globalThis as any).ImageData ??= class { constructor(public data: Uint8ClampedArray, public width: number, public height: number) {} };
    const frame = { width: 2, height: 2, data: new Uint8Array([1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255, 4, 4, 4, 255]) };
    // No handler draws: the engine's frame stands.
    let sizes: number[] = [];
    expect(r.host.exportFrame(frame, () => { sizes = [r.made[0].width, r.made[0].height]; })).toBeNull();
    expect(sizes).toEqual([2, 2]);
    expect([r.made[0].width, r.made[0].height]).toEqual([200, 100]);
    // A handler draws: the frame is uploaded under it, and read back flipped to top-down.
    // (the fake's readPixels writes nothing: the pixels themselves are measured in e2e/engine-compat-context.spec.ts)
    const out = r.host.exportFrame(frame, () => { gl.clear(0x4000); });
    expect(r.drawn.some((d) => d[0] === 'putImageData')).toBe(true);
    expect(r.made[0].width).toBe(200);
    expect(out).not.toBeNull();
    expect(out!.length).toBe(16);
    expect([...out!].filter((_, i) => i % 4 === 3)).toEqual([255, 255, 255, 255]);
    // The drawing went to the export, not the viewer's layer.
    r.flush();
    expect(r.host.shownLayer).toBeNull();
    expect(r.host.stats.exportFrames).toBe(1);
  });
});
