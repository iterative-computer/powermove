import { describe, expect, it, vi } from 'vitest';
import { RecoveringStudioBackend } from './recovery';
import type { StudioBackend, StudioRenderRequest, StudioSceneDescription } from '../scene3d/studio-backend';
import { StudioBinaryWriter } from './buffers';

const request: StudioRenderRequest = { tier: 'live', size: { width: 1920, height: 1080 }, samples: 1 };
function backend(id: string): StudioBackend {
  return { id, initialize: vi.fn(async () => {}), setScene: vi.fn(), patchScene: vi.fn(), setTime: vi.fn(), setCamera: vi.fn(), setView: vi.fn(), dispose: vi.fn(), render: vi.fn(() => ({ source: {} as HTMLCanvasElement, width: 1920, height: 1080, samples: 1, version: id })) };
}
describe('native Studio fallback', () => {
  it('replays scene and time before rendering after a host crash', async () => {
    const native = backend('native'), portable = backend('webgpu'), invalidate = vi.fn();
    native.render = vi.fn(async () => { throw new Error('process exited'); });
    const recovering = new RecoveringStudioBackend(native, () => portable, { invalidate });
    await recovering.initialize();
    const scene = { version: 1 } as StudioSceneDescription;
    recovering.setScene(scene); recovering.setTime(2.5);
    expect((await recovering.render(request)).version).toBe('webgpu');
    expect(native.dispose).toHaveBeenCalledOnce();
    expect(portable.setScene).toHaveBeenCalledWith(scene);
    expect(portable.setTime).toHaveBeenCalledWith(2.5);
    expect(recovering.id).toBe('webgpu');
    expect(invalidate).toHaveBeenCalledOnce();
    recovering.dispose(); expect(portable.dispose).toHaveBeenCalledOnce();
  });
  it('starts the portable backend if the native binary cannot launch', async () => {
    const native = backend('native'), portable = backend('webgpu');
    native.initialize = vi.fn(async () => { throw new Error('not installed'); });
    const recovering = new RecoveringStudioBackend(native, () => portable, { invalidate: () => {} });
    await recovering.initialize(); expect(recovering.id).toBe('webgpu');
    recovering.dispose();
  });
  it('does not replace the renderer for a cancelled frame', async () => {
    const native = backend('native'), factory = vi.fn(() => backend('webgpu'));
    const controller = new AbortController(); controller.abort();
    const recovering = new RecoveringStudioBackend(native, factory, { invalidate: () => {} });
    await expect(recovering.render({ ...request, signal: controller.signal })).rejects.toThrow('cancelled');
    expect(factory).not.toHaveBeenCalled(); recovering.dispose();
  });
  it('copies only the selected bytes of a typed-array view', () => {
    const writer = new StudioBinaryWriter();
    expect(writer.append(new Uint8Array([9, 1, 2, 8]).subarray(1, 3))).toEqual({ offset: 0, length: 2 });
    expect(writer.append(new Uint8Array([3]))).toEqual({ offset: 2, length: 1 });
    expect([...writer.finish()]).toEqual([1, 2, 3]);
  });
});
