import { EventEmitter } from 'node:events';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import type { FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ inspect: vi.fn(), show: vi.fn(), window: vi.fn(), open: vi.fn(), spawn: vi.fn() }));
vi.mock('node:child_process', async importOriginal => {
  const child = await importOriginal<typeof import('node:child_process')>();
  mocks.spawn.mockImplementation(child.spawn);
  return { ...child, spawn: mocks.spawn };
});
vi.mock('node:fs/promises', async importOriginal => {
  const fs = await importOriginal<typeof import('node:fs/promises')>();
  mocks.open.mockImplementation(fs.open);
  return { ...fs, open: mocks.open };
});
vi.mock('@powermove/macos-haptics', () => ({ cloudFileState: mocks.inspect }));
vi.mock('electron', () => ({ BrowserWindow: { fromWebContents: mocks.window }, dialog: { showMessageBox: mocks.show } }));
import { CloudMediaService, CloudFileNotReadyError, cloudPath, cloudPromptOptions, materializeFile, registerCloudMediaIpc } from './cloud-media';
import { IPC } from '../shared/ipc';
const directories: string[] = [];
afterEach(async () => { vi.clearAllMocks(); await Promise.all(directories.splice(0).map(dir => rm(dir, { recursive: true, force: true }))); });
async function file() {
  const dir = await mkdtemp(path.join(tmpdir(), 'powermove-cloud-test-')); directories.push(dir);
  const source = path.join(dir, 'clip with spaces.mp4');
  await writeFile(source, 'video bytes'); return source;
}
function ipc(service: CloudMediaService, trusted = true) {
  const handlers = new Map<string, any>();
  registerCloudMediaIpc({ handle: (channel: string, fn: any) => handlers.set(channel, fn) } as any, { isTrustedSender: () => trusted }, service);
  const sender = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false });
  return { call: (channel: string, value: any, owner = sender) => handlers.get(channel)({ sender: owner }, value), sender };
}
it('validates media paths and native prompt defaults', () => {
  expect(() => cloudPath('relative.mov')).toThrow();
  expect(() => cloudPath('/tmp/a\0.mov')).toThrow();
  expect(cloudPromptOptions(['clip.mp4'])).toMatchObject({ buttons: ['Download files', 'Not now'], cancelId: 1, checkboxChecked: false });
});
it('deduplicates downloads and materializes before reporting success', async () => {
  const source = await file();
  const inspect = vi.fn().mockResolvedValueOnce('cloud').mockResolvedValue('local');
  const read = vi.fn(materializeFile);
  const service = new CloudMediaService(inspect, read);
  await Promise.all([service.download(source), service.download(source)]);
  expect(read).toHaveBeenCalledExactlyOnceWith(source, expect.any(AbortSignal));
  expect(inspect).toHaveBeenNthCalledWith(1, source, true);
});
it('does not try reading deleted files or mistake a failed hydration for success', async () => {
  const read = vi.fn();
  const service = new CloudMediaService(vi.fn().mockResolvedValue('missing'), read);
  await expect(service.download('/missing.mov')).rejects.toThrow('unavailable');
  expect(read).not.toHaveBeenCalled();
  const source = await file();
  read.mockRejectedValueOnce(new Error('Cloud provider could not read the file'));
  await expect(new CloudMediaService(vi.fn().mockResolvedValue('cloud'), read).download(source)).rejects.toThrow('could not read');
});
it('restores readable cloud media even when provider metadata still says offloaded after download', async () => {
  const source = await file();
  const inspect = vi.fn().mockResolvedValue('icloud');
  const read = vi.fn(materializeFile);
  const { call } = ipc(new CloudMediaService(inspect, read));
  const result = await call(IPC.cloudDownload, source);
  try {
    expect(new TextDecoder().decode(await call(IPC.cloudRead, { token: result.token, offset: 0, length: result.size }))).toBe('video bytes');
    expect(read).toHaveBeenCalledExactlyOnceWith(source, expect.any(AbortSignal));
  } finally { if (result) await call(IPC.cloudRelease, result.token); }
});
it('checks trust before touching filesystem paths', async () => {
  const service = new CloudMediaService();
  const status = vi.spyOn(service, 'status');
  const { call } = ipc(service, false);
  await expect(call(IPC.cloudStatus, ['/tmp/a.mov'])).rejects.toThrow('Unauthorized');
  expect(status).not.toHaveBeenCalled();
});
it('streams only bounded chunks to the window that owns the token', async () => {
  const source = await file();
  const service = new CloudMediaService(vi.fn().mockResolvedValue('local'), vi.fn());
  const { call, sender } = ipc(service);
  const result = await call(IPC.cloudDownload, source);
  expect(result.size).toBe(11);
  expect(new TextDecoder().decode(await call(IPC.cloudRead, { token: result.token, offset: 0, length: 5 }))).toBe('video');
  await expect(call(IPC.cloudRead, { token: result.token, offset: 0, length: 12 })).rejects.toThrow('Invalid media read');
  await expect(call(IPC.cloudRead, { token: result.token, offset: 0, length: 5 }, { ...sender, id: 2 })).rejects.toThrow('Unknown');
  await call(IPC.cloudRelease, result.token);
  expect(sender.listenerCount('destroyed')).toBe(0);
  await expect(call(IPC.cloudRead, { token: result.token, offset: 0, length: 1 })).rejects.toThrow('Unknown');
});
it('returns the owned read allocation directly, with independent bytes and exact short reads', async () => {
  const source = await file();
  const { call } = ipc(new CloudMediaService(vi.fn().mockResolvedValue('local'), vi.fn()));
  const result = await call(IPC.mediaOpenLocalSource, source);
  const handle = await mocks.open.mock.results.at(-1)!.value;
  const read = vi.spyOn(handle, 'read');
  const first: Uint8Array = await call(IPC.cloudRead, { token: result.token, offset: 6, length: 5 });
  expect(new TextDecoder().decode(first)).toBe('bytes');
  expect(first).toBe(read.mock.calls[0]![0]);
  expect(Buffer.isBuffer(first)).toBe(false);
  expect(first.byteOffset).toBe(0);
  expect(first.buffer.byteLength).toBe(first.byteLength);
  first[0] = 0;
  expect(new TextDecoder().decode(await call(IPC.cloudRead, { token: result.token, offset: 6, length: 5 }))).toBe('bytes');

  // The original size is validated when opening; a source can shrink afterward.
  await writeFile(source, 'video by');
  const short: Uint8Array = await call(IPC.cloudRead, { token: result.token, offset: 6, length: 5 });
  expect([...short]).toEqual([...Buffer.from('by')]);
  expect(short.buffer.byteLength).toBe(2);
  const empty: Uint8Array = await call(IPC.cloudRead, { token: result.token, offset: 10, length: 1 });
  expect(empty.byteLength).toBe(0);
  expect(empty.buffer.byteLength).toBe(0);
  await call(IPC.cloudRelease, result.token);
});
it('returns both prompt choices, including the automatic download checkbox', async () => {
  const { call } = ipc(new CloudMediaService());
  mocks.window.mockReturnValue({ isDestroyed: () => false });
  mocks.show.mockResolvedValue({ response: 1, checkboxChecked: true });
  await expect(call(IPC.cloudPrompt, ['clip.mp4'])).resolves.toEqual({ download: false, automatic: true });
});

