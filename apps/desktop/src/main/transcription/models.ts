import { createHash } from 'node:crypto';
import { createReadStream, createWriteStream, type WriteStream } from 'node:fs';
import { mkdir, readFile, readdir, rename, rm, stat, statfs, truncate, writeFile } from 'node:fs/promises';
import path from 'node:path';

import type { TranscriptionModelInfo, TranscriptionStatus } from '../../shared/transcription';
import { modelSize, type CatalogFile, type CatalogModel } from './catalog';

/*
 * Downloaded speech models on disk, and the downloads that put them there.
 *
 *   <root>/settings.json             active model + language
 *   <root>/<id>/…                    a complete model; model.json is written last
 *   <root>/.partial/<id>/<file>.part a file still arriving (resumed with Range)
 *   <root>/.partial/<id>/<file>      a file that arrived and matched its SHA-256
 *
 * A model becomes visible only by renaming its finished partial folder into
 * place, so a crash or a pulled cable leaves either nothing or a resumable
 * partial folder, never a half model that looks ready.
 */

export type FetchLike = (url: string, init: { headers: Record<string, string>; signal: AbortSignal }) => Promise<Response>;

export interface ModelStoreOptions {
  root: string;
  catalog: readonly CatalogModel[];
  fetch: FetchLike;
  /** Called (throttled) whenever anything in status() changes. */
  onChange?: (status: TranscriptionStatus) => void;
  /** Free bytes on the volume holding `dir`. */
  freeBytes?: (dir: string) => Promise<number>;
  /** No bytes for this long aborts the download. */
  stallMs?: number;
  /** Minimum interval between progress notifications. */
  progressIntervalMs?: number;
}

interface Download {
  controller: AbortController;
  received: number;
  total: number;
  promise: Promise<void>;
}

interface Settings {
  activeModelId: string | null;
  language: string;
}

export const LANGUAGE_TAG = /^(?:auto|[a-z]{2,3}(?:-[A-Za-z0-9]{2,8}){0,2})$/;
/** Headroom kept free on the disk after a model lands. */
const DISK_HEADROOM = 256 * 1024 * 1024;

export class TranscriptionDownloadError extends Error {}

export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  if (bytes >= 1e3) return `${Math.round(bytes / 1e3)} KB`;
  return `${bytes} bytes`;
}

async function sizeOf(file: string): Promise<number | null> {
  try {
    const info = await stat(file);
    return info.isFile() ? info.size : null;
  } catch {
    return null;
  }
}

async function hashFile(file: string, hash: ReturnType<typeof createHash>): Promise<void> {
  for await (const chunk of createReadStream(file)) hash.update(chunk as Buffer);
}

function write(out: WriteStream, chunk: Uint8Array): Promise<void> {
  return new Promise((resolve, reject) => {
    const onError = (error: Error) => reject(error);
    out.once('error', onError);
    const flushed = out.write(chunk, (error) => {
      out.off('error', onError);
      if (error) reject(error);
    });
    if (flushed) {
      out.off('error', onError);
      resolve();
    } else {
      out.once('drain', () => { out.off('error', onError); resolve(); });
    }
  });
}

function close(out: WriteStream): Promise<void> {
  return new Promise((resolve, reject) => {
    out.end((error?: Error | null) => (error ? reject(error) : resolve()));
  });
}

export class ModelStore {
  private settings: Settings = { activeModelId: null, language: 'auto' };
  private readonly ready = new Set<string>();
  private readonly downloads = new Map<string, Download>();
  private readonly errors = new Map<string, string>();
  /** Bytes already on disk for interrupted downloads, by model. */
  private readonly partialBytes = new Map<string, number>();
  private loaded: Promise<void> | null = null;
  private notifyTimer: ReturnType<typeof setTimeout> | null = null;
  private lastNotify = 0;
  private settingsWrite: Promise<void> = Promise.resolve();

  constructor(private readonly options: ModelStoreOptions) {}

  get root(): string { return this.options.root; }
  get catalog(): readonly CatalogModel[] { return this.options.catalog; }

  load(): Promise<void> {
    this.loaded ??= this.scan();
    return this.loaded;
  }

  private model(id: string): CatalogModel {
    const model = this.options.catalog.find((entry) => entry.id === id);
    if (!model) throw new Error('That transcription model is not available.');
    return model;
  }

  private partialDir(id: string): string { return path.join(this.options.root, '.partial', id); }
  modelDir(id: string): string { return path.join(this.options.root, id); }

