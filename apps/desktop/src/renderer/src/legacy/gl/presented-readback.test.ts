import { afterEach, expect, it, vi } from 'vitest';
import * as opaque from './opaque-readback';
import { createPresentedReadback } from './presented-readback';

afterEach(() => vi.restoreAllMocks());
function fixture() {
  const state = new Map<number, unknown>([[1, { texture: true }], [2, { draw: true }], [3, { read: true }]]);
  const gl: any = { TEXTURE_BINDING_2D: 1, DRAW_FRAMEBUFFER_BINDING: 2, READ_FRAMEBUFFER_BINDING: 3,
    TEXTURE_2D: 4, DRAW_FRAMEBUFFER: 5, READ_FRAMEBUFFER: 6, FRAMEBUFFER_COMPLETE: 7,
    RGBA8: 8, RGBA: 9, UNSIGNED_BYTE: 10, TEXTURE_MIN_FILTER: 11, TEXTURE_MAG_FILTER: 12, LINEAR: 13, COLOR_ATTACHMENT0: 14, COLOR_BUFFER_BIT: 15,
    isContextLost: () => false, getParameter: (key: number) => state.get(key),
    createTexture: vi.fn(() => ({})), createFramebuffer: vi.fn(() => ({})),
    bindTexture: vi.fn((_target, value) => state.set(1, value)),
    bindFramebuffer: vi.fn((target, value) => state.set(target === 5 ? 2 : 3, value)),
    texImage2D: vi.fn(), texParameteri: vi.fn(), framebufferTexture2D: vi.fn(),
    checkFramebufferStatus: vi.fn(() => 7), deleteTexture: vi.fn(), deleteFramebuffer: vi.fn(), blitFramebuffer: vi.fn() };
  return { gl, state, before: [...state] };
}

it('copies only the displayed canvas, flips rows on the GPU and restores framebuffer/texture bindings', () => {
  const { gl, state, before } = fixture(), read = vi.fn(), dispose = vi.fn();
  const ticket: any = { readInto: vi.fn(), dispose: vi.fn() };
  vi.spyOn(opaque, 'createOpaqueReadback').mockImplementation((_gl, _w, _h, capture) => ({ begin(time, options = {}) { capture(time, options, read); return ticket; }, dispose }));
  const session = createPresentedReadback(gl, { width: 640, height: 360 } as HTMLCanvasElement, 320, 180, () => true)!;
  expect([...state]).toEqual(before);
  expect(session.begin(0)).toBe(ticket);
  expect(gl.blitFramebuffer).toHaveBeenCalledWith(0, 0, 640, 360, 0, 180, 320, 0, 15, 13);
  expect(read).toHaveBeenCalledOnce(); expect([...state]).toEqual(before);
  session.dispose(); session.dispose();
  expect(gl.deleteTexture).toHaveBeenCalledOnce(); expect(gl.deleteFramebuffer).toHaveBeenCalledOnce();
});

it.each(['framebuffer', 'incomplete', 'buffer', 'throw'])('cleans up a failed %s allocation and preserves GPU state', failure => {
  const { gl, state, before } = fixture();
  if (failure === 'framebuffer') gl.createFramebuffer.mockReturnValue(null);
  if (failure === 'incomplete') gl.checkFramebufferStatus.mockReturnValue(0);
  const factory = vi.spyOn(opaque, 'createOpaqueReadback').mockImplementation(() => { if (failure === 'throw') throw new Error('GPU allocation failed'); return null; });
  expect(createPresentedReadback(gl, {} as HTMLCanvasElement, 320, 180, () => true)).toBeNull();
  expect(gl.deleteTexture).toHaveBeenCalledOnce();
  expect(gl.deleteFramebuffer).toHaveBeenCalledTimes(failure === 'framebuffer' ? 0 : 1);
  expect([...state]).toEqual(before);
  expect(factory).toHaveBeenCalledTimes(['buffer', 'throw'].includes(failure) ? 1 : 0);
});

it('rejects oversized captures before allocating GPU memory', () => {
  const { gl } = fixture();
  expect(createPresentedReadback(gl, {} as HTMLCanvasElement, 321, 180, () => true)).toBeNull();
  expect(gl.createTexture).not.toHaveBeenCalled();
});
