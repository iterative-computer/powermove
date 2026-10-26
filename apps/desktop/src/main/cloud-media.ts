import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { open, stat, type FileHandle } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { BrowserWindow, dialog, type IpcMain, type IpcMainInvokeEvent } from 'electron';
import { cloudFileState } from '@powermove/macos-haptics';
import { IPC, type CloudFileState } from '../shared/ipc';
import { isRendererUnavailable } from './renderer-recovery';

const DOWNLOAD_TIMEOUT = 5 * 60_000;
const CHUNK_BYTES = 4 * 1024 * 1024;
export function cloudPath(value: unknown): string {
  if (typeof value !== 'string' || !path.isAbsolute(value) || value.includes('\0') || value.length > 16_384) {
    throw new Error('Expected an absolute media path');
  }
  return path.normalize(value);
}

export class CloudFileNotReadyError extends Error {
  override name = 'CloudFileNotReadyError';
}

/** A child process keeps provider I/O and its timeout off Electron's event loop.
 * Reading the complete data fork asks File Provider to materialize every byte;
 * stdout is counted and discarded, so a placeholder returning early EOF cannot
 * masquerade as a completed download, and large movies do not fill main's heap. */
export async function materializeFile(source: string, signal?: AbortSignal): Promise<void> {
  const expected = (await stat(source)).size;
  signal?.throwIfAborted();
  return new Promise((resolve, reject) => {
    const child = spawn('/bin/cat', [source], { stdio: ['ignore', 'pipe', 'pipe'] });
    let bytesRead = 0;
    child.stdout.on('data', chunk => { bytesRead += chunk.length; });
    let detail = '';
    let timedOut = false;
    child.stderr.on('data', chunk => { detail = (detail + String(chunk)).slice(0, 1000); });
    const timer = setTimeout(() => {
      timedOut = true; child.kill('SIGKILL');
      reject(new Error('Download timed out. Check your connection and try again.'));
    }, DOWNLOAD_TIMEOUT);
    const abort = () => { cleanup(); child.kill('SIGKILL'); reject(signal?.reason ?? new Error('Cloud download cancelled')); };
    const cleanup = () => { clearTimeout(timer); signal?.removeEventListener('abort', abort); };
    signal?.addEventListener('abort', abort, { once: true });
    child.once('error', error => { cleanup(); reject(error); });
    child.once('close', code => {
      cleanup();
      if (code === 0 && bytesRead === expected) resolve();
      else if (code === 0) reject(new CloudFileNotReadyError(`Cloud media is still downloading (${bytesRead} of ${expected} bytes available).`));
      else reject(new Error(timedOut ? 'Download timed out. Check your connection and try again.' : detail.trim() || 'Cloud download failed. Try again or download the file in Finder.'));
    });
  });
}