it('recovers local bytes without requesting a cloud download', async () => {
  const source = await file();
  const inspect = vi.fn().mockResolvedValue('local');
  const materialize = vi.fn();
  const { call } = ipc(new CloudMediaService(inspect, materialize));
  const result = await call(IPC.mediaOpenLocalSource, source);
  expect(new TextDecoder().decode(await call(IPC.cloudRead, { token: result.token, offset: 0, length: result.size }))).toBe('video bytes');
  await call(IPC.cloudRelease, result.token);
  expect(inspect).toHaveBeenCalledExactlyOnceWith(source);
  expect(materialize).not.toHaveBeenCalled();
});

it.each(['icloud', 'cloud', 'missing', 'unknown'])('does not open a %s source during automatic local recovery', async state => {
  const materialize = vi.fn();
  const { call } = ipc(new CloudMediaService(vi.fn().mockResolvedValue(state), materialize));
  await expect(call(IPC.mediaOpenLocalSource, '/nonexistent/source.mov')).resolves.toBeNull();
  expect(materialize).not.toHaveBeenCalled();
});

it('validates trust and paths before local recovery', async () => {
  const service = new CloudMediaService();
  const inspect = vi.spyOn(service, 'status');
  await expect(ipc(service, false).call(IPC.mediaOpenLocalSource, '/tmp/a.mov')).rejects.toThrow('Unauthorized');
  await expect(ipc(service).call(IPC.mediaOpenLocalSource, 'relative.mov')).rejects.toThrow('absolute');
  expect(inspect).not.toHaveBeenCalled();
});

