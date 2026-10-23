import { afterEach, expect, it, vi } from 'vitest';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { cloudSourcePaths } from './cloud-media';
import { readDroppedFolder } from './folder-import';

afterEach(() => { resetBridgeForTests(); vi.unstubAllGlobals(); });

function directory(name: string, children: any[]): FileSystemDirectoryEntry {
  return { name, isDirectory: true, createReader: () => {
    let read = false;
    return { readEntries: (done: (entries: any[]) => void) => { done(read ? [] : children); read = true; } };
  } } as any;
}

it('waits for cloud preparation before acquiring a nested file snapshot', async () => {
  installBridgeForTests({ media: { sourcePath: () => null } } as any);
  const file = new File(['sound'], 'Sound.wav');
  let downloaded = false;
  let finish!: () => void;
  const download = new Promise<void>(resolve => { finish = resolve; });
  const snapshot = vi.fn((done: (file: File) => void) => {
    if (!downloaded) throw new DOMException('The requested file could not be read', 'NotReadableError');
    done(file);
  });
  const prepare = vi.fn(async () => { await download; downloaded = true; });
  const root = directory('SFX', [directory('Nested', [{
    name: 'Sound.wav', fullPath: '/SFX/Nested/Sound.wav', isDirectory: false, file: snapshot,
  }])]);
  const reading = readDroppedFolder(root, '/cloud/SFX', prepare);
  await vi.waitFor(() => expect(prepare).toHaveBeenCalledWith('/cloud/SFX/Nested/Sound.wav', 'Sound.wav'));
  expect(snapshot).not.toHaveBeenCalled();
  finish();
  const tree = await reading;
  expect(tree.folders[0]!.files).toEqual([file]);
  expect(cloudSourcePaths.get(file)).toBe('/cloud/SFX/Nested/Sound.wav');
});

it('does not acquire a stale file when its cloud download fails', async () => {
  const snapshot = vi.fn();
  const root = directory('SFX', [{ name: 'Sound.wav', fullPath: '/SFX/Sound.wav', isDirectory: false, file: snapshot }]);
  await expect(readDroppedFolder(root, '/cloud/SFX', async () => { throw new Error('Download failed'); }))
    .rejects.toThrow('Download failed');
  expect(snapshot).not.toHaveBeenCalled();
});

it('keeps browser-only folder drops working and skips hidden entries', async () => {
  const file = new File(['sound'], 'Sound.wav');
  const prepare = vi.fn();
  const root = directory('SFX', [
    { name: '.DS_Store', file: () => { throw new Error('Hidden file was read'); } },
    { name: 'Sound.wav', fullPath: '/SFX/Sound.wav', isDirectory: false, file: (done: (file: File) => void) => done(file) },
  ]);
  const tree = await readDroppedFolder(root, '', prepare);
  expect(tree.files).toEqual([file]);
  expect(prepare).not.toHaveBeenCalled();
});

it('preserves readable files when a sibling cannot be downloaded', async () => {
  installBridgeForTests({ media: { sourcePath: () => null } } as any);
  const file = new File(['sound'], 'Local.wav');
  const unavailable = vi.fn();
  const root = directory('SFX', [
    { name: 'Cloud.wav', fullPath: '/SFX/Cloud.wav', isDirectory: false, file: unavailable },
    { name: 'Local.wav', fullPath: '/SFX/Local.wav', isDirectory: false, file: (done: (file: File) => void) => done(file) },
  ]);
  const error = new Error('Cloud unavailable');
  const report = vi.fn();
  const tree = await readDroppedFolder(root, '/cloud/SFX', async (_path, name) => {
    if (name === 'Cloud.wav') throw error;
  }, report);
  expect(tree.files).toEqual([file]);
  expect(unavailable).not.toHaveBeenCalled();
  expect(report).toHaveBeenCalledExactlyOnceWith('Cloud.wav', error);
});
