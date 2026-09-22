import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { open, stat, type FileHandle } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { BrowserWindow, dialog, type IpcMain, type IpcMainInvokeEvent } from 'electron';
import { cloudFileState } from '@powermove/macos-haptics';
import { IPC, type CloudFileState } from '../shared/ipc';

const DOWNLOAD_TIMEOUT = 5 * 60_000;
const CHUNK_BYTES = 4 * 1024 * 1024;
export function cloudPath(value: unknown): string {
  if (typeof value !== 'string' || !path.isAbsolute(value) || value.includes('\0') || value.length > 16_384) {
    throw new Error('Expected an absolute media path');
  }
  return path.normalize(value);
}

/** A child process keeps provider I/O and its timeout off Electron's event loop.
 * Reading the complete data fork asks File Provider to materialize every byte;
 * stdout is discarded, so even large movies do not accumulate in main's heap. */
export function materializeFile(source: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn('/bin/cat', [source], { stdio: ['ignore', 'ignore', 'pipe'] });
    let detail = '';
    let timedOut = false;
    child.stderr.on('data', chunk => { detail = (detail + String(chunk)).slice(0, 1000); });
    const timer = setTimeout(() => { timedOut = true; child.kill('SIGKILL'); }, DOWNLOAD_TIMEOUT);
    child.once('error', error => { clearTimeout(timer); reject(error); });
    child.once('close', code => {
      clearTimeout(timer);
      if (code === 0) resolve();
      else reject(new Error(timedOut ? 'Download timed out. Check your connection and try again.' : detail.trim() || 'Cloud download failed. Try again or download the file in Finder.'));
    });
  });
}

export class CloudMediaService {
  private downloads = new Map<string, Promise<void>>();
  constructor(
    private inspect: typeof cloudFileState = cloudFileState,
    private materialize: (source: string) => Promise<void> = materializeFile,
  ) {}
  async status(source: string): Promise<CloudFileState> { return this.inspect(source); }
  async download(source: string): Promise<void> {
    const pending = this.downloads.get(source);
    if (pending) return pending;
    if (this.downloads.size >= 4) throw new Error('Other files are downloading. Try again when they finish.');
    const operation = this.hydrate(source).finally(() => { this.downloads.delete(source); });
    this.downloads.set(source, operation);
    return operation;
  }
  private async hydrate(source: string): Promise<void> {
    const state = await this.inspect(source, true);
    if (state === 'missing' || state === 'unknown') throw new Error('The cloud file is unavailable. Locate it or download it in Finder.');
    // A legacy iCloud placeholder may not expose the original path until its
    // asynchronous download completes. Modern providers already expose it.
    const deadline = Date.now() + DOWNLOAD_TIMEOUT;
    while (state === 'icloud') {
      try { await stat(source); break; }
      catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
        if (Date.now() >= deadline) throw new Error('iCloud download timed out. Check your connection and try again.');
        await delay(500);
      }
    }
    const info = await stat(source);
    if (!info.isFile()) throw new Error('The media source is not a file');
    await this.materialize(source);
    if (await this.inspect(source) !== 'local') throw new Error('The file is still in the cloud. Try downloading it in Finder.');
  }
}

export function cloudPromptOptions(names: string[]): Electron.MessageBoxOptions {
  return {
    type: 'question',
    message: names.length === 1 ? 'Download cloud media?' : `Download ${names.length} cloud media files?`,
    detail: `These files are stored in the cloud and need to be downloaded to use them in this project.\n\n${names.slice(0, 8).join('\n')}${names.length > 8 ? `\n…and ${names.length - 8} more` : ''}`,
    buttons: ['Download files', 'Not now'], defaultId: 0, cancelId: 1, noLink: true,
    checkboxLabel: 'Automatically download cloud media when opening projects', checkboxChecked: false,
  };
}

export function registerCloudMediaIpc(ipc: Pick<IpcMain, 'handle'>, ctx: {
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
}, service = new CloudMediaService()): void {
  const files = new Map<string, { owner: number; handle: FileHandle; size: number; timer: ReturnType<typeof setTimeout>; detach: () => void }>();
  const release = async (token: string) => {
    const file = files.get(token);
    if (!file) return;
    files.delete(token);
    clearTimeout(file.timer);
    file.detach();
    await file.handle.close();
  };
  const trusted = (event: IpcMainInvokeEvent) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
  };
  ipc.handle(IPC.cloudStatus, async (event, input: unknown) => {
    trusted(event);
    if (!Array.isArray(input) || input.length > 256) throw new Error('Expected at most 256 media paths');
    const sources = [...new Set(input as string[])];
    sources.forEach(cloudPath);
    const result: Record<string, CloudFileState> = {};
    // Avoid flooding the native worker pool for large media libraries.
    for (const source of sources) result[source] = await service.status(cloudPath(source));
    return result;
  });
  ipc.handle(IPC.cloudPrompt, async (event, names: unknown) => {
    trusted(event);
    if (!Array.isArray(names) || !names.length || names.length > 10_000 || names.some(name => typeof name !== 'string' || name.length > 1024)) throw new Error('Invalid cloud media names');
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) return { download: false, automatic: false };
    const result = await dialog.showMessageBox(window, cloudPromptOptions(names));
    return { download: result.response === 0, automatic: result.checkboxChecked };
  });
  const openSource = async (event: IpcMainInvokeEvent, source: string) => {
    if (event.sender.isDestroyed()) throw new Error('The project window closed');
    const handle = await open(source, 'r');
    try {
      const info = await handle.stat();
      if (!info.isFile()) throw new Error('The media source is not a file');
      const size = info.size;
      if (event.sender.isDestroyed()) throw new Error('The project window closed');
      const token = randomUUID();
      const cleanup = () => { void release(token).catch(() => undefined); };
      const timer = setTimeout(cleanup, DOWNLOAD_TIMEOUT);
      timer.unref();
      files.set(token, { owner: event.sender.id, handle, size, timer, detach: () => event.sender.removeListener('destroyed', cleanup) });
      event.sender.once('destroyed', cleanup);
      return { token, size };
    } catch (error) { await handle.close(); throw error; }
  };
  ipc.handle(IPC.mediaOpenLocalSource, async (event, input: unknown) => {
    trusted(event);
    const source = cloudPath(input);
    // Inspect metadata before opening the data fork: cache recovery must not
    // silently download an offloaded file or bypass the cloud preference.
    if (await service.status(source) !== 'local') return null;
    return openSource(event, source);
  });
  ipc.handle(IPC.cloudDownload, async (event, input: unknown) => {
    trusted(event);
    const source = cloudPath(input);
    await service.download(source);
    return openSource(event, source);
  });
  ipc.handle(IPC.cloudRead, async (event, input: { token: string; offset: number; length: number }) => {
    trusted(event);
    const file = files.get(input?.token);
    if (!file || file.owner !== event.sender.id) throw new Error('Unknown media download');
    const { offset, length } = input;
    if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(length) || length < 1 || length > CHUNK_BYTES || offset + length > file.size) throw new Error('Invalid media read');
    file.timer.refresh();
    const buffer = Buffer.allocUnsafe(length);
    const { bytesRead } = await file.handle.read(buffer, 0, length, offset);
    return new Uint8Array(buffer.subarray(0, bytesRead));
  });
  ipc.handle(IPC.cloudRelease, async (event, token: string) => {
    trusted(event);
    if (files.get(token)?.owner === event.sender.id) await release(token);
  });
}
