import { readFileSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { TRANSCRIPTION_IPC } from '../../shared/transcription';
import { TranscriptCache } from './cache';
import { CATALOG, parseCatalog, type CatalogModel } from './catalog';
import { TranscriptionEngine, type WorkerChannel } from './engine';
import { registerTranscriptionIpc } from './ipc';
import { ModelStore, type FetchLike } from './models';
import { setTranscriptionService } from './service';
import type { WorkerResponse } from './worker-protocol';

/*
 * Boots on-device transcription for a host: the model store, the engine and
 * its IPC, installed as the process-wide TranscriptionService. The desktop
 * app passes Electron's pieces (utilityProcess, net.fetch, the focused
 * editor); the serve host passes Node's.
 */

export interface TranscriptionHost {
  userData: string;
  ffmpeg: string;
  fetch: FetchLike;
  spawnWorker: () => WorkerChannel;
  ipc: Parameters<typeof registerTranscriptionIpc>[0];
  isTrustedSender: Parameters<typeof registerTranscriptionIpc>[1]['isTrustedSender'];
  /** Every document that should hear statusChanged (editors and onboarding). */
  broadcast(channel: string, payload: unknown): void;
  /** Sends modelRequested to the focused editor; false when there is none. */
  requestModel(reason: string): void;
  reveal(dir: string): Promise<void> | void;
  /** e2e only: a JSON catalog that replaces the pinned one. */
  catalogFile?: string | null;
  /** Packaged app: libtranscribe.dylib in app.asar.unpacked (dlopen cannot read the asar). */
  runtime?: string;
  /** Warm the engine once after launch for each new value (the app version):
   *  the first model load under a new app compiles the Metal shaders. Omit
   *  to skip (tests, the serve host). */
  warmKey?: string;
  /** How long after launch the warm-up waits for a quiet moment. */
  warmDelayMs?: number;
}

export interface InstalledTranscription {
  engine: TranscriptionEngine;
  models: ModelStore;
  dispose(): Promise<void>;
}

function catalogFor(file: string | null | undefined): readonly CatalogModel[] {
  if (!file) return CATALOG;
  try {
    return parseCatalog(JSON.parse(readFileSync(file, 'utf8')));
  } catch (error) {
    console.error('[transcription] ignoring the test catalog', error);
    return CATALOG;
  }
}

export function installTranscription(host: TranscriptionHost): InstalledTranscription {
  /* A model newly in use (downloaded first, or chosen) is loaded straight
     away, so the shader compile is done before captions ask for it. */
  let inUse: string | null | undefined;
  const models = new ModelStore({
    root: path.join(host.userData, 'models', 'transcription'),
    catalog: catalogFor(host.catalogFile),
    fetch: host.fetch,
    onChange: (status) => {
      host.broadcast(TRANSCRIPTION_IPC.statusChanged, status);
      if (inUse !== undefined && status.activeModelId && status.activeModelId !== inUse) void engine.warm();
      if (inUse !== undefined) inUse = status.activeModelId;
    }
  });
  const engine = new TranscriptionEngine({
    models,
    cache: new TranscriptCache(path.join(host.userData, 'transcription-cache')),
    ffmpeg: host.ffmpeg,
    spawnWorker: host.spawnWorker,
    requestModel: (reason) => host.requestModel(String(reason ?? '').slice(0, 300)),
    ...(host.runtime ? { runtime: host.runtime } : {})
  });
  void models.load().then(() => { inUse = models.status().activeModelId; }, (error: unknown) => console.error('[transcription] model scan failed', error));
  const warmTimer = host.warmKey ? setTimeout(() => { void warmOnce(engine, path.join(host.userData, 'transcription-warm.json'), host.warmKey!); }, host.warmDelayMs ?? 20_000) : null;
  warmTimer?.unref?.();
  registerTranscriptionIpc(host.ipc, { service: engine, models, isTrustedSender: host.isTrustedSender, reveal: host.reveal });
  setTranscriptionService(engine);
  return {
    engine,
    models,
    dispose: async () => {
      if (warmTimer) clearTimeout(warmTimer);
      engine.dispose();
      await models.dispose();
    }
  };
}

/**
 * The launch warm-up: once per key (the app version), load the model in use
 * and let the process go again. macOS keeps the compiled shaders, so later
 * launches skip this. Not recorded when there was nothing to load or the
 * load failed, so the next launch tries again.
 */
export async function warmOnce(engine: Pick<TranscriptionEngine, 'warm'>, marker: string, key: string): Promise<void> {
  try {
    const seen = JSON.parse(await readFile(marker, 'utf8').catch(() => '{}')) as { key?: unknown };
    if (seen.key === key) return;
    if (!(await engine.warm({ release: true }))) return;
    await writeFile(marker, `${JSON.stringify({ key })}\n`, 'utf8');
  } catch (error) {
    console.warn('[transcription] warm-up failed', error);
  }
}

/** A WorkerChannel over Electron's utilityProcess. */
export function utilityWorker(fork: (modulePath: string) => Electron.UtilityProcess, workerPath: string): () => WorkerChannel {
  return () => {
    const child = fork(workerPath);
    child.stderr?.on('data', (chunk: Buffer) => console.warn('[transcription worker]', chunk.toString().trimEnd()));
    return {
      post: (message) => child.postMessage(message),
      onMessage: (listener) => child.on('message', (message: WorkerResponse) => listener(message)),
      onExit: (listener) => child.once('exit', (code) => listener(code)),
      kill: () => { child.kill(); }
    };
  };
}
