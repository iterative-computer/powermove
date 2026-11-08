import type { CloudFileState } from '../shared/ipc';
import { lstat } from 'node:fs/promises';
import { inspectFileCloudState } from './file-cloud-state';
import { contentIdentity } from './file-content-version';

interface CloudAccess {
  status(file: string): Promise<CloudFileState>;
  download(file: string): Promise<void>;
  version?(file: string): Promise<string>;
}
type Download = { provider: string; done: boolean; failed?: boolean; error?: unknown; version?: string; finished: Promise<void> };

/** Opening a placeholder must not put a blocking data-fork read into the
 * project transaction queue. Hydration already runs in a bounded child process.
 * The existing open-error surface can show this state until a progress UI exists. */
export class ProjectCloudAccess {
  private downloads = new Map<string, Download>();
  // Most projects download within a minute: wait and open them without a
  // second click. Larger ones keep downloading; the person reopens later.
  constructor(private readonly cloud: CloudAccess, private readonly metadataTimeoutMs = 10_000, private readonly waitMs = 60_000) {}

  async prepare(file: string): Promise<void> {
    const pending = this.downloads.get(file);
    if (pending) {
      if (!pending.done) throw this.downloading(pending.provider);
      this.downloads.delete(file);
      if (pending.failed) throw new Error('Could not download this project. Check your connection, download it in Finder, then try opening it again.', { cause: pending.error });
      // Provider flags can lag a successful read. Only reuse that proof while
      // the exact downloaded file is still present: a later eviction or sync
      // replacement must go through the cloud check again.
      if (!this.cloud.version || pending.version === await this.bounded(this.cloud.version(file))) return;
    }
    const state = await this.bounded(this.cloud.status(file));
    if (state !== 'icloud' && state !== 'cloud') return;
    // Another open may have started hydration while this metadata probe waited.
    if (this.downloads.has(file)) return this.prepare(file);
    // Unclaimed completed results must not fill the active-download limit.
    for (const [path, download] of this.downloads) if (download.done) this.downloads.delete(path);
    if (this.downloads.size >= 4) throw new Error('Other projects are downloading. Try opening this project when they finish.');
    const download = { provider: state === 'icloud' ? 'iCloud' : 'cloud storage', done: false } as Download;
    this.downloads.set(file, download);
    download.finished = Promise.resolve().then(async () => {
      await this.cloud.download(file);
      download.version = this.cloud.version ? await this.bounded(this.cloud.version(file)) : undefined;
    }).then(
      () => { download.done = true; },
      error => { download.failed = true; download.error = error; download.done = true; },
    );
    let timer: ReturnType<typeof setTimeout> | undefined;
    await Promise.race([download.finished, new Promise<void>(resolve => { timer = setTimeout(resolve, this.waitMs); })]);
    clearTimeout(timer);
    if (download.done) return this.prepare(file);
    throw this.downloading(download.provider);
  }
  private downloading(provider: string): Error {
    return new Error(`Downloading this project from ${provider}. Try opening it again when the download finishes. If you are offline, reconnect or download the file in Finder.`);
  }
  private async bounded<T>(operation: Promise<T>): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      return await Promise.race([operation, new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error('This file is taking too long to respond. Check your cloud storage or network connection, then try opening it again.')), this.metadataTimeoutMs);
      })]);
    } finally { clearTimeout(timer); }
  }
}

let media: Promise<import('./cloud-media').CloudMediaService> | undefined;
const access = new ProjectCloudAccess({
  status: inspectFileCloudState,
  version: async file => contentIdentity(await lstat(file)),
  async download(file) {
    const service = await (media ??= import('./cloud-media').then(module => new module.CloudMediaService()));
    await service.download(file);
  }
});
export const prepareProjectFile = (file: string): Promise<void> => access.prepare(file);

export function projectReadError(error: unknown): string {
  const failure = error as NodeJS.ErrnoException | null;
  if (failure?.errno === -11 || ['EDEADLK', 'Unknown system error -11', 'EAGAIN', 'ETIMEDOUT', 'ENOTCONN'].includes(failure?.code ?? '')) {
    return 'This project could not be read from cloud storage or a network drive. Check your connection, download the file in Finder, then try opening it again.';
  }
  if (failure?.code === 'ENOENT') return 'This project is unavailable or has moved. Wait for syncing to finish, then try opening it again from Finder.';
  return failure?.message || 'Could not open project.';
}
