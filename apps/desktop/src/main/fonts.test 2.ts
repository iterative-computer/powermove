import { expect, it, vi } from 'vitest';
vi.mock('@powermove/macos-haptics', () => ({ fontFamilies: vi.fn(() => ['New font']) }));
import { fontFamilies } from '@powermove/macos-haptics';
import { registerFontsIpc } from './fonts';
import { IPC } from '../shared/ipc';

it('reads a fresh native font list only for trusted editor requests', () => {
  const handle = vi.fn();
  const sender = {} as any;
  registerFontsIpc({ handle }, { isTrustedSenderContents: value => value === sender });
  expect(handle.mock.calls[0]![0]).toBe(IPC.fontFamilies);
  const read = handle.mock.calls[0]![1];
  expect(read({ sender: {} })).toBeNull();
  expect(fontFamilies).not.toHaveBeenCalled();
  expect(read({ sender })).toEqual(['New font']);
  expect(read({ sender })).toEqual(['New font']);
  expect(fontFamilies).toHaveBeenCalledTimes(2);
});
