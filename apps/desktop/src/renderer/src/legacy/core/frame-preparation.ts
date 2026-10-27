import { captureAborted, seekVideoFrame, VideoFrameDecodeError, withCaptureSignal } from './video-frame-decode';
import { cancelPreviewVideoSeek } from './video-seek';
import { pauseVideoInstances } from './video-instances';
import { videoPlaybackTime } from '../../../../shared/video-timing';
import { sourceTime } from './retiming';
import type { DecodedVideoFrame, ExportVideoDecoders } from './export-video-decoder';

type FontReadiness = {
  fonts: FontFaceSet | null;
  generation: number;
  readyKey: unknown[] | null;
  pending: { key: unknown[]; promise: Promise<unknown> } | null;
  invalidate: () => void;
};
const fontReadiness = new WeakMap<object, FontReadiness>();
const fontEvents = ['loading', 'loadingdone', 'loadingerror'];
const sameFontKey = (first: unknown[] | null, second: unknown[]) => !!first && first.every((value, i) => Object.is(value, second[i]));
const fontKey = (PM: any, project: any, state: FontReadiness, fonts: FontFaceSet) =>
  [state.generation, PM.proj, project, project.layers, project.layers?.length, PM.animVersion?.(), fonts.size];

/** True only when preparation can reuse a completed wait without reading the
 * fonts.ready layout getter or starting another font-loading wait. */
export function frameFontsReady(PM: any, project = PM.proj): boolean {
  const fonts = document.fonts;
  if (!fonts) return true;
  const state = fontReadiness.get(PM);
  return !!state && state.fonts === fonts && fonts.status !== 'loading'
    && sameFontKey(state.readyKey, fontKey(PM, project, state, fonts));
}

/** Chromium's resolved fonts.ready getter synchronously updates layout. Reuse
 * a completed wait until font loading or document content can change it. */
async function prepareFonts(PM: any, project: any): Promise<void> {
  let state = fontReadiness.get(PM);
  if (!state) {
    const created: FontReadiness = { fonts: null, generation: 0, readyKey: null, pending: null,
      invalidate: () => { created.generation++; created.readyKey = null; created.pending = null; } };
    state = created;
    fontReadiness.set(PM, state);
    for (const event of ['fonts', 'layers', 'project']) PM.bus?.on?.(event, state.invalidate);
    PM.Kernel?.events?.on?.('extensions:changed', state.invalidate);
    PM.Kernel?.events?.on?.('theme:changed', state.invalidate);
  }
  for (;;) {
    const fonts = document.fonts;
    if (!fonts) return;
    if (state.fonts !== fonts) {
      for (const event of fontEvents) state.fonts?.removeEventListener?.(event, state.invalidate);
      state.fonts = fonts; state.invalidate();
      for (const event of fontEvents) fonts.addEventListener?.(event, state.invalidate);
    }
    const key = () => fontKey(PM, project, state!, fonts);
    const captured = key();
    if (fonts.status !== 'loading' && sameFontKey(state.readyKey, captured)) return;
    const pending = sameFontKey(state.pending?.key ?? null, captured)
      ? state.pending! : { key: captured, promise: Promise.resolve(fonts.ready) };
    state.pending = pending;
    await pending.promise;
    // A new load, edit or FontFaceSet can arrive while the old promise settles.
    // Its result cannot stand in for the current document's readiness.
    if (document.fonts !== fonts || fonts.status === 'loading' || !sameFontKey(captured, key())) continue;
    state.readyKey = captured;
    if (state.pending === pending) state.pending = null;
    return;
  }
}

export type FramePreparationOptions = {
  width?: number; height?: number; mblur?: boolean; mbSamples?: number; shutter?: number;
  signal?: AbortSignal; videoFrameTimeoutMs?: number;
  videoForCapture?: (asset: any) => HTMLVideoElement;
  /** Export owns these recovered resources across frames and releases them at completion. */
  recoveryDecoders?: Map<HTMLVideoElement, HTMLVideoElement>;
  /** Export decodes supported clips in order instead of seeking for every frame. */
  sequentialDecoders?: ExportVideoDecoders;
  onVideoFrame?: (canvas: HTMLCanvasElement) => void;
};
type VideoRequest = { key?: string; layer: any; asset: any; time: number; at: number; frame?: HTMLCanvasElement };
const concurrentVideoDecoders = 4;