  private async scan(): Promise<void> {
    await mkdir(this.options.root, { recursive: true });
    try {
      const raw = JSON.parse(await readFile(path.join(this.options.root, 'settings.json'), 'utf8')) as Partial<Settings>;
      if (typeof raw.activeModelId === 'string') this.settings.activeModelId = raw.activeModelId;
      if (typeof raw.language === 'string' && LANGUAGE_TAG.test(raw.language)) this.settings.language = raw.language;
    } catch {
      // No settings yet, or unreadable: defaults.
    }
    for (const model of this.options.catalog) {
      if (await this.complete(model)) this.ready.add(model.id);
      else {
        await this.removeStale(model);
        const partial = await this.partialSize(model);
        if (partial > 0) this.partialBytes.set(model.id, partial);
      }
    }
    if (this.settings.activeModelId && !this.ready.has(this.settings.activeModelId)) {
      this.settings.activeModelId = null;
    }
    if (!this.settings.activeModelId) {
      const first = this.options.catalog.find((model) => this.ready.has(model.id));
      if (first) this.settings.activeModelId = first.id;
    }
  }

  /** A model is complete when its manifest names exactly the catalog's files and each has its size. */
  private async complete(model: CatalogModel): Promise<boolean> {
    const dir = this.modelDir(model.id);
    try {
      const manifest = JSON.parse(await readFile(path.join(dir, 'model.json'), 'utf8')) as { files?: Array<{ name: string; sha256: string }> };
      const listed = new Map((manifest.files ?? []).map((file) => [file.name, file.sha256]));
      for (const file of model.files) {
        if (listed.get(file.name) !== file.sha256) return false;
        if ((await sizeOf(path.join(dir, file.name))) !== file.size) return false;
      }
      return true;
    } catch {
      return false;
    }
  }

  /**
   * A finished folder whose manifest names other files is an older build of
   * the model (the ONNX files of the sherpa-onnx engine, or a re-pinned
   * revision): it can never load again, so give its space back.
   */
  private async removeStale(model: CatalogModel): Promise<void> {
    const dir = this.modelDir(model.id);
    try {
      const manifest = JSON.parse(await readFile(path.join(dir, 'model.json'), 'utf8')) as { files?: Array<{ name: string; sha256: string }> };
      const current = new Set(model.files.map((file) => `${file.name}:${file.sha256}`));
      if ((manifest.files ?? []).some((file) => current.has(`${file.name}:${file.sha256}`))) return;
    } catch {
      return;
    }
    await rm(dir, { recursive: true, force: true }).catch(() => undefined);
  }

  private async partialSize(model: CatalogModel): Promise<number> {
    const dir = this.partialDir(model.id);
    let total = 0;
    for (const file of model.files) {
      total += (await sizeOf(path.join(dir, file.name))) ?? (await sizeOf(path.join(dir, `${file.name}.part`))) ?? 0;
    }
    return total;
  }

  info(model: CatalogModel): TranscriptionModelInfo {
    const download = this.downloads.get(model.id);
    const size = modelSize(model);
    const base: TranscriptionModelInfo = {
      id: model.id,
      name: model.name,
      description: model.description,
      size,
      languages: model.languages,
      languageCodes: [...model.languageCodes],
      speed: model.speed,
      accuracy: model.accuracy,
      recommended: model.recommended,
      wordTimestamps: model.wordTimestamps,
      timing: model.timing,
      detectsLanguage: model.detectsLanguage,
      featured: model.featured,
      state: 'absent'
    };
    if (this.ready.has(model.id)) return { ...base, state: 'ready' };
    if (download) {
      return { ...base, state: 'downloading', progress: download.total ? Math.min(1, download.received / download.total) : 0, downloadedBytes: download.received };
    }
    const error = this.errors.get(model.id);
    const partial = this.partialBytes.get(model.id) ?? 0;
    return {
      ...base,
      ...(error ? { state: 'error' as const, error } : {}),
      ...(partial > 0 ? { downloadedBytes: partial, progress: Math.min(1, partial / size) } : {})
    };
  }

  status(): TranscriptionStatus {
    let storageBytes = 0;
    for (const model of this.options.catalog) {
      if (this.ready.has(model.id)) storageBytes += modelSize(model);
      else storageBytes += this.downloads.get(model.id)?.received ?? this.partialBytes.get(model.id) ?? 0;
    }
    return {
      models: this.options.catalog.map((model) => this.info(model)),
      activeModelId: this.settings.activeModelId && this.ready.has(this.settings.activeModelId) ? this.settings.activeModelId : null,
      available: true,
      language: this.settings.language,
      storageBytes
    };
  }

  /** The active model and its folder, or null when none is ready. */
  activeModel(): { model: CatalogModel; dir: string } | null {
    const id = this.settings.activeModelId;
    if (!id || !this.ready.has(id)) return null;
    return { model: this.model(id), dir: this.modelDir(id) };
  }

