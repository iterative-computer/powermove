/*
 * The kernel's engine connection (plan sections 6, 15, 18, 19, 21; the
 * Phase 2 tracer bullet), behind "Use New Engine".
 *
 * With the new engine on, main gives this window a port to the interactive
 * engine for every launch (`powermove:engine-port`, through the preload).
 * This module:
 *
 * - opens the connection and registers `engine.viewer` (the viewer's
 *   `EngineViewerService`), dispatching `ENGINE_VIEWER_CHANGED` so an open
 *   viewer attaches at once;
 * - gives each viewer session the window's canonical document and pm-wasm's
 *   patch stream (`DocumentSession`), so edits from the inspector, canvas
 *   gestures, Undo, and agents reach the engine as they reach pm-wasm;
 * - runs the transport through the engine while it draws the composition
 *   (`EngineTransport`, `PM.setTransportDriver`), the audio device clock as
 *   the master; when the engine routes the composition to the legacy viewer
 *   (capability routing) the legacy transport, compositor, and audio take
 *   over at the same time, and back;
 * - survives engine restarts: a crashed engine's sessions end, the legacy
 *   viewer shows, and the next launch's port opens new sessions that resync
 *   the whole document (section 19, "Crash handling").
 *
 * With the flag off nothing here runs.
 */
import type { PMRegistry } from '../legacy/registry';
import type { FrameBridgeApi } from '../../../shared/frame-bridge';
import type { EngineBridge } from '../../../shared/engine-ipc';
import { ENGINE_WINDOW_MESSAGE } from '../../../shared/engine-ipc';
import type { EngineViewerChannel, EngineViewerHandlers, EngineViewerService } from '../../../extensions/viewer/engine-present/presentation';
import type { ClockSample } from '../../../shared/engine-protocol';
import { EngineConnection, type Log, type PortLike } from './connection';
import { HostClock } from './host-clock';
import { DocumentSession, type DocumentStream } from './session';
import { EngineTransport } from './transport';

/** Mirrors engine-present/presentation.ts (the renderer does not import extension values). */
export const ENGINE_VIEWER_SERVICE = 'engine.viewer';
export const ENGINE_VIEWER_CHANGED = 'powermove:engine-viewer-changed';

export type EngineConnectionHost = {
  nativeEngine?: EngineBridge;
  frameBridge?: FrameBridgeApi;
};

/** Diagnostics for the e2e suite and the measurements (`PM.EngineConnection`). */
export type EngineConnectionStatus = {
  connected: boolean;
  generation: number;
  connections: number;
  sessions: number;
  route: string;
  routeReason: string | null;
  frames: number;
  revision: number;
  resyncs: number;
  batches: number;
  failures: string[];
  driving: boolean;
  hostOffsetUs: number | null;
  hostOffsetErrorUs: number | null;
  clock: unknown;
};

export interface EngineConnectionHandle {
  status(): EngineConnectionStatus;
  /**
   * Diagnostics (the measurement harness): the viewer's presentation keeps
   * a per-frame trace while on. Registers `engine.viewer` again so the
   * viewer opens a new presentation with it.
   */
  setTrace(on: boolean): void;
  /** Every clock sample of every session (the measurements). */
  onClock(listener: (sample: ClockSample) => void): () => void;
  /** The session the transport follows (the newest viewer session). */
  session(): DocumentSession | null;
  hostClock: HostClock;
  transport: EngineTransport;
  dispose(): void;
}