async function prepareVideos(requests: VideoRequest[], budget = Infinity, cancelled = () => false, exact = false, capture: FramePreparationOptions = {}, inputCapture = false): Promise<Map<string, HTMLCanvasElement>> {
  let bytes = 0;
  const owned = new Set<HTMLCanvasElement>();
  const checkCancellation = () => { if (cancelled()) throw new DOMException('Frame preparation cancelled', 'AbortError'); };
  // Asset aliases can share one decoder. Distinct source times on that element
  // must stay sequential, while independent decoders can seek together.
  const decoders = new Map<HTMLVideoElement, VideoRequest[]>();
  const paused = new Set<object>();
  for (const request of requests) {
    const { asset } = request;
    const video = capture.videoForCapture?.(asset) ?? asset.el;
    if (!capture.videoForCapture && !paused.has(asset)) {
      pauseVideoInstances(asset); asset.preview?.el.pause();
      cancelPreviewVideoSeek(asset.el); paused.add(asset);
    }
    const targets = decoders.get(video);
    if (targets) targets.push(request);
    else decoders.set(video, [request]);
  }
  const groups = [...decoders];
  let next = 0, failed = false, failure: unknown;
  const controller = new AbortController();
  const abort = () => controller.abort(capture.signal?.reason);
  capture.signal?.addEventListener('abort', abort, { once: true });
  if (capture.signal?.aborted) abort();
  const cancellationPoll = setInterval(() => { if (cancelled()) controller.abort(); }, 100);
  const worker = async () => {
    try {
      while (!failed && next < groups.length) {
        const [original, targets] = groups[next++]!;
        let replacement = capture.recoveryDecoders?.get(original);
        if (replacement && replacement.src !== (original.currentSrc || original.src)) {
          replacement.pause(); replacement.removeAttribute('src'); replacement.load();
          capture.recoveryDecoders?.delete(original); replacement = undefined;
        }
        let video = replacement ?? original;
        try {
          const captured = new Map<number | string, Map<string, HTMLCanvasElement>>();
          let capturedSource = video.currentSrc || video.src;
          for (const request of targets) {
            if (failed) return;
            checkCancellation();
            // Keep traversal order and preserve a separately keyed immutable
            // snapshot for each requested source time, including shared clips.
            const seek = () => seekVideoFrame(video, request.at, { tolerance: exact ? .0000011 : .0005,
              signal: controller.signal, timeoutMs: capture.videoFrameTimeoutMs ?? 15_000 });
            // A decoded frame is the one this seek would show; null keeps seeking.
            let decoded: DecodedVideoFrame | null = null;
            if (capture.sequentialDecoders && !capture.videoForCapture) {
              decoded = await capture.sequentialDecoders.frame(request.asset, request.layer.id, request.at, controller.signal);
              if (failed) return;
              checkCancellation();
            }
            if (!decoded) try {
              try { await seek(); }
              catch (error) {
                captureAborted(controller.signal); checkCancellation();
                // Private agent captures already own their retry/deadline. For
                // export, a fresh decoder can recover a stalled media pipeline.
                if (!(error instanceof VideoFrameDecodeError) || capture.videoForCapture || replacement
                    || !(original.currentSrc || original.src)) throw error;
                replacement = document.createElement('video');
                replacement.muted = true; replacement.playsInline = true; replacement.preload = 'auto';
                replacement.crossOrigin = original.crossOrigin;
                replacement.src = original.currentSrc || original.src;
                video = replacement;
                capture.recoveryDecoders?.set(original, replacement);
                await seek();
              }
            } catch (error) {
              if (!(error instanceof VideoFrameDecodeError)) throw error;
              throw new VideoFrameDecodeError(`Could not decode video frame for clip "${request.layer.name || request.layer.id}" at timeline ${request.time.toFixed(3)}s (source ${request.at.toFixed(6)}s). ${replacement ? 'A fresh decoder retry also failed. ' : ''}${error.message}. Check this part of the source video; relink it or convert it to H.264 MP4, then export again.`);
            }
            if (failed) return;
            checkCancellation();
            const source = video.currentSrc || video.src;
            if (source !== capturedSource) { captured.clear(); capturedSource = source; }
            // The seek tolerance can preserve a nearby decoded time on one
            // request and perform an exact seek on a later identical request.
            // Decoded frames are keyed by their own start time, apart from seeks.
            const decodedTime = decoded ? `decoded:${decoded.time}` : video.currentTime;
            let sizes = captured.get(decodedTime);
            if (!sizes) { sizes = new Map(); captured.set(decodedTime, sizes); }
            const w = request.asset.w || video.videoWidth, h = request.asset.h || video.videoHeight;
            const key = `${w}x${h}`;
            let canvas = sizes.get(key);
            if (!canvas) {
              const added = w * h * 4;
              if (!Number.isFinite(added) || bytes + added > budget) throw new Error('Video frames exceed the texture memory budget');
              bytes += added;
              canvas = document.createElement('canvas'); owned.add(canvas); capture.onVideoFrame?.(canvas); canvas.width = w; canvas.height = h;
              canvas.getContext('2d')!.drawImage(decoded ? decoded.frame : video, 0, 0, canvas.width, canvas.height);
              sizes.set(key, canvas);
            }
            request.frame = canvas;
          }
        } finally {
          if (replacement && !capture.recoveryDecoders) { replacement.pause(); replacement.removeAttribute('src'); replacement.load(); }
        }
      }
    } catch (error) {
      if (!failed) { failed = true; failure = inputCapture && cancelled() ? new DOMException('Frame preparation cancelled', 'AbortError') : error; controller.abort(); }
    }
  };
  // Abort and drain sibling waits on failure before restoring the preview;
  // no older worker may capture or publish pixels after this job ends.
  try { await Promise.all(Array.from({ length: Math.min(concurrentVideoDecoders, groups.length) }, worker)); }
  finally { clearInterval(cancellationPoll); capture.signal?.removeEventListener('abort', abort); }
  if (failed) { for (const canvas of owned) { canvas.width = 0; canvas.height = 0; } throw failure; }
  const frames = new Map<string, HTMLCanvasElement>();
  // Preserve traversal order when repeated nested layers share a lookup key.
  for (const request of requests) frames.set(request.key ?? request.layer.id + '@' + request.time, request.frame!);
  return frames;
}
/** Preserve each instance's decoded frame even when several layers share media. */
export async function prepareFrame(PM:any,time:number,project=PM.proj,options: FramePreparationOptions = {}):Promise<void> {
  const cancelled = () => !!options.signal?.aborted || PM.Export?.busy && PM.Export.cancel;
  if (cancelled()) throw new DOMException('Frame preparation cancelled', 'AbortError');
  const requests:VideoRequest[]=[];
  const visit=(comp:any,t:number,depth:number)=>{if(depth>8)throw new Error('Composition nesting is too deep');PM.scope.push(comp);try{for(const layer of comp.layers){if(layer.type!=='video'&&layer.type!=='precomp')continue;if(!PM.active(layer,t))continue;if(layer.type==='video'){const asset=PM.assets.get(layer.d.asset);if(asset?.el)requests.push({layer,asset,time:t,at:videoPlaybackTime(asset,sourceTime(PM,layer,t),1/project.fps)});}else if(layer.type==='precomp'){const sub=comp.comps?.[layer.d.comp]||project.comps?.[layer.d.comp];if(sub)visit(sub,sourceTime(PM,layer,t),depth+1);}}}finally{PM.scope.pop();}};
  visit(project,time,0);
  const frames = requests.length ? await prepareVideos(requests, Infinity, cancelled, false, options) : new Map<string, HTMLCanvasElement>();
  // Font loading can yield after all seeks finish. Do not publish a frame
  // cancelled during that final wait, and release captures owned by this job.
  try{await (options.videoForCapture ? withCaptureSignal(prepareFonts(PM,project),options.signal) : prepareFonts(PM,project));if(cancelled())throw new DOMException('Frame preparation cancelled', 'AbortError');}
  catch(error){for(const frame of new Set(frames.values())){frame.width=0;frame.height=0;}throw error;}
  PM.preparedVideoFrames=frames;PM.preparedVideoVersion=(PM.preparedVideoVersion||0)+1;
}
