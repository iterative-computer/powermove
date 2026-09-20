/*
 * The document engine: the editor's own core (model, animation, history,
 * editing, the agent tool harness) running in Node against a DOM shim, with
 * no browser and no GPU. It connects to the host like a tab, joins project
 * sessions, and answers the agent's document tools, so a run keeps going
 * after every browser tab has gone.
 *
 * Panel and capture tools need pixels and a real UI; the host routes those
 * to a live tab and this engine never sees them.
 */
import { Window } from 'happy-dom';
import { WebSocket } from 'ws';

import { IPC, type AgentToolRequestEvent, type AgentToolResponseEvent, type PowermoveBridge } from '../shared/ipc';
import { Connection } from '../shared/link';
import { WEB } from '../shared/wire';

export interface EngineOptions {
  /** wss://127.0.0.1:<port>/__powermove/ws */
  url: string;
  token: string;
  log?: (line: string) => void;
}

/** A tool request the host forwards to the engine, tagged with its project. */
export interface EngineToolRequest extends AgentToolRequestEvent { projectId: string }

function installDom(): Window {
  const window = new Window({ url: 'https://engine.powermove.local/' });
  const globals = globalThis as Record<string, unknown>;
  globals['window'] = window;
  for (const key of ['document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'matchMedia', 'localStorage', 'sessionStorage', 'Image', 'Blob', 'File', 'FileReader', 'DOMParser', 'XMLSerializer', 'MutationObserver', 'ResizeObserver', 'IntersectionObserver']) {
    const value = (window as unknown as Record<string, unknown>)[key];
    if (value !== undefined && globals[key] === undefined) globals[key] = typeof value === 'function' && !/^[A-Z]/.test(key) ? (value as (...a: unknown[]) => unknown).bind(window) : value;
  }
  return window;
}

/** Everything the renderer's core needs from the bridge; the rest is absent. */
function bridgeStub(agentTools: PowermoveBridge['agentTools']): Partial<PowermoveBridge> {
  return {
    agentTools,
    remote: true,
    versions: { electron: '', chrome: '', node: process.versions.node },
    log: (level, text) => console.log(`[engine:${level}] ${text}`)
  };
}

function memoryStore(): Record<string, unknown> {
  const values = new Map<string, unknown>();
  return {
    get: (key: string, fallback: unknown) => values.has(key) ? JSON.parse(JSON.stringify(values.get(key))) : fallback,
    set: (key: string, value: unknown) => { values.set(key, JSON.parse(JSON.stringify(value ?? null))); return true; },
    setAsync: (key: string, value: unknown) => { values.set(key, JSON.parse(JSON.stringify(value ?? null))); return Promise.resolve(true); },
    del: (key: string) => { values.delete(key); },
    flush: async () => {}
  };
}

export async function startEngine(options: EngineOptions): Promise<{ close(): void }> {
  const log = options.log ?? ((line: string) => console.log(line));
  installDom();

  let onRequest: ((request: AgentToolRequestEvent) => void) | null = null;
  const responses: Array<(response: AgentToolResponseEvent) => void> = [];
  const agentTools: PowermoveBridge['agentTools'] = {
    onRequest: (cb) => { onRequest = cb; return () => { if (onRequest === cb) onRequest = null; }; },
    respond: (response) => { for (const listener of responses) listener(response); }
  };
  (window as unknown as { powermove: unknown }).powermove = bridgeStub(agentTools);

  const PM = ((window as unknown as { PM?: Record<string, any> }).PM = {});
  const [diag, util, kernel, uiState, memory, fonts, easing, model, selection, anim, history, library, projects, editing, capabilities, workspace, harness] = await Promise.all([
    import('../renderer/src/legacy/core/diag'), import('../renderer/src/legacy/core/util'), import('../renderer/src/kernel/install'),
    import('../renderer/src/legacy/core/ui-state'), import('../renderer/src/legacy/core/memory'), import('../renderer/src/legacy/core/fonts'),
    import('../renderer/src/legacy/core/easing'), import('../renderer/src/legacy/core/model'), import('../renderer/src/legacy/core/selection'),
    import('../renderer/src/legacy/core/anim'), import('../renderer/src/legacy/core/history'), import('../renderer/src/legacy/core/library'),
    import('../renderer/src/legacy/core/projects'), import('../renderer/src/legacy/core/editing'), import('../renderer/src/legacy/core/capabilities'),
    import('../renderer/src/legacy/core/workspace'), import('../renderer/src/legacy/assistant/harness')
  ]);
  diag.install(PM as never); util.install(PM as never);
  PM['store'] = memoryStore();
  void kernel.installKernel(PM as never);
  for (const step of [uiState, memory, fonts, easing, model, selection, anim, history, library, projects, editing, capabilities, workspace, harness]) step.install(PM as never);
  if (!onRequest) throw new Error('engine: the agent tool harness did not register');
  PM['proj'] = PM['mkProject']({ name: 'Untitled' });
  PM['isHomeProject'] = () => !PM['proj']?.id;

  /* the link to the host */
  const socket = new WebSocket(options.url, { rejectUnauthorized: false, headers: { cookie: `pm_session=${options.token}` } });
  await new Promise<void>((resolve, reject) => {
    socket.once('open', () => resolve());
    socket.once('error', (error) => reject(error));
  });
  const link = new Connection(socket as unknown as ConstructorParameters<typeof Connection>[0]);
  const { attachRemoteSync } = await import('../renderer/src/host/remote-sync');
  const detachSync = attachRemoteSync(link, PM as never);

  /* project switching: a tool call names the project it works on */
  let current: string | null = null;
  const ensureProject = async (projectId: string): Promise<void> => {
    if (current === projectId && PM['proj']?.id === projectId) return;
    const result = await link.invoke<{ seq: number; doc: unknown | null }>(WEB.syncJoin, { projectId, doc: null });
    if (!result.doc) throw new Error(`The host has no document for project ${projectId}.`);
    current = projectId;
    PM['hist']?.clear?.();
    PM['replaceProject'](result.doc, {});
    PM['bus']?.emit('projects:open');
    log(`[engine] opened ${projectId} (seq ${result.seq})`);
  };

  responses.push((response) => link.send(IPC.agentToolResponse, response));
  link.on(IPC.agentToolRequest, (payload) => {
    const request = payload as EngineToolRequest;
    void (async () => {
      try {
        await ensureProject(request.projectId);
        onRequest!(request);
      } catch (error) {
        link.send(IPC.agentToolResponse, { runId: request.runId, callId: request.callId, ok: false, content: [], error: error instanceof Error ? error.message : String(error) } satisfies AgentToolResponseEvent);
      }
    })();
  });
  link.on('__closed', () => { log('[engine] host connection closed'); process.exit(0); });
  await link.invoke(WEB.hello);
  log('[engine] connected');
  return { close: () => { detachSync(); socket.close(); } };
}

/* child-process entry: node engine.mjs <url> <token> */
if (process.argv[1] && /engine\.(m?js|ts)$/.test(process.argv[1]) && process.argv[2]) {
  startEngine({ url: process.argv[2]!, token: process.argv[3] ?? '' }).catch((error) => {
    console.error('[engine] failed to start', error);
    process.exit(1);
  });
}