export class CloudMediaService {
  private downloads = new Map<string, Promise<void>>();
  constructor(
    private inspect: typeof cloudFileState = cloudFileState,
    private materialize: (source: string, signal?: AbortSignal) => Promise<void> = materializeFile,
  ) {}
  async status(source: string): Promise<CloudFileState> { return this.inspect(source); }
  async download(source: string): Promise<void> {
    const pending = this.downloads.get(source);
    if (pending) return pending;
    if (this.downloads.size >= 4) throw new Error('Other files are downloading. Try again when they finish.');
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const operation = Promise.race([this.hydrate(source, controller.signal), new Promise<never>((_, reject) => {
      timer = setTimeout(() => {
        controller.abort();
        reject(new Error('Cloud download timed out. Check your connection and try again.'));
      }, DOWNLOAD_TIMEOUT);
    })]).finally(() => { clearTimeout(timer!); this.downloads.delete(source); });
    this.downloads.set(source, operation);
    return operation;
  }
  private async hydrate(source: string, signal: AbortSignal): Promise<void> {
    const state = await this.inspect(source, true);
    signal.throwIfAborted();
    if (state === 'missing' || state === 'unknown') throw new Error('The cloud file is unavailable. Locate it or download it in Finder.');
    // A legacy iCloud placeholder may not expose the original path until its
    // asynchronous download completes. Modern providers already expose it.
    const deadline = Date.now() + DOWNLOAD_TIMEOUT;
    while (state === 'icloud') {
      signal.throwIfAborted();
      try { await stat(source); break; }
      catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
        if (Date.now() >= deadline) throw new Error('iCloud download timed out. Check your connection and try again.');
        await delay(500, undefined, { signal });
      }
    }
    const info = await stat(source);
    if (!info.isFile()) throw new Error('The media source is not a file');
    signal.throwIfAborted();
    for (;;) {
      try { await this.materialize(source, signal); break; }
      catch (error) {
        signal.throwIfAborted();
        if (!(error instanceof CloudFileNotReadyError)) throw error;
        await delay(500, undefined, { signal });
      }
    }
    signal.throwIfAborted();
    // A successful full read proves the bytes are available. File Provider and
    // iCloud status flags can lag behind that read; treating those flags as a
    // second download verdict rejects files we have just downloaded correctly.
    // The subsequent owned file handle/read still reports real I/O failures.
    const downloaded = await stat(source);
    if (!downloaded.isFile() || downloaded.size !== info.size) {
      throw new Error('The cloud file changed while downloading. Try restoring it again.');
    }
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
  const openSource = async (event: IpcMainInvokeEvent, source: string, prepare: () => Promise<boolean>) => {
    // Recovery reuses WebContents, so availability alone cannot distinguish
    // an old request from one initiated by the replacement renderer.
    let rendererGone = false;
    const gone = () => { rendererGone = true; };
    const checkRenderer = () => {
      if (rendererGone || isRendererUnavailable(event.sender)) throw new Error('The media renderer has closed or crashed');
    };
    event.sender.once('render-process-gone', gone);
    try {
      checkRenderer();
      const available = await prepare();
      checkRenderer();
      if (!available) return null;
      const handle = await open(source, 'r');
      try {
        checkRenderer();
        const info = await handle.stat();
        checkRenderer();
        if (!info.isFile()) throw new Error('The media source is not a file');
        const size = info.size;
        const token = randomUUID();
        const cleanup = () => { void release(token).catch(() => undefined); };
        const timer = setTimeout(cleanup, DOWNLOAD_TIMEOUT);
        timer.unref();
        files.set(token, { owner: event.sender.id, handle, size, timer, detach: () => {
          event.sender.removeListener('destroyed', cleanup);
          event.sender.removeListener('render-process-gone', cleanup);
        } });
        event.sender.once('destroyed', cleanup);
        event.sender.once('render-process-gone', cleanup);
        return { token, size };
      } catch (error) { await handle.close().catch(() => undefined); throw error; }
    } finally { event.sender.removeListener('render-process-gone', gone); }
  };
  ipc.handle(IPC.mediaOpenLocalSource, async (event, input: unknown) => {
    trusted(event);
    const source = cloudPath(input);
    // Inspect metadata before opening the data fork: cache recovery must not
    // silently download an offloaded file or bypass the cloud preference.
    return openSource(event, source, async () => await service.status(source) === 'local');
  });
  ipc.handle(IPC.cloudDownload, async (event, input: unknown) => {
    trusted(event);
    const source = cloudPath(input);
    return openSource(event, source, async () => { await service.download(source); return true; });
  });
  ipc.handle(IPC.cloudRead, async (event, input: { token: string; offset: number; length: number }) => {
    trusted(event);
    const file = files.get(input?.token);
    if (!file || file.owner !== event.sender.id) throw new Error('Unknown media download');
    const { offset, length } = input;
    if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(length) || length < 1 || length > CHUNK_BYTES || offset + length > file.size) throw new Error('Invalid media read');
    file.timer.refresh();
    const buffer = new Uint8Array(length);
    const { bytesRead } = await file.handle.read(buffer, 0, length, offset);
    return bytesRead === length ? buffer : buffer.slice(0, bytesRead);
  });
  ipc.handle(IPC.cloudRelease, async (event, token: string) => {
    trusted(event);
    if (files.get(token)?.owner === event.sender.id) await release(token);
  });
}