  /**
   * The model a request runs on: the active one, unless the request needs
   * word timing it lacks; then a downloaded model that times words, one that
   * lists the language first. Null when nothing ready fits.
   */
  modelFor(needs: { wordTimestamps?: boolean; language?: string } = {}): { model: CatalogModel; dir: string } | null {
    const active = this.activeModel();
    if (!needs.wordTimestamps || active?.model.wordTimestamps) return active;
    const timed = this.options.catalog.filter((model) => model.wordTimestamps && this.ready.has(model.id));
    const base = needs.language && needs.language !== 'auto' ? needs.language.split('-')[0]!.toLowerCase() : '';
    const model = (base ? timed.find((entry) => entry.languageCodes.includes(base)) : undefined) ?? timed[0];
    return model ? { model, dir: this.modelDir(model.id) } : null;
  }

  language(): string { return this.settings.language; }

  private notify(immediate = false): void {
    const run = () => {
      this.notifyTimer = null;
      this.lastNotify = Date.now();
      this.options.onChange?.(this.status());
    };
    const interval = this.options.progressIntervalMs ?? 200;
    if (immediate) {
      if (this.notifyTimer) clearTimeout(this.notifyTimer);
      run();
      return;
    }
    if (this.notifyTimer) return;
    this.notifyTimer = setTimeout(run, Math.max(0, interval - (Date.now() - this.lastNotify)));
  }

  private persist(): Promise<void> {
    const snapshot = JSON.stringify(this.settings);
    this.settingsWrite = this.settingsWrite.then(async () => {
      const file = path.join(this.options.root, 'settings.json');
      const temporary = `${file}.${process.pid}.tmp`;
      await mkdir(this.options.root, { recursive: true });
      await writeFile(temporary, `${snapshot}\n`, 'utf8');
      await rename(temporary, file);
    }).catch((error: unknown) => console.error('[transcription] could not save settings', error));
    return this.settingsWrite;
  }

  async setActive(id: string): Promise<void> {
    await this.load();
    this.model(id);
    if (!this.ready.has(id)) throw new Error('Download this model before using it.');
    this.settings.activeModelId = id;
    await this.persist();
    this.notify(true);
  }

  async setLanguage(language: string): Promise<void> {
    await this.load();
    if (!LANGUAGE_TAG.test(language)) throw new Error('Choose a language.');
    this.settings.language = language;
    await this.persist();
    this.notify(true);
  }

  /** Starts (or joins) the download; resolves when the model is ready. */
  async download(id: string): Promise<void> {
    await this.load();
    const model = this.model(id);
    if (this.ready.has(id)) return;
    const running = this.downloads.get(id);
    if (running) return running.promise;
    const controller = new AbortController();
    const entry: Download = {
      controller,
      received: this.partialBytes.get(id) ?? 0,
      total: modelSize(model),
      promise: Promise.resolve()
    };
    this.errors.delete(id);
    this.downloads.set(id, entry);
    entry.promise = this.run(model, entry).then(async () => {
      this.ready.add(id);
      this.partialBytes.delete(id);
      if (!this.settings.activeModelId || !this.ready.has(this.settings.activeModelId)) {
        this.settings.activeModelId = id;
        await this.persist();
      }
    }, async (error: unknown) => {
      this.partialBytes.set(id, await this.partialSize(model));
      if (controller.signal.aborted && !(controller.signal.reason instanceof TranscriptionDownloadError)) return;
      const message = error instanceof Error && error.message ? error.message : 'The download failed. Try again.';
      this.errors.set(id, message);
      throw error;
    }).finally(() => {
      if (this.downloads.get(id) === entry) this.downloads.delete(id);
      this.notify(true);
    });
    this.notify(true);
    return entry.promise;
  }

  /** Stops a download. What arrived stays on disk so the next attempt resumes. */
  async cancel(id: string): Promise<void> {
    const running = this.downloads.get(id);
    if (!running) return;
    running.controller.abort();
    await running.promise.catch(() => undefined);
  }

  async remove(id: string): Promise<void> {
    await this.load();
    this.model(id);
    await this.cancel(id);
    this.ready.delete(id);
    this.errors.delete(id);
    this.partialBytes.delete(id);
    await rm(this.modelDir(id), { recursive: true, force: true });
    await rm(this.partialDir(id), { recursive: true, force: true });
    if (this.settings.activeModelId === id) {
      this.settings.activeModelId = this.options.catalog.find((model) => this.ready.has(model.id))?.id ?? null;
      await this.persist();
    }
    this.notify(true);
  }

  /** Cancels every running download (app quit). Partial files stay for next time. */
  async dispose(): Promise<void> {
    await Promise.all([...this.downloads.keys()].map((id) => this.cancel(id)));
    if (this.notifyTimer) clearTimeout(this.notifyTimer);
    this.notifyTimer = null;
  }

