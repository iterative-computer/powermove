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
 * - keeps one window session for engine transport and pm-audio, independently
 *   of presentation; a closed viewer keeps the device clock and sound;
 * - survives engine restarts by resyncing the document and definitions on
 *   the next native launch, keeping transport exclusive to the native engine.
 *
 * With the flag off nothing here runs.
 */
import type { PMRegistry } from '../legacy/registry';
import type { FrameBridgeApi } from '../../../shared/frame-bridge';
import type { EngineBridge } from '../../../shared/engine-ipc';
import { ENGINE_WINDOW_MESSAGE } from '../../../shared/engine-ipc';
import type { EngineViewerChannel, EngineViewerHandlers, EngineViewerService } from '../../../extensions/viewer/engine-present/presentation';
import type { ClockSample } from '../../../shared/engine-protocol';
import { EngineAudio, routeAudioPanels } from './audio';
import { EngineConnection, type Log, type PortLike } from './connection';
import { HostClock } from './host-clock';
import { DocumentSession, documentId, type DocumentStream, type Route } from './session';
import { EngineRenderTransport } from './render-transport';
import { EngineStatsView, PresentedEvents, RevisionVersions, emitPresented } from './presentation-events';
import { EngineCanvasBinding } from './canvas-binding';
import { resolveFrameRate, TICKS_PER_SECOND } from '../../../shared/frame-rate';
import { installAppCaptures, usesBlender } from './app-captures';
// The presentation is platform code hosted in the viewer's folder; the
// kernel's `render.gl.init` binding uses it for viewers that do not present
// by themselves (old forks of the viewer).
import { ViewerPresentation } from '../../../extensions/viewer/engine-present/presentation';
import { ShaderFallbackRunner } from './shader-fallback';
import { SvgRasters } from './svg-rasters';
import { EngineTransport } from './transport';
import { followOpenComposition } from './composition-follow';

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
  /** The engine's audio for the mixer panel and waveforms. */
  audio: EngineAudio;
  /** Every clock sample of every session (the measurements). */
  onClock(listener: (sample: ClockSample) => void): () => void;
  /** The session the transport follows (the newest viewer session). */
  session(): DocumentSession | null;
  /** Engine snapshots, compatibility pixels, and UI snapshots (the Phase 3c
   *  render transport; also `PM.EngineRender`). */
  render: EngineRenderTransport;
  hostClock: HostClock;
  transport: EngineTransport;
  dispose(): void;
}

/**
 * The route message a viewer attaching to `session` missed: the session
 * already presents (no viewerless state in between, which would make the
 * engine say it again) a composition the engine cannot draw. Null
 * otherwise.
 */
