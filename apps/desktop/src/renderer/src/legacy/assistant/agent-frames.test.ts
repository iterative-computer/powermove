import { describe, expect, it, vi } from 'vitest';
import { captureAgentFrames } from './agent-frames';

describe('agent observations', () => {
  it('pins all batches before edits and never touches playback or the compatibility capture', async () => {
    const PM = { proj: { revision: 9, name: 'before' }, playing: true, pause: vi.fn(), play: vi.fn(),
      Export: { snapshot: vi.fn(), snapshotAsync: vi.fn() },
      EngineDoc: { whenReady: Promise.resolve(true) },
      HostMedia: { flushBindings: vi.fn(async () => {}) }, EngineDefinitions: { settle: vi.fn(async () => {}) } };
    const agentFrames = vi.fn(async request => {
      expect(PM.HostMedia.flushBindings).toHaveBeenCalled();
      expect(PM.EngineDefinitions.settle).toHaveBeenCalled();
      PM.proj = { revision: 10, name: 'after' };
      return { status: 'rendered', revision: 1, queuedMs: 0, images: request.times.map((time: number) => ({ time, data: new Uint8Array([1, 2]), mimeType: 'image/jpeg', width: 480, height: 270 })) };
    });
    const times = [0, 1, 2, 3, 4, 5];
    const result = await captureAgentFrames(PM, { agentFrames } as never, times, 480);
    expect(agentFrames).toHaveBeenCalledTimes(2);
    for (const [request] of agentFrames.mock.calls) expect(request).toMatchObject({ revision: 9, project: '{"revision":9,"name":"before"}', width: 480 });
    expect(result).toEqual({ times, images: times.map(() => 'data:image/jpeg;base64,AQI=') });
    expect(PM.playing).toBe(true);
    for (const spy of [PM.pause, PM.play, PM.Export.snapshot, PM.Export.snapshotAsync]) expect(spy).not.toHaveBeenCalled();
  });

  it('propagates native failures and names a missing batch service', async () => {
    await expect(captureAgentFrames({ proj: {} }, undefined, [0], 720)).rejects.toThrow('agent-frames-unavailable');
    const agentFrames = vi.fn(async () => { throw new Error('mediaOffline: clip'); });
    await expect(captureAgentFrames({ proj: {} }, { agentFrames } as never, [0], 720)).rejects.toThrow('mediaOffline: clip');
  });
});
