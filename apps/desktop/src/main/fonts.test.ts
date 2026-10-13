import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('@powermove/macos-haptics', () => ({
  fontFamilies: vi.fn(() => ['New font']), fontRegistryVersion: vi.fn(() => null)
}));
import { fontFamilies, fontRegistryVersion } from '@powermove/macos-haptics';
import { registerFontsIpc } from './fonts';
import { IPC } from '../shared/ipc';

beforeEach(() => {
  vi.mocked(fontFamilies).mockReset().mockReturnValue(['New font']);
  vi.mocked(fontRegistryVersion).mockReset().mockReturnValue(null);
});

function reader() {
  const handle = vi.fn(), sender = {} as any;
  registerFontsIpc({ handle }, { isTrustedSenderContents: value => value === sender });
  const read = handle.mock.calls[0]![1];
  return () => read({ sender });
}

it('reads a fresh native font list only for trusted editor requests', () => {
  const handle = vi.fn();
  const sender = {} as any;
  registerFontsIpc({ handle }, { isTrustedSenderContents: value => value === sender });
  expect(handle.mock.calls[0]![0]).toBe(IPC.fontFamilies);
  const read = handle.mock.calls[0]![1];
  expect(read({ sender: {} })).toBeNull();
  expect(fontFamilies).not.toHaveBeenCalled();
  expect(fontRegistryVersion).not.toHaveBeenCalled();
  expect(read({ sender })).toEqual(['New font']);
  expect(read({ sender })).toEqual(['New font']);
  expect(fontFamilies).toHaveBeenCalledTimes(2);
});

it('reuses a stable native snapshot without exposing its mutable array', () => {
  vi.mocked(fontRegistryVersion).mockReturnValue(0);
  const read = reader();
  const first = read(); first.push('Caller mutation');
  const next = read(); next.push('Another mutation');
  expect(read()).toEqual(['New font']);
  expect(fontFamilies).toHaveBeenCalledTimes(1);
});

it('refreshes on a native font registry change across editor requests', () => {
  vi.mocked(fontRegistryVersion).mockReturnValue(3);
  const read = reader();
  expect(read()).toEqual(['New font']);
  vi.mocked(fontFamilies).mockReturnValue(['Installed font']);
  vi.mocked(fontRegistryVersion).mockReturnValue(4);
  expect(read()).toEqual(['Installed font']);
  expect(read()).toEqual(['Installed font']);
  expect(fontFamilies).toHaveBeenCalledTimes(2);
});

it('retries when the registry changes during enumeration', () => {
  vi.mocked(fontRegistryVersion).mockReturnValueOnce(1).mockReturnValue(2);
  const read = reader(); read(); read(); read();
  expect(fontFamilies).toHaveBeenCalledTimes(2);
});

it('retries unavailable and empty snapshots even when the revision stays stable', () => {
  vi.mocked(fontRegistryVersion).mockReturnValue(0);
  vi.mocked(fontFamilies).mockReturnValueOnce(null).mockReturnValueOnce([]).mockReturnValue(['Recovered']);
  const read = reader();
  expect(read()).toBeNull(); expect(read()).toEqual([]);
  expect(read()).toEqual(['Recovered']); expect(read()).toEqual(['Recovered']);
  expect(fontFamilies).toHaveBeenCalledTimes(3);
});

it('recovers after an enumeration error without caching the failed revision', () => {
  vi.mocked(fontRegistryVersion).mockReturnValue(0);
  const read = reader(); read();
  vi.mocked(fontRegistryVersion).mockReturnValue(1);
  vi.mocked(fontFamilies).mockImplementationOnce(() => { throw new Error('Temporary native failure'); });
  expect(() => read()).toThrow('Temporary native failure');
  expect(read()).toEqual(['New font']); expect(read()).toEqual(['New font']);
  expect(fontFamilies).toHaveBeenCalledTimes(3);
});