export function missedRoute(session: { readonly headless: boolean; readonly route: Route; readonly routeReason: string | null }): { type: 'routed'; route: 'legacy'; reason: string } | null {
  if (session.headless || session.route !== 'legacy') return null;
  return { type: 'routed', route: 'legacy', reason: session.routeReason ?? '' };
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
  let fallbackRunner: ShaderFallbackRunner | null = null;
  // One session owns this window's clock. A viewer attaches to it; closing
  // that channel retires presentation only, leaving pm-audio and time alive.
  let viewerHandlers: EngineViewerHandlers | null = null;
  let viewerLease = 0;
  let sessionReady: Promise<void> | null = null;
  /** The open composition (what Play plays and the viewer shows). */
  const openComposition = (): string => String(PM.proj?.compId ?? PM.proj?.id ?? '');
  /** A viewerless session follows the open composition (composition-follow.ts). */
  const follower = followOpenComposition({ session: () => current, open: openComposition, bus: PM.bus });
  const transport = new EngineTransport(() => current, hostClock, {
    exclusive: true, wake: () => queueMicrotask(() => { void wakeStandby(); }),
    prepare: follower.follow
  });
  /** animVersion per revision sent, for `frame:rendered.version`. */
  const versions = new RevisionVersions();
  const noteVersion = (): void => {
    const barrier = PM.EngineDoc?.barrier;
    if (barrier) versions.note(barrier.revision, PM.animVersion?.());
  };
  const presentedEvents = new PresentedEvents(
    (frame, quality) => emitPresented(PM.bus, frame, quality, { viewport: () => PM.GL?.previewViewport ?? null, versions, project: () => PM.proj }),
    (fn) => target.requestAnimationFrame(fn)
  );
  /** Engine stats for the status bar and the performance panel. */
  const stats = new EngineStatsView();
  (PM as any).EngineStats = stats;
  (PM as any).EnginePresentedEvents = presentedEvents;
  PM.setTransportDriver?.(transport);

  // The engine's levels, monitor solo, and waveform peaks for today's audio
  // panels (the mixer's meters and solo, `asset.peaks`).
  const audio = new EngineAudio({
    session: () => current,
    driving: () => transport.active(),
    assets: () => (PM.assets?.map?.values?.() ?? []) as Iterable<Record<string, any>>,
    peaksChanged: () => {
      PM.invalidate?.('timeline');
      PM.bus?.emit?.('audio:peaks');
    }
  });
  const unrouteAudio = routeAudioPanels(PM.Audio, audio, () => transport.active());
  // Level 1 SVG images: Chromium draws today's bucket rasters for the
  // engine (decision S1, svg-rasters.ts); their content hashes are taken
  // ahead so a synchronous snapshot can draw one.
  const svgRasters = new SvgRasters({
    asset: (id) => PM.assets?.get?.(id) ?? null,
    assets: () => (PM.assets?.map?.values?.() ?? []) as Iterable<Record<string, any>>,
    log
  });
  svgRasters.prime();
  const offAssets = PM.bus?.on?.('assets', () => {
    audio.sync();
    svgRasters.prime();
  }) ?? (() => {});

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
        noteVersion();
      } catch (error) {
        log('warn', `flushing the document for the engine failed: ${error instanceof Error ? error.message : String(error)}`);
      } finally {
        flushing = false;
      }
    });
  };
  const offPending = PM.EngineDoc?.onPending?.(flushSoon) ?? (() => {});
  // And before the listeners of a document announcement (an edit's panels
  // and overlays), not after them.
  const offEager = PM.EngineDoc?.flushBeforeListeners?.(() => sessions.size > 0 && !flushing) ?? (() => {});

  const stream = (): DocumentStream => {
    const doc = PM.EngineDoc;
    if (!doc?.enabled) throw new Error('the document mirror (pm-wasm) is not enabled');
    return doc as DocumentStream;
  };

  const snapshotSession = (): DocumentSession | null => current;
  /** Lazily opens a transport session for a snapshot, Play, or a viewer. */
  const openSession = (): DocumentSession | null => {
    if (current && !current.closed) return current;
    const c = connection;
    if (!c?.ready || c.closed || !PM.EngineDoc?.enabled) return null;
    const session = new DocumentSession({
      connection: c, stream: stream(), documentId: () => String(PM.proj?.id ?? ''),
      stage: (id, bytes) => native.stageDocument({ process: 'interactive', session: id, bytes }),
      definitions: () => ((PM as any).EngineDefinitions ?? null),
      fallbackRunner: () => (fallbackRunner ??= new ShaderFallbackRunner()),
      // The `raster` override's answers (kernel/raster-compat.ts).
      onCompat: (message) => renderTransport.receiveCompat(message, session),
      svgRasters: () => svgRasters,
      handlers: {
        message: (message, arrivedMs) => {
          if (message.type === 'frame' && !firstFrame) {
            firstFrame = true;
            performance.mark?.('powermove:engine-first-frame');
          }
          if (viewerHandlers) viewerHandlers.message(message as never, arrivedMs);
          else if (message.type === 'frame') session.release(message.frame);
        },
        closed: (reason) => {
          sessions.delete(session);
          if (current === session) {
            current = null;
            sessionReady = null;
            const handlers = viewerHandlers;
            viewerHandlers = null;
            handlers?.closed(reason);
          }
          transport.refresh();
        }
      },
      log,
      onClock: (sample) => {
        transport.refresh();
        for (const listener of clockListeners) listener(sample);
      },
      onAudio: (message) => audio.receive(message, session),
      onStats: (message) => {
        stats.receive(message, performance.now());
        PM.invalidate?.('status');
      },
      // Replace a seek queued on the home screen before any old clock can
      // overwrite the replacement document's requested UI position.
      onLoading: () => transport.seek(Number(PM.time) || 0, false),
      onLoaded: () => {
        if (session.headless) session.sendHeadless(openComposition());
        PM.bus?.emit?.('layout');
      },
      onRoute: (route, reason) => {
        if (route === 'legacy') log('warn', `engine composition unavailable: ${reason ?? ''}`);
        transport.refresh();
        if (route === 'engine') audio.sync();
        PM.bus?.emit?.('draw');
      }
    });
    sessions.add(session);
    current = session;
    transport.resetLoop();
    session.sendHeadless(openComposition());
    sessionReady = session.start().then(() => renderTransport.syncProducers(session)).catch((error: unknown) => {
      log('warn', `engine session: ${error instanceof Error ? error.message : String(error)}`);
      session.close();
      sessions.delete(session);
      if (current === session) { current = null; sessionReady = null; }
    });
    return session;
  };
  const wakeStandby = (): Promise<void> => {
    openSession();
    return sessionReady ?? Promise.resolve();
  };
  let lastProducerFailure = -Infinity;
  const documentEncoder = new TextEncoder();
  const renderTransport = new EngineRenderTransport({
    native,
    session: snapshotSession,
    sessions: () => [...sessions],
    revision: () => {
      const barrier = PM.EngineDoc?.barrier;
      if (!barrier) return null;
      barrier.flush();
      // A snapshot may block this thread until the engine has the revision:
      // the batches this run holds (patch-coalesce.ts) leave now.
      connection?.sendPatches();
      noteVersion();
      return barrier.revision;
    },
    composition: () => String(PM.proj?.compId ?? PM.proj?.id ?? ''),
    document: () => documentId(PM.proj?.id),
    viewSeq: () => current?.viewSeq ?? null,
    fallbackRunner: () => (fallbackRunner ??= new ShaderFallbackRunner()),
    wake: wakeStandby,
    // A sync snapshot the window's session cannot serve carries the
    // document to main's session for the window (main/engine/window-sessions.ts).
    windowDocument: () => {
      const doc = PM.EngineDoc;
      const barrier = doc?.barrier;
      if (!doc?.enabled || !barrier || typeof doc.snapshot !== 'function') return null;
      barrier.flush();
      connection?.sendPatches();
      return {
        id: documentId(PM.proj?.id),
        revision: barrier.revision,
        full: () => {
          const snapshot = doc.snapshot();
          if (!snapshot) throw new Error('the document mirror has no snapshot');
          return { json: documentEncoder.encode(snapshot.json), hash: snapshot.hash, revision: snapshot.revision };
        }
      };
    },
    usesBlender: () => usesBlender(PM.proj),
    // A `raster` override that throws fails the frame, reported as today's
    // frame loop reports a compositor throw (legacy/core/engine.ts).
    playing: () => !!PM.playing,
    frameTicks: () => {
      const rate = resolveFrameRate(PM.proj?.fps, PM.proj?.fpsRational);
      const ticks = rate ? (TICKS_PER_SECOND * rate.den) / rate.num : 0;
      return Number.isInteger(ticks) && ticks > 0 ? ticks : null;
    },
    onProducerError: (error) => {
      const now = performance.now();
      if (now - lastProducerFailure <= 2000) return;
      lastProducerFailure = now;
      console.error('[engine] frame render failed', error);
      PM.toast?.('The preview could not be rendered. See the console for details.');
    },
    svgRasters: () => svgRasters,
    log
  });
  (PM as any).EngineRender = renderTransport;
  // The job engine's export and agent frames ask this window for the SVG
  // rasters of the document it has open (main's `compatFrame`).
  const unprovideSvg = renderTransport.provideCompat('svgImage', (request) => svgRasters.produce(request));
  (PM as any).EngineSvgRasters = svgRasters;
  // The app's own still captures (the Library's section thumbnails, the
  // agent's look and observe) draw through the engine too: with the new
  // engine on, nothing asks the current engine for pixels. The exporter's
  // methods ask `PM.EngineCaptures` (it installs after this module).
  const unrouteCaptures = installAppCaptures(PM, renderTransport);
  // The kernel's raster override producer answers through it (kernel/raster-compat.ts).
  const detachRasterCompat = (PM as any).RasterCompat?.attach?.(renderTransport) ?? (() => {});
  // Playback: keep compatibility answers ahead of the clock (render-transport.ts).
  const offPresented = PM.bus?.on?.('preview:presented', () => renderTransport.presentedWhilePlaying());
  let wasPlaying = !!PM.playing;
  const offTransport = PM.bus?.on?.('transport', () => {
    const playing = !!PM.playing;
    if (playing && !wasPlaying) renderTransport.playbackStarted();
    wasPlaying = playing;
  });

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
    transport.refresh();

  }

  const onMessage = (event: MessageEvent): void => {
    if (event.source !== target) return;
    const data = event.data as { type?: string; process?: string; generation?: number; report?: unknown } | null;
    if (data?.type === ENGINE_WINDOW_MESSAGE.port) {
      if (data.process !== 'interactive' || !event.ports[0]) return;
      void connect(Number(data.generation) || 0, event.ports[0] as unknown as PortLike);
    } else if (data?.type === ENGINE_WINDOW_MESSAGE.crash) {
      log('warn', 'an engine crashed and is restarting; the native session will reconnect');
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
    exclusive: true,
    // Frame events on presentation (presentation-events.ts), at most once
    // per animation frame for the newest frame on screen, as today's frame
    // loop emits them once per frame it draws.
    presented: (frame, quality) => presentedEvents.push(frame, quality),
    composition: () => String(PM.proj?.compId ?? PM.proj?.id ?? ''),
    hostOffsetUs: () => hostClock.offsetUs(),
    revision: () => {
      const barrier = PM.EngineDoc?.barrier;
      if (!barrier) return null;
      barrier.flush();
      noteVersion();
      return barrier.revision;
    },
    onReady(listener) {
      ready.add(listener);
      return () => ready.delete(listener);
    },
    open(handlers: EngineViewerHandlers): EngineViewerChannel | null {
      const c = connection;
      if (!c?.ready || c.closed || !PM.EngineDoc?.enabled) return null;
      const opened = openSession();
      if (!opened) return null;
      const lease = ++viewerLease;
      const previous = viewerHandlers;
      viewerHandlers = handlers;
      previous?.closed('another viewer attached to the engine session');
      // A viewer that replaces another on a session already presenting
      // (no viewerless state in between, which would make the engine say
      // it again) learns why a composition the engine cannot draw shows
      // nothing. Its channel is in place by the next microtask.
      const missed = missedRoute(opened);
      if (missed) {
        queueMicrotask(() => {
          if (lease === viewerLease && viewerHandlers === handlers && opened.route === 'legacy') {
            handlers.message(missed, performance.now());
          }
        });
      }
      return {
        sendViewerState: (state) => {
          if (lease === viewerLease) opened.sendViewerState({ ...state, headless: false } as never);
        },
        release: (release) => opened.release(release),
        close: () => {
          if (lease !== viewerLease || opened.closed) return;
          viewerHandlers = null;
          opened.sendHeadless(openComposition());
          transport.refresh();
        }
      };
    }
  });
  let service = makeService();
  let registration = PM.Kernel.services.register(ENGINE_VIEWER_SERVICE, service);
  target.dispatchEvent(new Event(ENGINE_VIEWER_CHANGED));
  // `render.gl.init` / `resize` bind engine presentation to the caller's
  // canvas (canvas-binding.ts; also `PM.EngineCanvas`).
  const canvasBinding = new EngineCanvasBinding({
    service: () => service,
    composition: () => (PM.proj ? { w: Number(PM.proj.w) || 0, h: Number(PM.proj.h) || 0 } : null),
    quality: () => Number(PM.quality ?? 1),
    present: (options) => new ViewerPresentation({ ...options, log: (level, message) => log(level, message) })
  });
  (PM as any).EngineCanvas = canvasBinding;

  const handle: EngineConnectionHandle = {
    hostClock,
    transport,
    audio,
    render: renderTransport,
    session: () => current,
    setTrace: (on) => {
      if (on === trace) return;
      trace = on;
      registration.dispose();
      service = makeService();
      registration = PM.Kernel.services.register(ENGINE_VIEWER_SERVICE, service);
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
      offEager();
      offAssets();
      follower.dispose();
      unprovideSvg();
      unrouteAudio();
      target.removeEventListener('message', onMessage);
      PM.setTransportDriver?.(null);
      registration.dispose();
      target.dispatchEvent(new Event(ENGINE_VIEWER_CHANGED));
      for (const s of [...sessions]) s.close();
      sessions.clear();
      current = null;
      viewerHandlers = null;
      sessionReady = null;
      unrouteCaptures();
      detachRasterCompat();
      if (typeof offPresented === 'function') offPresented();
      if (typeof offTransport === 'function') offTransport();
      renderTransport.dispose();
      if ((PM as any).EngineRender === renderTransport) (PM as any).EngineRender = null;
      if ((PM as any).EngineStats === stats) (PM as any).EngineStats = null;
      canvasBinding.unbind();
      if ((PM as any).EngineCanvas === canvasBinding) (PM as any).EngineCanvas = null;
      connection?.close('the engine connection was removed');
      connection = null;
      hostClock.stop();
    }
  };
  PM.EngineConnection = handle;
  return handle;
}
