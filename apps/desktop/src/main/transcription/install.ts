import { readFileSync } from 'node:fs';
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
  const models = new ModelStore({
    root: path.join(host.userData, 'models', 'transcription'),
    catalog: catalogFor(host.catalogFile),
    fetch: host.fetch,
    onChange: (status) => host.broadcast(TRANSCRIPTION_IPC.statusChanged, status)
  });
  const engine = new TranscriptionEngine({
    models,
    cache: new TranscriptCache(path.join(host.userData, 'transcription-cache')),
    ffmpeg: host.ffmpeg,
    spawnWorker: host.spawnWorker,
    requestModel: (reason) => host.requestModel(String(reason ?? '').slice(0, 300)),
    ...(host.runtime ? { runtime: host.runtime } : {})
  });
  void models.load().catch((error: unknown) => console.error('[transcription] model scan failed', error));
  registerTranscriptionIpc(host.ipc, { service: engine, models, isTrustedSender: host.isTrustedSender, reveal: host.reveal });
  setTranscriptionService(engine);
  return {
    engine,
    models,
    dispose: async () => {
      engine.dispose();
      await models.dispose();
    }
  };
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