export function installEngineConnection(PM: PMRegistry, host: EngineConnectionHost | null | undefined, target: Window = window): EngineConnectionHandle | null {
  const native = host?.nativeEngine;
  let enabled = false;
  try {
    enabled = native?.enabledSync?.() === true;
  } catch {
    enabled = false;
  }
  const source = host?.frameBridge;
  if (!enabled || !native || !source || !PM.Kernel?.services) return null;
  const log: Log = (level, message) => {
    const line = `[engine] ${message}`;
    if (level === 'error') console.error(line);
    else if (level === 'warn') console.warn(line);
    else console.info(line);
  };
  const hostClock = new HostClock(() => native.hostTime());
  hostClock.start();
  let connection: EngineConnection | null = null;
  let generation = 0;
  let connections = 0;
  const ready = new Set<() => void>();
  const sessions = new Set<DocumentSession>();
  let current: DocumentSession | null = null;
  const transport = new EngineTransport(() => current, hostClock);
  PM.setTransportDriver?.(transport);

  // Edits reach the engine as soon as the write that made them returns: a
  // write that leaves the barrier dirty schedules one flush at the end of
  // the current task (one per synchronous run, however many writes), so
  // the batch does not wait for the next pm-wasm query (an overlay draw at
  // the next animation frame). Only while an engine session is open.
  let flushQueued = false;
  let flushing = false;
  const flushSoon = (): void => {
    // Marks made while this connection's own flush runs are part of it.
    if (flushQueued || flushing || !sessions.size) return;
    flushQueued = true;
    queueMicrotask(() => {
      flushQueued = false;
      if (!sessions.size) return;
      flushing = true;
      try {
        PM.EngineDoc?.barrier?.flush();
      } catch (error) {
        log('warn', `flushing the document for the engine failed: ${error instanceof Error ? error.message : String(error)}`);
      } finally {
        flushing = false;
      }
    });
  };
  const offPending = PM.EngineDoc?.onPending?.(flushSoon) ?? (() => {});

  const stream = (): DocumentStream => {
    const doc = PM.EngineDoc;
    if (!doc?.enabled) throw new Error('the document mirror (pm-wasm) is not enabled');
    return doc as DocumentStream;
  };

  async function connect(nextGeneration: number, port: PortLike): Promise<void> {
    connection?.close('a new interactive engine launch');
    const next = new EngineConnection(nextGeneration, port, log);
    connection = next;
    generation = nextGeneration;
    next.onClose((reason) => {
      if (connection === next) connection = null;
      log('info', `the interactive engine connection closed (${reason})`);
      transport.refresh();
    });
    try {
      await next.open();
    } catch (error) {
      log('warn', `the interactive engine did not connect: ${error instanceof Error ? error.message : String(error)}`);
      next.close('handshake failed');
      return;
    }
    if (connection !== next) return;
    connections++;
    if (connections === 1) performance.mark?.('powermove:engine-connected');
    void hostClock.measure();
    log('info', `connected to the interactive engine (launch ${nextGeneration})`);
    for (const listener of [...ready]) listener();
  }

  const onMessage = (event: MessageEvent): void => {
    if (event.source !== target) return;
    const data = event.data as { type?: string; process?: string; generation?: number; report?: unknown } | null;
    if (data?.type === ENGINE_WINDOW_MESSAGE.port) {
      if (data.process !== 'interactive' || !event.ports[0]) return;
      void connect(Number(data.generation) || 0, event.ports[0] as unknown as PortLike);
    } else if (data?.type === ENGINE_WINDOW_MESSAGE.crash) {
      log('warn', 'an engine crashed and is restarting; the current engine draws meanwhile');
    }
  };
  target.addEventListener('message', onMessage);
  // Ports main posted before this listener existed (the window loaded
  // before the kernel booted) were dropped: ask for fresh ones.
  native.requestPorts?.();

  const clockListeners = new Set<(sample: ClockSample) => void>();
  /** Whether a first engine frame arrived (the cold start mark). */
  let firstFrame = false;
  let trace = false;
  const makeService = (): EngineViewerService & { trace?: boolean } => ({
    trace,
    source,
    composition: () => String(PM.proj?.compId ?? PM.proj?.id ?? ''),
    hostOffsetUs: () => hostClock.offsetUs(),
    revision: () => {
      const barrier = PM.EngineDoc?.barrier;
      if (!barrier) return null;
      barrier.flush();
      return barrier.revision;
    },
    onReady(listener) {
      ready.add(listener);
      return () => ready.delete(listener);
    },
    open(handlers: EngineViewerHandlers): EngineViewerChannel | null {
      const c = connection;
      if (!c?.ready || c.closed || !PM.EngineDoc?.enabled) return null;
      let session: DocumentSession | null = null;
      const forget = (s: DocumentSession): void => {
        sessions.delete(s);
        if (current === s) current = [...sessions].at(-1) ?? null;
        transport.refresh();
      };
      session = new DocumentSession({
        connection: c,
        stream: stream(),
        documentId: () => String(PM.proj?.id ?? ''),
        stage: (id, bytes) => native.stageDocument({ process: 'interactive', session: id, bytes }),
        handlers: {
          message: (message, arrivedMs) => {
            if (message.type === 'frame' && !firstFrame) {
              firstFrame = true;
              performance.mark?.('powermove:engine-first-frame');
            }
            handlers.message(message as never, arrivedMs);
          },
          closed: (reason) => {
            if (session) forget(session);
            handlers.closed(reason);
          }
        },
        log,
        onClock: (sample) => {
          for (const listener of clockListeners) listener(sample);
        },
        // The viewer lays out for the new document, as opening a project
        // makes it do: its state names the new composition, so the engine
        // draws and plays it (a `PM.proj` assignment emits nothing).
        onLoaded: () => PM.bus?.emit?.('layout'),
        onRoute: (route, reason) => {
          if (route === 'legacy') log('info', `the current engine draws this composition: ${reason ?? ''}`);
          transport.refresh();
          PM.bus?.emit?.('draw');
        }
      });
      sessions.add(session);
      current = session;
      transport.resetLoop();
      const opened = session;
      void opened.start().catch((error: unknown) => {
        log('warn', `engine viewer session: ${error instanceof Error ? error.message : String(error)}`);
      });
      return {
        sendViewerState: (state) => opened.sendViewerState(state as never),
        release: (release) => opened.release(release),
        close: () => {
          opened.close();
          forget(opened);
        }
      };
    }
  });
  let registration = PM.Kernel.services.register(ENGINE_VIEWER_SERVICE, makeService());
  target.dispatchEvent(new Event(ENGINE_VIEWER_CHANGED));

  const handle: EngineConnectionHandle = {
    hostClock,
    transport,
    session: () => current,
    setTrace: (on) => {
      if (on === trace) return;
      trace = on;
      registration.dispose();
      registration = PM.Kernel.services.register(ENGINE_VIEWER_SERVICE, makeService());
      target.dispatchEvent(new Event(ENGINE_VIEWER_CHANGED));
    },
    onClock: (listener) => {
      clockListeners.add(listener);
      return () => clockListeners.delete(listener);
    },
    status: () => ({
      connected: !!connection?.ready && !connection.closed,
      generation,
      connections,
      sessions: sessions.size,
      route: current?.route ?? 'none',
      routeReason: current?.routeReason ?? null,
      frames: current?.frames ?? 0,
      revision: current?.applied ?? 0,
      resyncs: current?.resyncs ?? 0,
      batches: current?.batches ?? 0,
      failures: current?.failures.slice() ?? [],
      driving: transport.active(),
      hostOffsetUs: hostClock.offsetUs(),
      hostOffsetErrorUs: hostClock.errorUs(),
      clock: current?.clock ?? null
    }),
    dispose: () => {
      offPending();
      target.removeEventListener('message', onMessage);
      PM.setTransportDriver?.(null);
      registration.dispose();
      target.dispatchEvent(new Event(ENGINE_VIEWER_CHANGED));
      for (const s of [...sessions]) s.close();
      sessions.clear();
      current = null;
      connection?.close('the engine connection was removed');
      connection = null;
      hostClock.stop();
    }
  };
  PM.EngineConnection = handle;
  return handle;
}
