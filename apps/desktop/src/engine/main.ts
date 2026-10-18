/*
 * The document engine: the editor's own core (model, animation, history,
 * editing, the agent tool harness, captions, the extension registry) running
 * in Node against a DOM shim, with no browser and no GPU. It connects to the
 * host like a tab and answers the agent's document tools, so a run keeps
 * going after every browser tab has gone.
 *
 * This process is the supervisor: it owns a `DocumentHostPool` (one worker
 * process per open project, docs/engine/document-host.md) and gives it the
 * host's services over the link (the profile store, project sessions, media
 * paths). Each project keeps its own pm-wasm mirror, kernel, history, and
 * extension registry; opening another project never clears one.
 *
 * Panel and capture tools need pixels and a real UI; the host routes those
 * to a live tab and this engine never sees them.
 *
 *   node engine.mjs <url> <token>         the supervisor (serve's child)
 *   node engine.mjs --document-worker     one project's worker (the pool forks it)
 */
import { fileURLToPath } from 'node:url';
import { WebSocket } from 'ws';

import { IPC, type AgentToolRequestEvent, type AgentToolResponseEvent } from '../shared/ipc';
import { Connection } from '../shared/link';
import { WEB } from '../shared/wire';
import { MEDIA_PATH_IPC, type MediaPathResult } from '../shared/media-tools';
import { NEW_ENGINE_ENV, NEW_ENGINE_STORE_KEY, newEngineEnabled } from '../main/engine/flag';
import type { Patch } from '../shared/patch';
import { DocumentHostPool } from './document-host/pool';
import { DocumentHostError, type DocumentHostServices, type SessionPatch } from './document-host/protocol';
import { runWorkerProcess } from './document-host/worker';

export { installHeadlessEngineDoc, pmWasmPath, headlessFonts } from './document-host/chain';

export interface EngineOptions {
  /** wss://127.0.0.1:<port>/__powermove/ws */
  url: string;
  token: string;
  log?: (line: string) => void;
}

/** A tool request the host forwards to the engine, tagged with its project. */
export interface EngineToolRequest extends AgentToolRequestEvent { projectId: string }

/** The host's services over the link to the serve host. */
export function linkServices(link: Connection, log: (line: string) => void): DocumentHostServices {
  const patchListeners = new Set<(patch: SessionPatch) => void>();
  link.on(WEB.syncPatch, (payload) => {
    const message = payload as { projectId?: unknown; seq?: unknown; patches?: unknown };
    if (!message || typeof message.projectId !== 'string' || !Array.isArray(message.patches)) return;
    const patch: SessionPatch = { projectId: message.projectId, seq: Number(message.seq) || 0, patches: message.patches as Patch[] };
    for (const listener of patchListeners) listener(patch);
  });
  return {
    store: {
      read: async (key) => {
        const raw = await link.sync<string | null>(IPC.storeGetSync, { key });
        return raw == null ? null : JSON.parse(raw);
      },
      write: async (key, value) => { link.send(IPC.storeSet, { key, value }); },
      remove: async (key) => { link.send(IPC.storeDelete, { key }); },
    },
    session: {
      join: (projectId) => link.invoke<{ seq: number; doc: unknown | null }>(WEB.syncJoin, { projectId, doc: null }),
      patch: (projectId, patches) => link.invoke<number>(WEB.syncPatch, { projectId, patches }),
      leave: (projectId) => link.send(WEB.syncLeave, projectId),
      onPatch: (listener) => { patchListeners.add(listener); return () => { patchListeners.delete(listener); }; },
    },
    /* Agent media tools and captions find a project's media in the host's own
       store by key; the engine has no bytes of its own to stage. */
    mediaPath: { lookup: (request) => link.invoke<MediaPathResult | null>(MEDIA_PATH_IPC.lookup, request) },
    log,
  };
}

/** The built bundle this module is part of (engine.mjs), which the pool forks. */
function bundlePath(): string | undefined {
  const self = fileURLToPath(import.meta.url);
  return /engine\.m?js$/.test(self) ? self : process.env['POWERMOVE_ENGINE_SCRIPT'];
}

export async function startEngine(options: EngineOptions): Promise<{ close(): Promise<void>; pool: DocumentHostPool }> {
  const log = options.log ?? ((line: string) => console.log(line));

  const socket = new WebSocket(options.url, { rejectUnauthorized: false, headers: { cookie: `pm_session=${options.token}` } });
  await new Promise<void>((resolve, reject) => {
    socket.once('open', () => resolve());
    socket.once('error', (error) => reject(error));
  });
  const link = new Connection(socket as unknown as ConstructorParameters<typeof Connection>[0]);
  const services = linkServices(link, log);
  const stored = await services.store.read(NEW_ENGINE_STORE_KEY).catch(() => null);
  const newEngine = newEngineEnabled({ [NEW_ENGINE_ENV]: process.env[NEW_ENGINE_ENV] }, stored);
  const pool = new DocumentHostPool({ services, newEngine, script: bundlePath() });

  link.on(IPC.agentToolRequest, (payload) => {
    const request = payload as EngineToolRequest;
    void pool.call(request.projectId, request).then(
      (response) => link.send(IPC.agentToolResponse, response),
      (error) => link.send(IPC.agentToolResponse, {
        runId: request.runId, callId: request.callId, ok: false, content: [],
        error: error instanceof DocumentHostError ? `${error.message}${error.retry ? ' (retry the call)' : ''}` : error instanceof Error ? error.message : String(error),
      } satisfies AgentToolResponseEvent),
    );
  });
  link.on('__closed', () => {
    log('[engine] host connection closed');
    void pool.shutdown().finally(() => process.exit(0));
  });
  await link.invoke(WEB.hello);
  log(`[engine] connected (document host protocol ${pool.protocol}, new engine ${newEngine ? 'on' : 'off'})`);
  return {
    pool,
    close: async () => { await pool.shutdown(); socket.close(); },
  };
}

/* child-process entries */
if (process.argv[1] && /engine\.(m?js|ts)$/.test(process.argv[1])) {
  if (process.argv.includes('--document-worker')) {
    runWorkerProcess();
  } else if (process.argv[2]) {
    startEngine({ url: process.argv[2]!, token: process.argv[3] ?? '' }).catch((error) => {
      console.error('[engine] failed to start', error);
      process.exit(1);
    });
  }
}