it('closes owned local and downloaded source handles exactly once after a renderer crash, then allows fresh reads', async () => {
  const source = await file();
  const service = new CloudMediaService(vi.fn().mockResolvedValue('local'), vi.fn());
  const { call, sender } = ipc(service);
  const local = await call(IPC.mediaOpenLocalSource, source);
  const localHandle: FileHandle = await mocks.open.mock.results.at(-1)!.value;
  const downloaded = await call(IPC.cloudDownload, source);
  const downloadHandle: FileHandle = await mocks.open.mock.results.at(-1)!.value;
  const localClose = vi.spyOn(localHandle, 'close');
  const downloadClose = vi.spyOn(downloadHandle, 'close');
  try {
    sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    expect(sender.isDestroyed()).toBe(false);
    await vi.waitFor(() => {
      expect(localHandle.fd).toBe(-1);
      expect(downloadHandle.fd).toBe(-1);
    });
    expect(localClose).toHaveBeenCalledTimes(1);
    expect(downloadClose).toHaveBeenCalledTimes(1);
    expect(sender.listenerCount('destroyed')).toBe(0);
    expect(sender.listenerCount('render-process-gone')).toBe(0);
    await expect(call(IPC.cloudRead, { token: local.token, offset: 0, length: 1 })).rejects.toThrow('Unknown');
    await expect(call(IPC.cloudRead, { token: downloaded.token, offset: 0, length: 1 })).rejects.toThrow('Unknown');
    sender.emit('did-finish-load');
    const fresh = await call(IPC.mediaOpenLocalSource, source);
    expect(new TextDecoder().decode(await call(IPC.cloudRead, { token: fresh.token, offset: 0, length: fresh.size }))).toBe('video bytes');
    await call(IPC.cloudRelease, fresh.token);
    sender.emit('destroyed');
    expect(localClose).toHaveBeenCalledTimes(1);
    expect(downloadClose).toHaveBeenCalledTimes(1);
  } finally {
    await call(IPC.cloudRelease, local.token);
    await call(IPC.cloudRelease, downloaded.token);
    sender.emit('destroyed');
  }
});

it.each(['status', 'download', 'stat'] as const)('does not publish an old source token after recovery during %s', async stage => {
  const source = await file();
  const service = new CloudMediaService(vi.fn().mockResolvedValue('local'), vi.fn());
  const { call, sender } = ipc(service);
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let waiting = false;
  let handle: FileHandle | undefined;
  let close: ReturnType<typeof vi.spyOn> | undefined;
  if (stage === 'status') vi.spyOn(service, 'status').mockImplementationOnce(async () => { waiting = true; await pause; return 'local'; });
  if (stage === 'download') vi.spyOn(service, 'download').mockImplementationOnce(async () => { waiting = true; await pause; });
  if (stage === 'stat') {
    const actualFs = await vi.importActual<typeof import('node:fs/promises')>('node:fs/promises');
    mocks.open.mockImplementationOnce(async (filePath: string, flags: string) => {
      handle = await actualFs.open(filePath, flags);
      close = vi.spyOn(handle, 'close');
      const originalStat = handle.stat.bind(handle);
      vi.spyOn(handle, 'stat').mockImplementationOnce(async () => { waiting = true; await pause; return originalStat(); });
      return handle;
    });
  }
  let result: { token?: string; error?: Error } | undefined;
  const pending = call(stage === 'download' ? IPC.cloudDownload : IPC.mediaOpenLocalSource, source)
    .then((opened: { token: string }) => { result = opened; }, (error: Error) => { result = { error }; });
  try {
    await vi.waitFor(() => expect(waiting).toBe(true));
    sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    sender.emit('did-finish-load');
    resume();
    await pending;
    expect(result).toEqual({ error: expect.objectContaining({ message: 'The media renderer has closed or crashed' }) });
    if (stage === 'stat') {
      expect(handle!.fd).toBe(-1);
      expect(close).toHaveBeenCalledTimes(1);
    } else expect(mocks.open).not.toHaveBeenCalled();
    expect(sender.listenerCount('render-process-gone')).toBe(0);
    expect(sender.listenerCount('destroyed')).toBe(0);
  } finally {
    resume();
    await pending;
    if (result?.token) await call(IPC.cloudRelease, result.token);
    if (handle && handle.fd !== -1) await handle.close();
  }
});


