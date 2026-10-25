/** Authoritative fixture timing, collected with ffprobe before app measurement. */
export type SourceFrameTimeline = { pts: number[]; quantum: number };
/** Exact half-open presentation interval; never advance into a future frame. */
export function sourceFrameIndex(timeline: SourceFrameTimeline, time: number): number {
  let index = -1;
  for (let i = 0; i < timeline.pts.length && timeline.pts[i]! <= time; i++) index = i;
  return index;
}

/** Keep nominal targets unless they touch a route's interval boundary. The
 * same frozen targets serve both routes. One original timebase tick of margin
 * avoids boundary rounding in native decoding; this is a target adjustment,
 * not an epsilon in frame selection or a relaxed presentation tolerance. */
export function commonSourceTargets(original: SourceFrameTimeline, actual: SourceFrameTimeline, nominal: number[]) {
  sourceTimelineCorrespondence(original, actual, []);
  const tickRate = 705_600_000;
  const margin = Math.max(original.quantum, 1 / tickRate);
  return nominal.map(time => {
    const frame = sourceFrameIndex(original, time);
    if (frame < 0 || frame + 1 >= original.pts.length)
      throw new Error(`No bounded source interval for target ${time}`);
    const lower = Math.max(original.pts[frame]!, actual.pts[frame]!);
    const upper = Math.min(original.pts[frame + 1]!, actual.pts[frame + 1]!);
    const firstTick = Math.ceil((lower + margin) * tickRate);
    const lastTick = Math.floor((upper - margin) * tickRate);
    if (firstTick > lastTick) throw new Error(`No common interior for source frame ${frame}`);
    const target = Math.max(firstTick, Math.min(lastTick, Math.round(time * tickRate))) / tickRate;
    if (!(target > lower && target < upper)
        || sourceFrameIndex(original, target) !== frame || sourceFrameIndex(actual, target) !== frame)
      throw new Error(`Common source target crossed frame ${frame}`);
    return { nominal: time, target, deltaMs: (target - time) * 1000, frame, lower, upper, margin };
  });
}
export type SourceDisplayProbe = {
  ready(): Promise<void>;
  seek(time: number): Promise<number>;
  /** The acknowledged, presented delivery now on screen, or null. */
  shown(): { frame: number; timestamp: number } | null;
  evidence(): { events: Record<string, unknown>[]; dropped: number };
  dispose(): void;
};

export function sourceFrameTimeline(probe: {
  streams?: { time_base?: string; start_pts?: number }[];
  frames?: { best_effort_timestamp?: number }[];
}): SourceFrameTimeline {
  const stream = probe.streams?.[0];
  const [num, den] = (stream?.time_base ?? '').split('/').map(Number);
  const quantum = num! / den!;
  const raw = probe.frames?.map(frame => frame.best_effort_timestamp) ?? [];
  if (!(quantum > 0 && Number.isFinite(quantum)) || stream?.start_pts !== 0
      || raw.length < 2 || raw.some(pts => !Number.isSafeInteger(pts)))
    throw new Error('Source performance needs a zero-origin fixture with authoritative frame PTS.');
  // ffprobe -show_frames emits decoded presentation order. Do not sort away
  // a reordering/duplicate: the ordinal mapping is part of our evidence.
  const pts = (raw as number[]).map(value => value * quantum);
  // These fixtures have fine container timebases. Refuse ambiguous/coarse
  // timing instead of guessing frame identity from nominal FPS.
  if (pts[0] !== 0 || pts.slice(1).some((value, i) => value - pts[i]! < 4 * quantum))
    throw new Error('Source performance needs unique frame PTS and a fine timebase.');
  return { pts, quantum };
}

/** Temporal correspondence only; picture correctness belongs to the source
 * correctness campaign. A lossy rendition cannot use original pixel hashes.
 * Require every frame ordinal and its timestamp rounded to the ACTUAL output
 * timebase, plus equal selected ordinals at every measured target. This does
 * not widen the 1 µs rVFC match against either route's own indexed PTS. */
