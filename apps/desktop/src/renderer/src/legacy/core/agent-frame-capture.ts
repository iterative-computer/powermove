import { prepareFrame } from './frame-preparation';
import { captureAborted, VideoFrameDecodeError, withCaptureSignal } from './video-frame-decode';

type Queue = { tail: Promise<unknown>; pending: number };
const queues = new WeakMap<object, Queue>();

/** Agent reviews own disposable decoders. Viewer scrubbing, proxy installation
 * and a new project's media registry must never move their decoder time. */
export function captureAgentFrame(PM: any, exporter: any, time: number, maxWidth = 480,
  options: { signal?: AbortSignal } = {}): Promise<string> {
  const project = PM.proj, composition = project.compId, revision = project.revision;
  let queue = queues.get(PM);
  if (!queue) { queue = { tail: Promise.resolve(), pending: 0 }; queues.set(PM, queue); }
  if (queue.pending >= 8) return Promise.reject(new Error('Too many composition previews are queued.'));
  const controller = new AbortController();
  const cancel = () => controller.abort(new DOMException('The composition changed during preview', 'AbortError'));
  const checkProject = () => {
    if (PM.proj !== project || project.compId !== composition || project.revision !== revision) cancel();
  };
  const cancelRequest = () => controller.abort(options.signal?.reason ?? new DOMException('Frame capture cancelled', 'AbortError'));
  options.signal?.addEventListener('abort', cancelRequest, { once: true });
  if (options.signal?.aborted) cancelRequest();
  const unsubscribers = ['project', 'layers'].map(event => PM.bus?.on?.(event, checkProject));
  queue.pending++;
  const run = queue.tail.catch(() => undefined).then(async () => {
    checkProject(); captureAborted(controller.signal);
    if (exporter.busy || PM.agentFrameCapture) throw new Error('Wait for the current frame capture or export to finish.');
    const wasPlaying = PM.playing;
    const decoders = new Map<any, HTMLVideoElement>();
    const canvases = new Set<HTMLCanvasElement>();
    const disposeDecoders = () => {
      for (const video of decoders.values()) {
        try { video.pause(); video.removeAttribute('src'); video.load(); } catch { /* Already detached. */ }
      }
      decoders.clear();
    };
    const videoForCapture = (asset: any) => {
      let video = decoders.get(asset);
      if (!video) {
        video = document.createElement('video');
        video.muted = true; video.playsInline = true; video.preload = 'auto';
        decoders.set(asset, video);
        video.src = asset.el.currentSrc || asset.el.src || asset.url;
      }
      return video;
    };
    // Keep the original ten-second ceiling, including one fresh-decoder retry.
    const deadline = setTimeout(() => controller.abort(new VideoFrameDecodeError()), 10_000);
    let ownedFrames: Map<string, HTMLCanvasElement> | undefined;
    PM.pause(); PM.agentFrameCapture = true;
    try {
      const scale = Math.min(1, maxWidth / project.w);
      const prepare = () => prepareFrame(PM, time, project, {
        width: Math.max(2, Math.round(project.w * scale)), height: Math.max(2, Math.round(project.h * scale)),
        mblur: false, signal: controller.signal, videoFrameTimeoutMs: 5_000, videoForCapture,
        onVideoFrame: canvas => canvases.add(canvas)
      });
      try { await prepare(); }
      catch (error) {
        captureAborted(controller.signal);
        if (!(error instanceof VideoFrameDecodeError)) throw error;
        // A decoder that errored or stopped producing frames cannot recover by
        // waiting longer on the same element. Retry once with a clean resource.
        disposeDecoders(); await prepare();
      }
      checkProject(); captureAborted(controller.signal);
      ownedFrames = PM.preparedVideoFrames;
      return exporter.snapshot(time, maxWidth);
    } finally {
      clearTimeout(deadline); disposeDecoders();
      for (const canvas of canvases) { canvas.width = 0; canvas.height = 0; }
      if (ownedFrames && PM.preparedVideoFrames === ownedFrames) PM.preparedVideoFrames = null;
      if (ownedFrames && PM.preparedVideoExactInputs?.frames === ownedFrames) PM.preparedVideoExactInputs = null;
      PM.agentFrameCapture = false;
      PM.invalidate('render');
      if (wasPlaying && PM.proj === project && project.compId === composition) PM.play();
    }
  });
  const result = run.finally(() => {
    queue!.pending--;
    options.signal?.removeEventListener('abort', cancelRequest);
    for (const off of unsubscribers) if (typeof off === 'function') off();
  });
  queue.tail = result.catch(() => undefined);
  return withCaptureSignal(result, controller.signal);
}