it('bounds a stalled native metadata check and permits a fresh attempt', async () => {
  vi.useFakeTimers();
  try {
    const source = await file();
    const inspect = vi.fn().mockImplementationOnce(() => new Promise(() => {})).mockResolvedValue('local');
    const read = vi.fn().mockResolvedValue(undefined);
    const service = new CloudMediaService(inspect, read);
    const failure = expect(service.download(source)).rejects.toThrow('timed out');
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    await failure;
    await service.download(source);
    expect(read).toHaveBeenCalledExactlyOnceWith(source, expect.any(AbortSignal));
  } finally { vi.useRealTimers(); }
});


it('times out stalled materialization, ignores its late completion, and allows retry', async () => {
  vi.useFakeTimers();
  try {
    const source = await file();
    let finish!: () => void;
    const inspect = vi.fn().mockResolvedValue('local');
    const read = vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; })).mockResolvedValue(undefined);
    const service = new CloudMediaService(inspect, read);
    const failure = expect(service.download(source)).rejects.toThrow('timed out');
    // Let real filesystem metadata settle before advancing the download clock.
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    await failure;
    finish();
    await Promise.resolve();
    await service.download(source);
    expect(read).toHaveBeenCalledTimes(2);
    expect(inspect).toHaveBeenCalledTimes(2);
  } finally { vi.useRealTimers(); }
});

it('waits for all cloud bytes to arrive before opening a restored video', async () => {
  const source = await file();
  const read = vi.fn().mockRejectedValueOnce(new CloudFileNotReadyError('Still downloading')).mockImplementation(materializeFile);
  const { call } = ipc(new CloudMediaService(vi.fn().mockResolvedValue('icloud'), read));
  const result = await call(IPC.cloudDownload, source);
  try {
    expect(read).toHaveBeenCalledTimes(2);
    expect(new TextDecoder().decode(await call(IPC.cloudRead, { token: result.token, offset: 0, length: result.size }))).toBe('video bytes');
  } finally { await call(IPC.cloudRelease, result.token); }
});

it('rejects a source changed during hydration instead of importing partial bytes', async () => {
  const source = await file();
  const service = new CloudMediaService(vi.fn().mockResolvedValue('icloud'), async () => { await writeFile(source, 'changed'); });
  await expect(service.download(source)).rejects.toThrow('changed while downloading');
});

it('does not treat a successful cat exit with a short data fork as downloaded media', async () => {
  const source = await file();
  const child = Object.assign(new EventEmitter(), { stdout: new EventEmitter(), stderr: new EventEmitter(), kill: vi.fn() });
  mocks.spawn.mockReturnValueOnce(child);
  const reading = materializeFile(source);
  const rejected = expect(reading).rejects.toBeInstanceOf(CloudFileNotReadyError);
  await vi.waitFor(() => expect(child.listenerCount('close')).toBe(1));
  child.stdout.emit('data', Buffer.from('short'));
  child.emit('close', 0);
  await rejected;
});