export function sourceTimelineCorrespondence(original: SourceFrameTimeline, actual: SourceFrameTimeline, targets: number[]) {
  if (original.pts.length !== actual.pts.length)
    throw new Error('Legacy source changed the fixture frame count; cross-route frame identity is unproven.');
  for (let i = 0; i < original.pts.length; i++) {
    if (Math.round(original.pts[i]! / actual.quantum) !== Math.round(actual.pts[i]! / actual.quantum))
      throw new Error(`Legacy source changed the fixture timestamp/order at frame ${i}; cross-route frame identity is unproven.`);
  }
  for (const time of targets) {
    if (sourceFrameIndex(original, time) !== sourceFrameIndex(actual, time))
      throw new Error(`Source time ${time} selects different original/rendition frame ordinals.`);
  }
  return { frames: actual.pts.length, checkedTargets: targets.length, mapping: 'same ordinals; original PTS quantized to actual timebase' };
}

/** Self-contained browser installer; page.evaluate serializes this function.
 * Observes production promises, never replaces their settlement or drain flow. */
export function installSourcePerformanceDisplay(): void {
  (window as any).__sourcePerformanceDisplay = (monitor: any, options: {
    native: boolean; asset: string; timeline: SourceFrameTimeline; trace: boolean; expectedSource?: string;
  }) => {
    const video: HTMLVideoElement = options.native ? monitor.surface : monitor;
    const presenter = options.native ? monitor.videoPresenter : null;
    if (typeof video.requestVideoFrameCallback !== 'function'
        || (options.native && typeof presenter?.presentedAt !== 'function'))
      throw new Error('Source performance requires observable frame presentation.');
    const trace: Record<string, unknown>[] = [];
    let dropped = 0, disposed = false, geometry = 0, callback = 0, latestTimestamp = -1;
    let pending: { time: number; frame: number; generation: number; result: boolean;
      began: number; resolve(): void; reject(error: Error): void } | null = null;
    type Delivery = { time: number; frame: number; generation: number; signal?: AbortSignal;
      timestamp?: number; acknowledged?: boolean; geometry: number; source: unknown };
    type Shown = { frame: number; timestamp: number; geometry: number; source: unknown };
    let painting: Delivery | null = null, confirmed: Shown | null = null;
    let openedSource: unknown;
    let displayNotBefore = -Infinity;
    let failure: Error | null = null;
    const deliveries = new Map<number, Delivery>();
    const observed = new Map<number, VideoFrameCallbackMetadata>();
    const source = () => options.native ? video.srcObject : video.currentSrc;
    const visible = () => video.isConnected && !document.hidden;
    const mark = (event: string, details: Record<string, unknown> = {}) => {
      if (!options.trace) return;
      if (trace.length < 512) trace.push({ event, at: performance.now(), ...details });
      else dropped++;
    };
    const trim = (map: Map<number, unknown>) => { while (map.size > 64) map.delete(map.keys().next().value!); };
    const selectedFrame = (time: number) => {
      let frame = -1;
      for (let i = 0; i < options.timeline.pts.length; i++) {
        if (options.timeline.pts[i]! > time) break;
        frame = i;
      }
      if (frame < 0) throw new Error(`No authoritative source frame for ${time}`);
      return frame;
    };
    const validShown = (shown: Shown | null): shown is Shown => !!shown && visible() && video.readyState >= 2
      && shown.geometry === geometry && shown.source === source();
    const fail = (error: Error) => { failure = error; mark('failure', { message: error.message }); pending?.reject(error); };
    const check = () => {
      if (!pending || !pending.result) return;
      if (failure) { pending.reject(failure); return; }
      if (!validShown(confirmed) || confirmed.frame !== pending.frame) return;
      // Native currently submits a delivery on every seek: prove that exact
      // delivery, including same-frame seeks. Legacy may keep its confirmed
      // frame on an intra-frame seek without producing another callback.
      if (options.native && deliveries.get(confirmed.timestamp)?.generation !== pending.generation) return;
      mark(options.native ? 'delivery-matched' : 'source-pts-matched', { time: pending.time, requestedFrame: pending.frame, timestamp: confirmed.timestamp,
        reused: options.native ? deliveries.get(confirmed.timestamp)?.generation !== pending.generation
          : !observed.has(confirmed.timestamp) || observed.get(confirmed.timestamp)!.presentationTime < pending.began });
      pending.resolve();
    };
    const acknowledge = (delivery: Delivery, timestamp: number) => {
      if (!delivery.acknowledged || !observed.has(timestamp) || delivery.signal?.aborted
          || timestamp !== latestTimestamp || delivery.geometry !== geometry || delivery.source !== source()) return;
      confirmed = { frame: delivery.frame, timestamp, geometry, source: source() };
      check();
    };
    const onFrame: VideoFrameRequestCallback = (_now, metadata) => {
      if (disposed) return;
      const timestamp = Math.round(metadata.mediaTime * 1e6);
      latestTimestamp = timestamp; confirmed = null;
      observed.set(timestamp, metadata); trim(observed);
      mark('rvfc', { timestamp, mediaTime: metadata.mediaTime, presentationTime: metadata.presentationTime,
        expectedDisplayTime: metadata.expectedDisplayTime, presentedFrames: metadata.presentedFrames });
      if (options.native) {
        const delivery = deliveries.get(timestamp);
        if (delivery) acknowledge(delivery, timestamp);
      } else if (metadata.presentationTime >= displayNotBefore) {
        // Match a PTS, not a currentTime value or an FPS-rounded frame number.
        const frame = options.timeline.pts.findIndex(pts => Math.abs(pts - metadata.mediaTime) <= 1e-6);
        if (frame < 0) fail(new Error(`Legacy displayed unindexed PTS ${metadata.mediaTime}; source identity is unproven.`));
        else { confirmed = { frame, timestamp, geometry, source: source() }; check(); }
      }
      callback = video.requestVideoFrameCallback(onFrame);
    };
    callback = video.requestVideoFrameCallback(onFrame);
    let previousSize = `${video.clientWidth}:${video.clientHeight}:${devicePixelRatio}`;
    const resize = new ResizeObserver(() => {
      const size = `${video.clientWidth}:${video.clientHeight}:${devicePixelRatio}`;
      if (size !== previousSize) { previousSize = size; geometry++; confirmed = null; }
    });
    resize.observe(video);
    const oldPaint = presenter?.paint, oldPresentedAt = presenter?.presentedAt;
    const media = options.native ? monitor.source : null;
    const oldFrame = media?.frame;
    if (options.trace && media) {
      media.frame = function(...args: any[]) {
        const began = performance.now(), generation = monitor.requested;
        const result = oldFrame.apply(this, args);
        if (args[0] === options.asset && args[4]?.color === 'compositor') {
          void result.then((snapshot: any) => mark('frame-result', { time: args[1], generation,
            elapsedMs: performance.now() - began, waitedUs: snapshot.waitedUs, renderUs: snapshot.renderUs,
            mainMs: snapshot.mainMs, width: snapshot.width, height: snapshot.height,
            format: snapshot.videoPlanes?.format, timestamp: snapshot.videoPlanes?.timestamp }), () => {});
        }
        return result;
      };
    }
    if (presenter) {
      presenter.paint = function(descriptor: unknown, signal?: AbortSignal) {
        // This ordinal labels the REQUEST, not the decoder output. The native
        // wall-clock stamp proves delivery only; original-media correctness
        // needs separate untimed evidence before accepting performance.
        const delivery: Delivery = { time: monitor.currentTime, frame: selectedFrame(monitor.currentTime),
          generation: monitor.requested, signal, geometry, source: source() };
        painting = delivery;
        const result = oldPaint.call(this, descriptor, signal);
        void result.then(() => { if (painting === delivery) painting = null; }, () => { if (painting === delivery) painting = null; });
        return result;
      };
      presenter.presentedAt = function(timestamp: number) {
        const delivery = painting;
        const result = oldPresentedAt.call(this, timestamp);
        if (!delivery) { fail(new Error('Native presentation has no associated source request.')); return result; }
        // Startup can assign srcObject inside paint(), after our entry hook.
        delivery.source = source(); delivery.timestamp = timestamp;
        deliveries.set(timestamp, delivery); trim(deliveries);
        mark('delivery', { time: delivery.time, requestedFrame: delivery.frame, generation: delivery.generation, timestamp });
        void Promise.resolve(result).then(() => {
          if (disposed || delivery.signal?.aborted) return;
          if (!visible() || this.presentedTimestamp !== timestamp) {
            fail(new Error(`Native delivery ${timestamp} was not presented (observed ${this.presentedTimestamp}); presentation bound is not an acknowledgement.`));
            return;
          }
          delivery.acknowledged = true;
          mark('current-stamp-match', { timestamp, generation: delivery.generation });
          acknowledge(delivery, timestamp);
        }, error => fail(new Error(String(error))));
        return result;
      };
    }
    const event = options.native ? 'loadeddata' : 'seeked';
    const onResult = () => {
      if (!pending || Math.abs(monitor.currentTime - pending.time) > 1e-6) return;
      if (options.native && monitor.requested !== pending.generation) return;
      pending.result = true;
      mark('result', { time: pending.time, generation: pending.generation, event });
      check();
    };
    monitor.addEventListener(event, onResult);
    const error = () => fail(new Error(monitor.error?.message ?? 'Source monitor failed'));
    monitor.addEventListener('error', error);
    return {
      async ready() {
        // A paused element may have displayed before observation began and
        // never issue another callback. Re-load that SAME source once during
        // setup, then require a real post-load rVFC. No seek/readback, synthetic
        // acknowledgement, or decode-ahead warm-up of measured seek targets.
        if (!options.native && !validShown(confirmed) && video.paused && video.readyState >= 2) {
          mark('setup-reload');
          confirmed = null; observed.clear(); latestTimestamp = -1;
          displayNotBefore = performance.now(); // Reject an already-queued callback from before load().
          video.load();
        }
        const began = performance.now();
        // This is setup, excluded from firstScrub. The timer also bounds a
        // hidden window whose RAF stops entirely.
        let timer: ReturnType<typeof setTimeout>;
        try {
          await Promise.race([
            (async () => {
              while (!validShown(confirmed) || (options.native && (monitor.rendering || monitor.paintAgain || presenter.previous))) {
                if (failure) throw failure;
                if (disposed || performance.now() - began > 15_000) throw new Error('Source display setup timed out');
                await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
              }
            })(),
            new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('Source display setup timed out')), 15_000); })
          ]);
        } finally { clearTimeout(timer!); }
        openedSource = source();
        if (options.expectedSource !== undefined && openedSource !== options.expectedSource)
          throw new Error('Displayed source URL differs from the bytes indexed by ffprobe.');
      },
      async seek(time: number): Promise<number> {
        if (pending) throw new Error('Overlapping source performance seeks');
        if (failure) throw failure;
        if (!visible()) throw new Error('Source performance requires a visible video');
        if (source() !== openedSource) throw new Error('Source identity changed after opening');
        if (options.native && monitor.asset !== options.asset) throw new Error('Source asset changed');
        const frame = selectedFrame(time);
        const began = performance.now();
        mark('request', { time, frame });
        let timer: ReturnType<typeof setTimeout>;
        let animation = 0;
        try {
          await new Promise<void>((resolve, reject) => {
            timer = setTimeout(() => pending?.reject(new Error(`Source presentation seek timed out at ${time}; requested frame ${frame}`)), 15_000);
            pending = { time, frame, generation: -1, result: false, began, resolve, reject };
            monitor.currentTime = time;
            pending.generation = options.native ? monitor.requested : 0;
          });
          await new Promise<void>((resolve, reject) => {
            // Keep the request deadline live through RAF as well.
            pending!.reject = reject;
            animation = requestAnimationFrame(() => {
              animation = 0;
              if (failure) { reject(failure); return; }
              if (!validShown(confirmed) || confirmed.frame !== frame || source() !== openedSource) { reject(new Error('Displayed source changed before RAF')); return; }
              mark('raf', { time, frame }); resolve();
            });
          });
          return performance.now() - began;
        } finally { clearTimeout(timer!); if (animation) cancelAnimationFrame(animation); pending = null; }
      },
      shown: () => validShown(confirmed) && !failure ? { frame: confirmed.frame, timestamp: confirmed.timestamp } : null,
      evidence: () => ({ events: trace, dropped }),
      dispose() {
        if (disposed) return;
        disposed = true;
        video.cancelVideoFrameCallback(callback); resize.disconnect();
        monitor.removeEventListener(event, onResult); monitor.removeEventListener('error', error);
        if (presenter) { presenter.paint = oldPaint; presenter.presentedAt = oldPresentedAt; }
        if (options.trace && media) media.frame = oldFrame;
      }
    };
  };
}
