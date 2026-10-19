import { afterEach, expect, it, vi } from 'vitest';
import { pinNativeDocument } from './pin-document';
const old = (globalThis as any).PM;
afterEach(() => { (globalThis as any).PM = old; });
function fixture(joined: { id: string; revision: number; layers: any[] }) {
  const PM: any = { version: '1', proj: { id: 'Pone', revision: 0, layers: [] }, sel: { layers: [] },
    store: { set: vi.fn() }, openProjectHere: vi.fn(async () => true), EngineDoc: { whenReady: Promise.resolve(true) },
    RemoteSync: { refresh: vi.fn(async () => { PM.proj = joined; }), flush: vi.fn(async () => {}) },
    replaceProject: vi.fn((doc) => { PM.proj = doc; }) };
  (globalThis as any).PM = PM;
  return PM;
}
it('loads the authoritative worker seed when opening the already active ID retained an empty boot copy', async () => {
  const PM = fixture({ id: 'Pone', revision: 0, layers: [] });
  const seed = { id: 'Pone', revision: 2, layers: [{ id: 'Lone' }] };
  await pinNativeDocument(seed);
  expect(PM.proj).toEqual(seed);
  expect(PM.RemoteSync.flush).toHaveBeenCalledOnce();
});
it('keeps a newer joined document rather than rolling it back to the earlier worker snapshot', async () => {
  const joined = { id: 'Pone', revision: 3, layers: [{ id: 'Lone' }, { id: 'Ltwo' }] };
  const PM = fixture(joined);
  await pinNativeDocument({ id: 'Pone', revision: 2, layers: [{ id: 'Lone' }] });
  expect(PM.proj).toEqual(joined);
  expect(PM.replaceProject).not.toHaveBeenCalled();
});
it('rejects an unavailable pinned project before dispatching tools', async () => {
  const PM = fixture({ id: 'Pone', revision: 0, layers: [] }); PM.openProjectHere.mockResolvedValue(false);
  await expect(pinNativeDocument({ id: 'Pone', revision: 2, layers: [] })).rejects.toThrow('The pinned project could not be opened.');
  expect(PM.RemoteSync.refresh).not.toHaveBeenCalled();
});
