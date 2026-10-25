import { installHiddenCpu } from '../host/hidden-cpu';
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
import { RemotePmWasm } from '../engine-doc/remote';
import type { FrameBridgeApi } from '../../../shared/frame-bridge';
import type { EngineBridge } from '../../../shared/engine-ipc';
import { ENGINE_WINDOW_MESSAGE } from '../../../shared/engine-ipc';
import type { EngineViewerChannel, EngineViewerHandlers, EngineViewerService } from '../../../extensions/viewer/engine-present/presentation';
import type { ClockSample, FrameDescriptor } from '../../../shared/engine-protocol';
import { EngineAudio, routeAudioPanels } from './audio';
import { EngineConnection, type Log, type PortLike } from './connection';
import { PortReconnector } from './reconnect';
import { HostClock } from './host-clock';
import { DocumentSession, documentId, type DocumentStream, type Route } from './session';
import { EngineRenderTransport, EngineSnapshotError } from './render-transport';
import { EngineSourceMedia, SOURCE_SERVICE } from './source-media';
import { EngineStatsView, PresentedEvents, RevisionVersions, emitPresented } from './presentation-events';
import { EngineCanvasBinding } from './canvas-binding';
import { resolveFrameRate, TICKS_PER_SECOND } from '../../../shared/frame-rate';
import { installAppCaptures } from './app-captures';
import { viewportEngineCamera } from '../core/scene3d/viewport';
// The presentation is platform code hosted in the viewer's folder; the
// kernel's `render.gl.init` binding uses it for viewers that do not present
// by themselves (old forks of the viewer).
import { ViewerPresentation } from '../../../extensions/viewer/engine-present/presentation';
import { PresentationWarmup } from '../../../extensions/viewer/engine-present/prewarm';
import { ShaderFallbackRunner, type ShaderFallbackRequest } from './shader-fallback';
import { SvgRasters } from './svg-rasters';
import { EngineTransport, ticks } from './transport';
import { EnginePreview, documentWake } from './preview';
import { EngineSceneMetadata } from './scene-metadata';
import { engineCanvasViewParams, watchEngineCanvasView } from './canvas-view';
import { followOpenComposition } from './composition-follow';

/** Mirrors engine-present/presentation.ts (the renderer does not import extension values). */
export const ENGINE_VIEWER_SERVICE = 'engine.viewer';
export const ENGINE_VIEWER_CHANGED = 'powermove:engine-viewer-changed';

