import { EventEmitter } from 'node:events';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ inspect: vi.fn(), show: vi.fn(), window: vi.fn() }));
vi.mock('@powermove/macos-haptics', () => ({ cloudFileState: mocks.inspect }));
vi.mock('electron', () => ({ BrowserWindow: { fromWebContents: mocks.window }, dialog: { showMessageBox: mocks.show } }));
import { CloudMediaService, cloudPath, cloudPromptOptions, materializeFile, registerCloudMediaIpc } from './cloud-media';
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
  expect(read).toHaveBeenCalledExactlyOnceWith(source);
  expect(inspect).toHaveBeenNthCalledWith(1, source, true);
});
it('does not try reading deleted files or mistake a failed hydration for success', async () => {
  const read = vi.fn();
  const service = new CloudMediaService(vi.fn().mockResolvedValue('missing'), read);
  await expect(service.download('/missing.mov')).rejects.toThrow('unavailable');
  expect(read).not.toHaveBeenCalled();
  const source = await file();
  await expect(new CloudMediaService(vi.fn().mockResolvedValue('cloud'), read).download(source)).rejects.toThrow('still in the cloud');
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
