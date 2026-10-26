import { hasLayerInputs } from '../../core/effect-inputs';
import { createAutomaticPreview } from './automatic-preview';
import { createAutomaticPreviewVideos } from './automatic-preview-video';
import { viewerService } from './services';

/** Idle preparation owns no transport, visible canvas or source decoder. */
export function installAutomaticPreview(PM: any) {
  const document = window.document;
  const videos = createAutomaticPreviewVideos(PM);
  let blockedBefore = true, pressureUntil = 0, disposed = false;
  let eligibilityProject: any, eligibilityVersion: any, eligible = true;
  let lastArea = '';
  const canPrepare = () => {
    const project = PM.proj, version = PM.animVersion?.();
    if (eligibilityProject !== project || eligibilityVersion !== version) {
      eligibilityProject = project; eligibilityVersion = version;
      const layers = PM.ProjectIndex?.allLayers?.() || project?.layers || [];
      // A single frozen source time cannot stand in for temporal video samples.
      // Input effects currently use invocation-owned dependency surfaces, not
      // retained preview frames. Avoid competing speculative decoder work.
      eligible = !hasLayerInputs(project, (fx:any)=>PM.effectDef?.(fx) ?? PM.Kernel?.effects?.get?.(fx.type)) && !(layers.some((layer: any) => layer.mblur)
        && layers.some((layer: any) => layer.type === 'video'));
    }
    return eligible;
  };
  const options = () => ({ mblur: true, mbSamples: 6, shutter: PM.proj?.shutter || .5, hideShy: false });
  const blocked = () => {
    const gl = PM.GL;
    return !gl?.gl || !gl.canvas?.width || !gl.canvas?.height || !PM.proj?.layers?.length
      || PM.playing || PM.Export?.busy || PM.agentFrameCapture || PM.Preview?.preparing || PM.Preview?.active
      || PM.preparedVideoFrames || PM.assets?.loading?.size || gl.compiling || gl.errors?.size
      || PM.interactionActive?.() || PM.ProjectsScreen?.isOpen || PM.SettingsUI?.isOpen
      || document.hidden || performance.now() < pressureUntil
      || document.fonts?.status === 'loading' || viewerService(PM)?.preview?.activeId
      || (typeof navigator !== 'undefined' && (navigator as any).scheduling?.isInputPending?.())
      || (PM.Memory?.budget?.('preview') ?? 256 * 1024 * 1024) === 0 || !canPrepare();
  };
  const state = () => {
    const project = PM.proj;
    const budget = PM.Memory?.budget?.('preview') ?? 256 * 1024 * 1024;
    const work = project?.work;
    return {
      project, key: PM.GL?.previewPreparationKey?.() || '', time: PM.time || 0,
      fps: Math.max(1, Number(PM.previewFps || project?.fps) || 30),
      start: work?.[1] > work?.[0] ? work[0] : 0,
      end: work?.[1] > work?.[0] ? work[1] : project?.dur || 0,
      blocked: !!blocked(),
      budget, bytes: (PM.GL?.previewFrames?.bytes || 0) + (PM.Preview?.bytes || 0),
      frameBytes: (PM.GL?.canvas?.width || 0) * (PM.GL?.canvas?.height || 0) * 4,
    };
  };
  const automatic = createAutomaticPreview({
    state,
    has: time => !!PM.GL?.hasPreviewFrame?.(time, options()),
    async prepare(time, valid) {
      let frames: Map<string, ImageBitmap> | null = null;
      try {
        if (!valid()) return false;
        frames = await videos.prepare(time, valid);
        if (!frames || !valid()) return false;
        return await PM.GL.preparePreviewFrame(time, options(), frames, valid);
      } finally { if (frames) videos.release(frames); }
    },
    schedule(run, delay) {
      let idle: number | undefined, cancelled = false;
      const timer = setTimeout(() => {
        if (cancelled) return;
        if (typeof window.requestIdleCallback === 'function') {
          const attempt = (deadline: IdleDeadline) => {
            idle = undefined;
            if (cancelled) return;
            // A short idle slice defers this frame, rather than resetting the
            // entire idle window forever on high-refresh/busy displays.
            if (deadline.timeRemaining() > 1 || deadline.didTimeout) run();
            else idle = window.requestIdleCallback(attempt, { timeout: 1000 });
          };
          idle = window.requestIdleCallback(attempt, { timeout: 1000 });
        } else run();
      }, delay);
      return () => { cancelled = true; clearTimeout(timer); if (idle !== undefined) window.cancelIdleCallback?.(idle); };
    },
  });
  const request = () => {
    if (disposed) return;
    const paused = !!blocked();
    // Hidden/Home/playing editors require no frame scans or signature work.
    if (paused && blockedBefore) return;
    if (paused && !blockedBefore) {
      PM.GL?.cancelPreviewPreparation?.(); videos.cancel();
    }
    blockedBefore = paused;
    if (!paused) {
      const current = state();
      const anchor = Math.floor(current.time * current.fps + 1e-7);
      const area = [current.key, anchor, current.start, current.end, current.budget].join('|');
      if (area !== lastArea) {
        lastArea = area;
        if (current.bytes + current.frameBytes > current.budget) {
          const endExclusive = Math.ceil(current.end * current.fps - 1e-7);
          const first = Math.max(0, Math.ceil(current.start * current.fps - 1e-7),
            Math.min(endExclusive - 1, anchor));
          const count = Math.max(0, Math.min(4096, Math.floor(current.budget / Math.max(1, current.frameBytes)),
            endExclusive - first));
          let complete = true;
          // Check once after navigation, never on every display tick. A ready
          // first frame must not hide gaps later in a full cache's work area.
          for (let index = 0; index < count; index++) {
            if (!PM.GL.hasPreviewFrame((first + index) / current.fps, options())) { complete = false; break; }
          }
          if (!complete) PM.GL.reservePreviewPreparation?.(count * current.frameBytes);
        }
      }
    }
    automatic.request();
  };
  const cancel = () => {
    automatic.cancel(); PM.GL?.cancelPreviewPreparation?.(); videos.cancel();
  };
  const subscriptions = ['transport', 'time', 'project', 'layers', 'assets', 'quality', 'fonts', 'projects:screen', 'settings:screen']
    .map(event => PM.bus?.on?.(event, request));
  const pressure = () => { pressureUntil = performance.now() + 30_000; cancel(); };
  window.addEventListener?.('memorypressure', pressure);
  document.addEventListener?.('visibilitychange', request);
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    automatic.dispose(); PM.GL?.cancelPreviewPreparation?.(); videos.dispose();
    for (const off of subscriptions) if (typeof off === 'function') off();
    window.removeEventListener?.('memorypressure', pressure);
    window.removeEventListener?.('pagehide', dispose);
    document.removeEventListener?.('visibilitychange', request);
  };
  window.addEventListener?.('pagehide', dispose, { once: true });
  const service = { request, cancel, dispose, get stagingBytes() { return videos.bytes; } };
  PM.AutomaticPreview = service;
  return service;
}
