/*
 * Entry point of an engine utility process (the interactive or the job
 * engine; plan section 19). Built to out/main/engine-process.js and started
 * by main with `utilityProcess.fork`.
 *
 * It loads the pm-node addon, takes its control port from main over its
 * parent port, takes ports over the control port (`connect` with the peer
 * kind main assigned), and moves bytes between
 * the ports and the engine. The engine decides everything; this file only
 * carries out its actions. Engine work never runs here beyond decoding
 * and dispatch: rendering lanes run on the engine's own threads.
 */
import type { MessagePortMain } from 'electron';

import type { EngineBootstrap, EngineControl, EngineNotice } from './ports';
import { isReleasePortOffer } from '../../shared/release-port';

type EngineAction = { kind: 'send' | 'close' | 'log'; port: number; bytes?: Uint8Array | null; level?: string | null; message?: string | null };

type Addon = {
  /** `onActions` receives the actions the engine produces off this thread
   *  (the viewer presenter's frame descriptors, clock samples, and frame
   *  responses, sent straight to the viewer's port: amendment 4). */
  EngineHost: new (process: string, onActions?: (actions: EngineAction[]) => void) => {
    attach(port: number, peer: string, owner?: number | null): EngineAction[];
    detach(port: number): EngineAction[];
    receive(port: number, bytes: Uint8Array): EngineAction[];
    tick(): EngineAction[];
    /** What threads off this one produced (host calls, job status). */
    pump(): EngineAction[];
    status(): string;
    simulateGpuLoss?(): boolean;
    /** The viewer presenter's per-frame trace as JSON, emptied (POWERMOVE_BRIDGE_TRACE=1). */
    presentTrace?(): string;
  };
  buildId(): string;
  hostNowUs(): number;
};

/** How often the engine reports leases and breadcrumbs. */
const TICK_MS = 100;

const parent = process.parentPort;

function notify(notice: EngineNotice): void {
  parent.postMessage(notice);
}

function loadAddon(file: string): Addon {
  const module = { exports: {} as Addon };
  process.dlopen(module, file);
  return module.exports;
}