  private async run(model: CatalogModel, entry: Download): Promise<void> {
    const partial = this.partialDir(model.id);
    await mkdir(partial, { recursive: true });
    const have = await this.partialSize(model);
    entry.received = have;
    const needed = modelSize(model) - have;
    const free = await (this.options.freeBytes ?? defaultFreeBytes)(this.options.root).catch(() => Infinity);
    if (free < needed + DISK_HEADROOM) {
      throw new TranscriptionDownloadError(`Not enough disk space. ${model.name} needs ${formatBytes(needed + DISK_HEADROOM)} free.`);
    }
    let before = 0;
    for (const file of model.files) {
      const done = path.join(partial, file.name);
      if ((await sizeOf(done)) !== file.size) {
        await this.fetchFile(file, `${done}.part`, entry, before);
        await rename(`${done}.part`, done);
      }
      before += file.size;
      entry.received = before;
    }
    await writeFile(path.join(partial, 'model.json'), `${JSON.stringify({
      id: model.id,
      files: model.files.map(({ name, size, sha256 }) => ({ name, size, sha256 })),
      completedAt: new Date().toISOString()
    }, null, 2)}\n`, 'utf8');
    const final = this.modelDir(model.id);
    await rm(final, { recursive: true, force: true });
    await rename(partial, final);
  }

  private async fetchFile(file: CatalogFile, part: string, entry: Download, before: number, attempt = 0): Promise<void> {
    const signal = entry.controller.signal;
    let offset = (await sizeOf(part)) ?? 0;
    if (offset > file.size) {
      await truncate(part, 0);
      offset = 0;
    }
    let hash = createHash('sha256');
    if (offset > 0) await hashFile(part, hash);
    entry.received = before + offset;
    if (offset < file.size) {
      /* A stalled connection never errors on its own: abort it after a quiet spell. */
      const local = new AbortController();
      const forward = () => local.abort(signal.reason);
      if (signal.aborted) forward();
      else signal.addEventListener('abort', forward, { once: true });
      const stallMs = this.options.stallMs ?? 60_000;
      let stall: ReturnType<typeof setTimeout> | null = null;
      const arm = () => {
        if (stall) clearTimeout(stall);
        stall = setTimeout(() => local.abort(new TranscriptionDownloadError('The download stalled. Check your connection and try again.')), stallMs);
      };
      let out: WriteStream | null = null;
      try {
        arm();
        const response = await this.options.fetch(file.url, { headers: offset > 0 ? { Range: `bytes=${offset}-` } : {}, signal: local.signal });
        if (offset > 0 && response.status === 416 && attempt === 0) {
          /* What is on disk is not a prefix the server recognises: start the file over. */
          await response.body?.cancel().catch(() => undefined);
          await truncate(part, 0);
          return await this.fetchFile(file, part, entry, before, attempt + 1);
        }
        if (!response.ok || !response.body) throw new TranscriptionDownloadError(`The download failed (HTTP ${response.status}). Try again.`);
        if (offset > 0 && response.status !== 206) {
          /* The server ignored the range and sent the whole file. */
          offset = 0;
          hash = createHash('sha256');
          entry.received = before;
        }
        out = createWriteStream(part, { flags: offset > 0 ? 'a' : 'w' });
        await this.pump(response, out, hash, entry, arm);
        await close(out);
        out = null;
      } catch (error) {
        if (local.signal.aborted && local.signal.reason instanceof TranscriptionDownloadError) throw local.signal.reason;
        throw error;
      } finally {
        if (stall) clearTimeout(stall);
        signal.removeEventListener('abort', forward);
        out?.destroy();
      }
    }
    await this.verify(file, part, hash);
  }

  private async pump(response: Response, out: WriteStream, hash: ReturnType<typeof createHash>, entry: Download, arm: () => void): Promise<void> {
    for await (const chunk of response.body as unknown as AsyncIterable<Uint8Array>) {
      arm();
      hash.update(chunk);
      await write(out, chunk);
      entry.received += chunk.byteLength;
      this.notify();
    }
  }

  private async verify(file: CatalogFile, part: string, hash: ReturnType<typeof createHash>): Promise<void> {
    const size = await sizeOf(part);
    if (size !== file.size || hash.digest('hex') !== file.sha256) {
      await rm(part, { force: true });
      throw new TranscriptionDownloadError('The downloaded model did not match its checksum. Try again.');
    }
  }
}

async function defaultFreeBytes(dir: string): Promise<number> {
  const info = await statfs(dir);
  return Number(info.bavail) * Number(info.bsize);
}

/** Every regular file under the models root, for "storage used" checks in tests. */
export async function filesUnder(root: string): Promise<string[]> {
  const out: string[] = [];
  const walk = async (dir: string) => {
    for (const entry of await readdir(dir, { withFileTypes: true }).catch(() => [])) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) await walk(full);
      else if (entry.isFile()) out.push(full);
    }
  };
  await walk(root);
  return out;
}
