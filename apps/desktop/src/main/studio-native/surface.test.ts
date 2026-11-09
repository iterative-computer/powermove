import { beforeEach, describe, expect, it, vi } from 'vitest';
const texture = vi.hoisted(() => ({ import: vi.fn(), send: vi.fn() }));
vi.mock('electron', () => ({ sharedTexture: { importSharedTexture: texture.import, sendSharedTexture: texture.send } }));
import { StudioSurfaceReceiver } from './surface';

beforeEach(() => { texture.import.mockReset(); texture.send.mockReset().mockResolvedValue(undefined); });
function setup() {
  const surface = {handle: Buffer.alloc(8), width: 4, height: 4};
  const addon = { create: () => ({port:1,service:'test'}), take: vi.fn(() => surface), release: vi.fn(), close: vi.fn() };
  const receiver = new (StudioSurfaceReceiver as any)(addon) as StudioSurfaceReceiver;
  const contents = {isDestroyed: () => false, mainFrame:{}} as any;
  const reply = {result:{surface:42,width:4,height:4,time:0},binary:new Uint8Array(),generation:1,roundTripMs:0};
  const released = vi.fn();
  return {surface,addon,receiver,contents,reply,released,present:()=>receiver.present(contents,'studio-1',reply,123,released)};
}

describe('Studio shared-texture lifetime', () => {
  it('returns the host slot when receiving a Mach message fails', async () => {
    const s=setup();s.addon.take.mockImplementation(()=>{throw new Error('receive failed');});
    await expect(s.present()).rejects.toThrow('receive failed');
    expect(s.released).toHaveBeenCalledOnce();expect(s.addon.release).not.toHaveBeenCalled();s.receiver.close();
  });
  it('releases a retained surface and host slot if import fails', async () => {
    const s=setup();texture.import.mockImplementation(()=>{throw new Error('import failed');});
    await expect(s.present()).rejects.toThrow('import failed');
    expect(s.addon.take).toHaveBeenCalledWith(1,42,123);
    expect(s.addon.release).toHaveBeenCalledWith(s.surface.handle);expect(s.released).toHaveBeenCalledOnce();s.receiver.close();
  });
  it('keeps a transferred surface alive across receiver close and acknowledges it once', async () => {
    const s=setup();let finish!:()=>void;
    const imported={release:vi.fn()};
    texture.import.mockImplementation(options=>{finish=options.allReferencesReleased;return imported;});
    await s.present();expect(imported.release).toHaveBeenCalledOnce();expect(s.addon.release).not.toHaveBeenCalled();
    s.receiver.close();expect(s.addon.close).toHaveBeenCalledOnce();
    finish();finish();expect(s.addon.release).toHaveBeenCalledOnce();expect(s.released).toHaveBeenCalledOnce();
  });
  it('releases the imported reference on a failed transfer', async () => {
    const s=setup();const imported={release:vi.fn(()=>texture.import.mock.calls[0]![0].allReferencesReleased())};
    texture.import.mockReturnValue(imported);texture.send.mockRejectedValue(new Error('frame gone'));
    await expect(s.present()).rejects.toThrow('frame gone');
    expect(imported.release).toHaveBeenCalledOnce();expect(s.addon.release).toHaveBeenCalledOnce();expect(s.released).toHaveBeenCalledOnce();s.receiver.close();
  });
});