function main(): void {
  const kind = process.env['POWERMOVE_ENGINE_PROCESS'];
  const addonPath = process.env['POWERMOVE_ENGINE_ADDON'];
  if ((kind !== 'interactive' && kind !== 'job') || !addonPath) {
    notify({ type: 'fatal', message: 'POWERMOVE_ENGINE_PROCESS and POWERMOVE_ENGINE_ADDON must be set' });
    process.exit(78);
  }
  let addon: Addon;
  let engine: InstanceType<Addon['EngineHost']>;
  const ports = new Map<number, MessagePortMain>();
  try {
    addon = loadAddon(addonPath);
    // An empty batch is a wake-up from a thread off this one (job lanes,
    // the viewer's Blender worker): its host calls wait in `pump`.
    engine = new addon.EngineHost(kind, (actions) => run(actions.length ? actions : engine.pump()));
  } catch (error) {
    notify({ type: 'fatal', message: `could not load the engine: ${error instanceof Error ? error.message : String(error)}` });
    process.exit(70);
  }

  function run(actions: EngineAction[]): void {
    for (const action of actions) {
      if (action.kind === 'send') {
        const port = ports.get(action.port);
        if (port && action.bytes) port.postMessage(action.bytes);
      } else if (action.kind === 'close') {
        const port = ports.get(action.port);
        ports.delete(action.port);
        port?.close();
      } else if (action.message) {
        notify({ type: 'log', level: action.level === 'error' ? 'error' : action.level === 'warn' ? 'warn' : 'info', message: action.message });
      }
    }
  }

  let control: MessagePortMain | null = null;
  // The S1 harness's per-frame engine trace (render start, GPU done, sent;
  // host µs) rides on status reports while it is on, emptied by each.
  const traceOn = process.env['POWERMOVE_BRIDGE_TRACE'] === '1';
  const withPresentTrace = (json: string): string => {
    if (!traceOn || typeof engine.presentTrace !== 'function' || !json.startsWith('{') || !json.endsWith('}')) return json;
    return `${json.slice(0, -1)}${json.length > 2 ? ',' : ''}"presentTrace":${engine.presentTrace()}}`;
  };
  const shutdown = (): void => {
    for (const port of ports.values()) port.close();
    ports.clear();
    process.exit(0);
  };

  /**
   * A window's presentation worker hands frames back to the engine itself
   * (`RELEASE_PORT_OFFER`, shared/release-port.ts): the port the window sends over its own port
   * carries the same session-bound envelopes (frame releases) and is read
   * as that port, so a release never waits for the window's main thread.
   * It lives as long as the window's port.
   */
  const releasePorts = new Map<number, Set<MessagePortMain>>();
  function acceptReleasePort(owner: number, port: MessagePortMain): void {
    let set = releasePorts.get(owner);
    if (!set) releasePorts.set(owner, (set = new Set()));
    set.add(port);
    port.on('message', (incoming) => {
      const data = incoming.data as unknown;
      if (data instanceof Uint8Array && ports.has(owner)) run(engine.receive(owner, data));
    });
    port.on('close', () => releasePorts.get(owner)?.delete(port));
    port.start();
  }
  function closeReleasePorts(owner: number): void {
    for (const port of releasePorts.get(owner) ?? []) port.close();
    releasePorts.delete(owner);
  }

  const onControl = (event: Electron.MessageEvent): void => {
    const message = event.data as EngineControl;
    if (message?.type === 'connect') {
      const port = event.ports[0];
      if (!port || !Number.isSafeInteger(message.port)) return;
      ports.get(message.port)?.close();
      closeReleasePorts(message.port);
      ports.set(message.port, port);
      port.on('message', (incoming) => {
        const data = incoming.data as unknown;
        if (data instanceof Uint8Array) run(engine.receive(message.port, data));
        else if (isReleasePortOffer(data) && incoming.ports[0]) acceptReleasePort(message.port, incoming.ports[0]);
        else run(engine.detach(message.port).concat([{ kind: 'close', port: message.port }]));
      });
      port.on('close', () => {
        if (ports.get(message.port) !== port) return;
        ports.delete(message.port);
        closeReleasePorts(message.port);
        run(engine.detach(message.port));
      });
      try {
        run(engine.attach(message.port, message.peer, Number.isSafeInteger(message.owner) ? message.owner : null));
      } catch (error) {
        notify({ type: 'log', level: 'error', message: `could not attach port ${message.port}: ${error instanceof Error ? error.message : String(error)}` });
        ports.delete(message.port);
        port.close();
        return;
      }
      port.start();
    } else if (message?.type === 'disconnect') {
      const port = ports.get(message.port);
      ports.delete(message.port);
      closeReleasePorts(message.port);
      run(engine.detach(message.port));
      port?.close();
    } else if (message?.type === 'simulateGpuLoss') {
      const lost = engine.simulateGpuLoss?.() === true;
      notify({ type: 'log', level: 'warn', message: lost ? 'simulated a GPU device loss' : 'this engine does not present; no GPU device to lose' });
    } else if (message?.type === 'status' && Number.isSafeInteger(message.id)) {
      notify({ type: 'status', id: message.id, json: withPresentTrace(engine.status()) });
    } else if (message?.type === 'shutdown') {
      shutdown();
    }
  };

  parent.on('message', (event) => {
    const message = event.data as EngineBootstrap;
    const port = event.ports[0];
    if (message?.type !== 'control' || !port || control) return;
    control = port;
    control.on('message', onControl);
    // Main closed its end or went away: nothing can reach this engine any more.
    control.on('close', shutdown);
    control.start();
  });

  setInterval(() => run(engine.tick()), TICK_MS);
  // Measurement harnesses only: the job service's counters (its GPU gate's
  // submissions, probes, and waits) as a log line this often.
  const statusEveryMs = Number(process.env['POWERMOVE_ENGINE_STATUS_LOG_MS'] ?? 0);
  if (kind === 'job' && Number.isFinite(statusEveryMs) && statusEveryMs >= 100) {
    setInterval(() => {
      try {
        const jobs = (JSON.parse(engine.status()) as { jobs?: unknown }).jobs;
        if (jobs) notify({ type: 'log', level: 'info', message: `job service status ${JSON.stringify(jobs)}` });
      } catch {
        // A status the engine could not give is not worth a log line.
      }
    }, statusEveryMs);
  }
  notify({ type: 'started', build: addon.buildId(), pid: process.pid, clock: addon.hostNowUs() });
}

main();
