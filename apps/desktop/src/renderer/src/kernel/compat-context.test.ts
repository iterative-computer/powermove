import { describe, expect, it, vi } from 'vitest';
import { compatContext } from './compat-context';

describe('render.gl.context with and without the engine', () => {
  it('is the compositor\'s context without the engine, as today', () => {
    const gl = { canvas: {} };
    expect(compatContext({ GL: { gl } })).toBe(gl);
    expect(compatContext({ GL: {} })).toBeNull();
  });

  it('is null until a viewer binds a canvas, then that canvas\'s own WebGL2 context', () => {
    const context = { canvas: null as unknown };
    const canvas = { getContext: vi.fn(() => context) };
    context.canvas = canvas;
    const PM: any = { EngineRender: {}, EngineCanvas: { bound: null }, GL: {} };
    expect(compatContext(PM)).toBeNull();
    PM.EngineCanvas.bound = canvas;
    const got = compatContext(PM);
    expect(got).toBe(context);
    expect((got as any).canvas).toBe(canvas);
    expect(canvas.getContext).toHaveBeenCalledWith('webgl2', expect.objectContaining({ preserveDrawingBuffer: true }));
  });

  it('is null rather than throwing when the canvas has another kind of context', () => {
    const canvas = { getContext: () => { throw new Error('in use'); } };
    expect(compatContext({ EngineRender: {}, EngineCanvas: { bound: canvas } })).toBeNull();
  });
});