export type EngineConnectionHost = {
  nativeHidden?: boolean;
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
  /** Experimental renderer memory gate. Only the actual final-tab
   * transition can pause native presentation; measurements can disable it. */
  setHomeSuspendForDiagnostics(on: boolean): void;
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
  preview: EnginePreview;
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
  if (host?.nativeHidden) { installHiddenCpu(PM); return null; }
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
  // Capture a compatibility context's engine surfaces before presentation
  // returns their pool slots. An import can arrive after its descriptor.
  const pendingCompatFrames = new Map<string, FrameDescriptor>();
  const captureCompatFrame = (desc: FrameDescriptor): boolean => {
    if (!PM.CompatContext?.capturesFrames) return true;
    const frame = source.videoFrame(desc.poolGeneration, desc.poolIndex);
    if (!frame) return false;
    try { PM.CompatContext.captureFrame(desc, frame); } finally { frame.close(); }
    return true;
  };
  const offCompatSurface = source.onSurface((generation, poolIndex) => {
    for (const [key, desc] of pendingCompatFrames) {
      if (desc.poolGeneration === generation && desc.poolIndex === poolIndex && captureCompatFrame(desc)) pendingCompatFrames.delete(key);
    }
  });
  const presentationWorker = (): Worker => new Worker(new URL('../../../extensions/viewer/engine-present/present-worker-entry.ts', import.meta.url), { type: 'module', name: 'engine-present' });
  const drawingWarmup = new PresentationWarmup(target.document, presentationWorker);
  drawingWarmup.warm();
  const hostClock = new HostClock(() => native.hostTime());
  hostClock.start();
  let connection: EngineConnection | null = null;
  let generation = 0;
  let starting: { connection: EngineConnection; done: Promise<void>; finish(): void } | null = null;
  let connections = 0;
  const ready = new Set<() => void>();
  const sessions = new Set<DocumentSession>();
  let current: DocumentSession | null = null;
  let fallbackRunner: ShaderFallbackRunner | null = null;
  // The job engine's fallback requests (exports, agent stills) run in this
  // window's runner too: one WebGL2 context for the window (decision 19).
  const offShaderFallback = native.onShaderFallback?.(async ({ request, textures }) => {
    const { rgba16f, rgba32f } = await (fallbackRunner ??= new ShaderFallbackRunner()).render(request as ShaderFallbackRequest, textures);
    return { rgba16f, ...(rgba32f ? { rgba32f } : {}) };
  }) ?? (() => {});
  // One session owns this window's clock. A viewer attaches to it; closing
  // that channel retires presentation only, leaving pm-audio and time alive.
  let viewerHandlers: EngineViewerHandlers | null = null;
  let viewerLease = 0;
  let sessionReady: Promise<void> | null = null;
  let sourceMedia: EngineSourceMedia | null = null;
  /** The open composition (what Play plays and the viewer shows). */
  const openComposition = (): string => String(PM.proj?.compId ?? PM.proj?.id ?? '');
  /** A viewerless session follows the open composition (composition-follow.ts). */
  const follower = followOpenComposition({ session: () => current, open: openComposition, bus: PM.bus });
  const transport = new EngineTransport(() => current, hostClock, {
    exclusive: true, wake: () => queueMicrotask(() => { void wakeStandby(); }),
    prepare: follower.follow
  });
  const preview = new EnginePreview(
    () => current, openComposition, () => Number(PM.time) || 0,
    () => {
      const barrier = PM.EngineDoc?.barrier;
      barrier?.flush();
      connection?.sendPatches();
      return barrier?.revision ?? current?.applied ?? 0;
    },
    // Main retired the Eevee preview setting; every preview uses the shot's rendered mode.
    () => 'rendered',
    // `wakeStandby` is declared below; read it when the preview wakes.
    documentWake(() => wakeStandby(), () => current)
  );
  /** animVersion per revision sent, for `frame:rendered.version`. */
  const versions = new RevisionVersions();
  const sceneMetadata = new EngineSceneMetadata(PM);
  const noteVersion = (): void => {
    const barrier = PM.EngineDoc?.barrier;
    if (barrier) versions.note(barrier.revision, PM.animVersion?.());
  };
  const presentedEvents = new PresentedEvents(
    (frame, quality) => {
      const doc = PM.EngineDoc;
      doc?.barrier?.flush();
      if (doc?.enabled && frame.revision === doc.barrier?.revision) {
        try { doc.api?.syncScene3dFrame(frame.pts / TICKS_PER_SECOND); }
        catch (error) { log('error', `native 3D scene synchronization: ${error instanceof Error ? error.message : String(error)}`); }
      }
      PM.CompatContext?.selectFrame?.(frame);
      renderTransport.trace.record('frame event', frame.revision, frame.pts);
      emitPresented(PM.bus, frame, quality, { viewport: () => PM.GL?.previewViewport ?? null, versions, project: () => PM.proj });
    },
    (fn) => target.requestAnimationFrame(fn)
  );
  /** Engine stats for the status bar and the performance panel. */
  const stats = new EngineStatsView();
  (PM as any).EngineStats = stats;
  (PM as any).EnginePresentedEvents = presentedEvents;
  /** A paused redraw request with nothing new for the engine to present:
   *  today's frame loop announces the frame it draws again
   *  (`PresentedEvents.redraw`). Called by the frame loop on `draw`. */
  (PM as any).EngineRedraw = (): boolean => {
    const doc = PM.EngineDoc;
    doc?.barrier?.flush();
    const fps = Number((PM.curComp?.() ?? PM.proj)?.fps) || 30;
    return presentedEvents.redraw(
      { revision: doc?.barrier?.revision ?? current?.applied ?? null, viewSeq: current?.viewSeq ?? null, pts: Number.isFinite(PM.time) ? ticks(PM.time) : null, frameTicks: ticks(1) / fps },
      (frame, quality) => {
        // Today's redraw calls the raster wrappers again: an output that
        // changed is the engine's to draw and present (once, as today).
        if (renderTransport.refreshShown(frame.pts)) return false;
        PM.CompatContext?.selectFrame?.(frame);
        PM.bus?.emit?.('preview:presented', { viewport: PM.GL?.previewViewport ?? null, time: frame.pts / TICKS_PER_SECOND, version: versions.at(frame.revision), project: PM.proj, quality });
        return true;
      }
    );
  };
  PM.setTransportDriver?.(transport);

  // The engine's levels, monitor solo, and waveform peaks for today's audio
  // panels (the mixer's meters and solo, `asset.peaks`).
  const audio = new EngineAudio({
    session: () => current,
    driving: () => transport.active(),
    assets: () => (PM.assets?.map?.values?.() ?? []) as Iterable<Record<string, any>>,
    source: (id) => {
      const meta = PM.proj?.assets?.[id];
      return [meta?.contentHash, meta?.storageKey, meta?.link?.hash, meta?.link?.path];
    },
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
    sourceMedia?.assetsChanged();
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
      const barrier = PM.EngineDoc?.barrier;
      flushing = true;
      const began = performance.now();
      try {
        const before = barrier?.revision ?? null;
        // A paused write without paths: the layers it most likely wrote
        // leave first, as a provisional revision the engine draws but does
        // not present while the rest of the document is compared. That
        // comparison confirms the revision or supersedes it; it runs before
        // anything else here, so every answer (and every query a raster
        // override makes) sees the whole document. During playback the
        // comparison would follow at once anyway: no provisional batch.
        if (!PM.playing && barrier?.flushProvisional()) {
          noteVersion();
          connection?.sendPatches();
          renderTransport.trace.record('provisional sent', barrier.revision, ticks(PM.time ?? 0), began);
        }
        try {
          barrier?.flush();
        } catch (error) {
          // A comparison that fails must still end a provisional revision:
          // a resync supersedes it (and releases or drops its frame).
          log('warn', `flushing the document for the engine failed: ${error instanceof Error ? error.message : String(error)}`);
          if (barrier?.provisionalRevision != null) {
            barrier.markResync();
            barrier.flush();
          }
        }
        noteVersion();
        renderTransport.trace.record('edit flushed', barrier?.revision ?? 0, ticks(PM.time ?? 0), began);
        // An edit while paused: the producers prepare the frame on screen at
        // the new revision now, beside the engine's render that will ask
        // for it (today draws an edit in the frame it happens).
        if (before !== null && (barrier?.revision ?? null) !== before) renderTransport.prepareShown();
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
    if (starting) return null;
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
          if (message.type === 'frame' && !captureCompatFrame(message.frame)) {
            const desc = message.frame;
            pendingCompatFrames.set(`${desc.poolGeneration}:${desc.seq}`, desc);
            while (pendingCompatFrames.size > 8) pendingCompatFrames.delete(pendingCompatFrames.keys().next().value!);
          }
          if (viewerHandlers) viewerHandlers.message(message as never, arrivedMs);
          else if (message.type === 'frame') session.release(message.frame);
        },
        closed: (reason) => {
          preview.close(session);
          sessions.delete(session);
          if (current === session) {
            stats.reset();
            PM.invalidate?.('status');
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
      onAudio: (message) => { audio.receive(message, session); sourceMedia?.receive(message); },
      onPreviewStatus: (status) => preview.receiveStatus(session, status),
      onStats: (message) => {
        stats.receive(message, performance.now());
        PM.invalidate?.('status');
      },
      // Replace a seek queued on the home screen before any old clock can
      // overwrite the replacement document's requested UI position.
      onLoading: () => { sourceMedia?.reset(); transport.seek(Number(PM.time) || 0, false); },
      onOpened: () => renderTransport.syncProducers(session),
      onLoaded: () => {
        stats.reset();
        PM.invalidate?.('status');
        audio.resynced();
        if (session.headless) session.sendHeadless(openComposition());
        PM.bus?.emit?.('layout');
      },
      onRoute: (route, reason) => {
        if (route === 'legacy') log('warn', `engine composition unavailable: ${reason ?? ''}`);
        // Compile errors follow whoever draws (kernel/engine-definitions.ts).
        (PM as any).EngineDefinitions?.meta?.setRoute?.(route);
        transport.refresh();
        if (route === 'engine') audio.sync();
        PM.bus?.emit?.('draw');
      }
    });
    sessions.add(session);
    current = session;
    transport.resetLoop();
    session.sendHeadless(openComposition());
    sessionReady = session.start().catch((error: unknown) => {
      log('warn', `engine session: ${error instanceof Error ? error.message : String(error)}`);
      session.close();
      sessions.delete(session);
      if (current === session) { current = null; sessionReady = null; }
    });
    return session;
  };
  const wakeStandby = async (): Promise<void> => {
    while (starting) await starting.done;
    openSession();
    await sessionReady;
  };
  let lastProducerFailure = -Infinity;
  const documentEncoder = new TextEncoder();
  // The window's revision once every pending edit is flushed to the engine.
  const flushedRevision = (): number | null => {
    const barrier = PM.EngineDoc?.barrier;
    if (!barrier) return null;
    barrier.flush();
    // A snapshot may block this thread until the engine has the revision:
    // the batches this run holds (patch-coalesce.ts) leave now.
    connection?.sendPatches();
    noteVersion();
    return barrier.revision;
  };
  const renderTransport = new EngineRenderTransport({
    native,
    session: snapshotSession,
    sessions: () => [...sessions],
    revision: flushedRevision,
    composition: () => String(PM.proj?.compId ?? PM.proj?.id ?? ''),
    document: () => documentId(PM.proj?.id),
    viewSeq: () => current?.viewSeq ?? null,
    fallbackRunner: () => (fallbackRunner ??= new ShaderFallbackRunner()),
    wake: wakeStandby,
    // A sync snapshot the window's session cannot serve carries the
    // document to main's session for the window (main/engine/window-sessions.ts).
    windowDocument: () => {
      const doc = PM.EngineDoc;
      const nativeDocument = doc?.ready && doc.wasm instanceof RemotePmWasm;
      if (!nativeDocument && (starting || !connection?.ready || connection.closed)) return null;
      const barrier = doc?.barrier;
      if (!doc?.enabled || !barrier || typeof doc.snapshot !== 'function') return null;
      barrier.flush();
      connection?.sendPatches();
      return {
        id: documentId(PM.proj?.id),
        revision: barrier.revision,
        ...(doc.wasm instanceof RemotePmWasm ? { nativeKernel: true as const } : {}),
        full: () => {
          if (doc.wasm instanceof RemotePmWasm) throw new Error('Native snapshots use the owner-scoped document reference');
          const snapshot = doc.snapshot();
          if (!snapshot) throw new Error('the document mirror has no snapshot');
          return { json: snapshot.bytes ?? documentEncoder.encode(snapshot.json), hash: snapshot.hash, revision: snapshot.revision };
        }
      };
    },
    // A `raster` override that throws fails the frame, reported as today's
    // frame loop reports a compositor throw (legacy/core/engine.ts).
    playing: () => !!PM.playing,
    frameTicks: () => {
      const rate = resolveFrameRate(PM.proj?.fps, PM.proj?.fpsRational);
      const ticks = rate ? (TICKS_PER_SECOND * rate.den) / rate.num : 0;
      return Number.isInteger(ticks) && ticks > 0 ? ticks : null;
    },
    playhead: () => {
      const position = transport.now(performance.now());
      const rate = current?.clock?.rate;
      return position ? { time: ticks(position.time), rate: rate ? rate.num / rate.den : 1 } : null;
    },
    span: () => {
      const loop = current?.clock?.loopRange;
      // The engine's clock wraps by the loop it was sent (today's looping
      // playback: the work area while `PM.loop` is on).
      if (loop) return { start: Number(loop.start), end: Number(loop.end), loop: true };
      const duration = Number((PM.curComp?.() ?? PM.proj)?.dur);
      return duration > 0 ? { start: 0, end: ticks(duration) } : null;
    },
    // A snapshot's own failure (an extension's request): recorded, never on the stage.
    onFailure: (code, message) => current?.reportFailure(code, message, undefined, false),
    onProducerError: (error) => {
      current?.reportFailure(error instanceof EngineSnapshotError ? error.code : 'internal', error instanceof Error ? error.message : String(error));
      const now = performance.now();
      if (now - lastProducerFailure <= 2000) return;
      lastProducerFailure = now;
      console.error('[engine] frame render failed', error);
      PM.toast?.('The preview could not be rendered. See the console for details.');
    },
    captionsHidden: () => PM.captionsHidden === true,
    svgRasters: () => svgRasters,
    log
  });
  (PM as any).EngineRender = renderTransport;
  sourceMedia = new EngineSourceMedia({
    render: renderTransport, native,
    project: () => PM.proj,
    revision: () => renderTransport.currentRevision() ?? 0,
    session: snapshotSession,
    wake: wakeStandby,
    bind: async () => { await (PM as any).HostMedia?.flushBindings?.(PM.proj); },
    changed: () => PM.bus?.emit?.('assets'),
    error: error => log('error', `source media: ${error instanceof Error ? error.message : String(error)}`)
  });
  (PM as any).EngineSourceMedia = sourceMedia;
  const sourceRegistration = PM.Kernel.services.register(SOURCE_SERVICE, sourceMedia);
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
  // And the compatibility context draws over WebM export frames (kernel/compat-context.ts).
  const detachContextExports = (PM as any).CompatContextExports?.attach?.(renderTransport) ?? (() => {});
  // Playback: keep compatibility answers ahead of the clock (render-transport.ts).
  const offPresented = PM.bus?.on?.('preview:presented', () => renderTransport.presentedWhilePlaying());
  let wasPlaying = !!PM.playing;
  const offTransport = PM.bus?.on?.('transport', () => {
    const playing = !!PM.playing;
    if (playing && !wasPlaying) renderTransport.playbackStarted();
    wasPlaying = playing;
  });

  // A connection that closes while its launch lives gets no new port from
  // main (reconnect.ts): ask for one, with the supervisor's backoff, so the
  // window's sessions reopen and resync.
  const reconnector = new PortReconnector(() => {
    log('info', 'asking main for a new interactive engine port');
    native.requestPorts?.();
  }, () => !!connection && !connection.closed);

  async function connect(nextGeneration: number, port: PortLike): Promise<void> {
    pendingCompatFrames.clear();
    // The old connection is replaced, not lost: its close asks for nothing.
    const replaced = connection;
    starting?.finish();
    starting = null;
    connection = null;
    replaced?.close('a new interactive engine launch');
    reconnector.portArrived();
    const next = new EngineConnection(nextGeneration, port, log);
    connection = next;
    let finish!: () => void;
    const startup = { connection: next, done: new Promise<void>(resolve => { finish = resolve; }), finish: () => finish() };
    starting = startup;
    generation = nextGeneration;
    next.onClose((reason) => {
      const current = connection === next;
      if (current) {
        connection = null;
        if (starting === startup) { starting = null; startup.finish(); }
      }
      log(current ? 'warn' : 'info', `the interactive engine connection closed (${reason})`);
      transport.refresh();
      if (current) reconnector.closed();
    });
    try {
      await next.open();
      if (connection !== next) return;
      // A fresh utility generation has lost its evaluator tree. Restore it
      // before any ready listener, viewer, or transport can open a session.
      const doc = PM.EngineDoc as typeof PM.EngineDoc & { reconnect?(generation: number): Promise<void> };
      await doc?.reconnect?.(nextGeneration);
    } catch (error) {
      if (connection === next) {
        log('warn', `the interactive engine did not connect: ${error instanceof Error ? error.message : String(error)}`);
        next.close('document recovery or handshake failed');
      }
      return;
    } finally {
      startup.finish();
    }
    if (connection !== next) return;
    if (starting === startup) starting = null;
    connections++;
    reconnector.opened();
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
  let homeSuspendEnabled = true;
  let visitedRealProject = false;
  let mayPresent = true;
  const reconcileHomePresentation = (): void => {
    // The home placeholder is present at initial startup too; keep that
    // eager path. A confirmed project tab must first have been on screen.
    if (PM.Tabs?.active) visitedRealProject = true;
    const closedLastTab = visitedRealProject && PM.isHomeProject?.() === true && PM.Tabs?.list?.().length === 0;
    const next = !(homeSuspendEnabled && closedLastTab);
    if (next === mayPresent) return;
    mayPresent = next;
    target.dispatchEvent(new Event(ENGINE_VIEWER_CHANGED));
  };
  const offHomeProject = PM.bus?.on?.('project', reconcileHomePresentation) ?? (() => {});
  const offHomeTabs = PM.bus?.on?.('projects:open', reconcileHomePresentation) ?? (() => {});
  // A restored project may precede connection installation; remember it so
  // its later final close still qualifies without changing startup home.
  reconcileHomePresentation();
  const makeService = (): EngineViewerService & { trace?: boolean } => ({
    trace,
    mayPresent: () => mayPresent,
    importStats: () => {
      const { launch, held, received, released, videoFrames } = source.stats();
      return { launch, held, received, released, videoFrames };
    },
    source,
    exclusive: true,
    drawingWarmup,
    // Frame events on presentation (presentation-events.ts), at most once
    // per animation frame for the newest frame on screen, as today's frame
    // loop emits them once per frame it draws.
    presented: (frame, quality) => {
      renderTransport.trace.record('viewer submitted', frame.revision, frame.pts);
      sceneMetadata.apply(frame, current);
      if (current) preview.receiveFrame(current, frame);
      presentedEvents.push(frame, quality);
    },
    composition: () => String(PM.proj?.compId ?? PM.proj?.id ?? ''),
    hostOffsetUs: () => hostClock.offsetUs(),
    editorCamera: () => viewportEngineCamera(PM),
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
      let retirementLaunch: number | null = null;
      let authorizedRetirement = false;
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
        close: (options) => {
          if (lease !== viewerLease || viewerHandlers !== handlers || opened.closed) return;
          // `sendViewerState` can merely queue headless while the document
          // has no session id or its first resync is pending. A new viewer
          // could overwrite that queued state and keep using this pool, so
          // only a posted headless state grants import-retirement ownership.
          const postedHeadless = opened.id !== null && !(opened.resyncPending && opened.applied === 0) && c.ready && !c.closed;
          const launch = postedHeadless ? source.stats().launch : null;
          viewerHandlers = null;
          opened.sendHeadless(openComposition());
          retirementLaunch = launch;
          authorizedRetirement = launch !== null && source.stats().launch === launch && c.ready && !c.closed;
          transport.refresh();
          // A viewer that goes away (extension reload or update, service
          // replacement, a fork binding its own canvas) leaves a prepared
          // drawing for the next one, as at startup. The final-tab
          // suspension keeps its own drawing: a second one would only hold
          // memory on the home screen.
          if (!options?.keepsDrawing) drawingWarmup.warm();
        },
        retireClosedPool: (generation) => {
          // A replacement viewer may claim the same pool before an old GPU
          // fence drains. A new launch also reuses generation numbers after
          // preload has cleared its old imports. Both revoke this channel's
          // right to retire the generation.
          if (!authorizedRetirement || lease !== viewerLease || viewerHandlers !== null || source.stats().launch !== retirementLaunch) return 0;
          return source.retire(generation);
        },
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
    view: () => engineCanvasViewParams(PM),
    // A bound viewer (a fork or override of the built-in one from before the
    // engine) presents exactly as the built-in viewer does: in a
    // presentation worker. Its canvas is then a placeholder the page reads
    // back like the built-in one (the Library card copies its pixels), and
    // playback keeps its cadence while the main thread is busy.
    present: (options) => new ViewerPresentation({
      ...options,
      log: (level, message) => log(level, message),
      createWorker: options.service.offMainThread === false ? undefined : presentationWorker
    })
  });
  (PM as any).EngineCanvas = canvasBinding;
  const offCanvasViewport = watchEngineCanvasView(PM, () => canvasBinding.refresh());

  const handle: EngineConnectionHandle = {
    hostClock,
    transport,
    preview,
    audio,
    render: renderTransport,
    session: () => current,
    setHomeSuspendForDiagnostics: (on) => {
      homeSuspendEnabled = on === true;
      reconcileHomePresentation();
    },
    setTrace: (on) => {
      renderTransport.trace.enabled = on;
      const barrier = PM.EngineDoc?.barrier;
      if (barrier) barrier.trace = on ? (stage: string, began: number) => renderTransport.trace.record(stage, barrier.revision, ticks(PM.time ?? 0), began) : null;
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
      connected: !starting && !!connection?.ready && !connection.closed,
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
      reconnector.dispose();
      offCompatSurface();
      pendingCompatFrames.clear();
      preview.dispose();
      drawingWarmup.dispose();
      offShaderFallback();
      offPending();
      offEager();
      offHomeProject();
      offHomeTabs();
      offAssets();
      follower.dispose();
      unprovideSvg();
      unrouteAudio();
      target.removeEventListener('message', onMessage);
      PM.setTransportDriver?.(null);
      delete (PM as any).EngineRedraw;
      registration.dispose();
      target.dispatchEvent(new Event(ENGINE_VIEWER_CHANGED));
      for (const s of [...sessions]) s.close();
      sessions.clear();
      current = null;
      viewerHandlers = null;
      sessionReady = null;
      unrouteCaptures();
      sourceRegistration.dispose();
      sourceMedia?.dispose();
      if ((PM as any).EngineSourceMedia === sourceMedia) (PM as any).EngineSourceMedia = null;
      detachRasterCompat();
      detachContextExports();
      if (typeof offPresented === 'function') offPresented();
      if (typeof offTransport === 'function') offTransport();
      renderTransport.dispose();
      if ((PM as any).EngineRender === renderTransport) (PM as any).EngineRender = null;
      if ((PM as any).EngineStats === stats) (PM as any).EngineStats = null;
      canvasBinding.unbind();
      offCanvasViewport();
      if ((PM as any).EngineCanvas === canvasBinding) (PM as any).EngineCanvas = null;
      connection?.close('the engine connection was removed');
      connection = null;
      starting?.finish();
      starting = null;
      hostClock.stop();
    }
  };
  PM.EngineConnection = handle;
  return handle;
}
