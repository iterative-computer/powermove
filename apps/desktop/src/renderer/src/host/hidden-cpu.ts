/** CPU presentation for the local native hidden host. The production kernel,
 * raster wrappers and panel DOM stay in this tab; composition pixels come from
 * the supervised pm-node process through the desktop snapshot contract. */
import { encodeFrame, decodeFrame } from '../../../shared/wire';
import type { EngineSnapshotRequest, EngineSnapshotReply } from '../../../shared/engine-ipc';
import type { PMRegistry } from '../legacy/registry';
import { EngineRenderTransport } from '../engine-connection/render-transport';
import { installAppCaptures, usesBlender } from '../engine-connection/app-captures';
import { ShaderFallbackRunner } from '../engine-connection/shader-fallback';
import { SvgRasters } from '../engine-connection/svg-rasters';

export function installHiddenCpu(PM: PMRegistry): void {
  const log = (level: string, message: string) => console[level === 'error' ? 'error' : level === 'warn' ? 'warn' : 'info'](`[native CPU] ${message}`);
  const runner = new ShaderFallbackRunner();
  const svg = new SvgRasters({ asset: id => PM.assets?.get?.(id) ?? null, assets: () => PM.assets?.map?.values?.() ?? [], log });
  PM.bus?.on?.('assets', () => svg.prime());
  const payload = (request: EngineSnapshotRequest) => encodeFrame({ project: PM.proj.id, request, definitions: (PM as any).EngineDefinitions?.publisher.all() ?? [] });
  const native = {
    snapshotSync(request: EngineSnapshotRequest): EngineSnapshotReply {
      const xhr = new XMLHttpRequest(); xhr.open('POST', '/__powermove/native-snapshot', false);
      // A synchronous window XHR cannot select arraybuffer. The byte-preserving
      // MIME override keeps the binary contract, including RGBA and f16 inputs.
      xhr.overrideMimeType('text/plain; charset=x-user-defined'); xhr.send(new Uint8Array(payload(request)).buffer);
      if (xhr.status !== 200) throw new Error(`native-snapshot-unavailable: HTTP ${xhr.status}`);
      return decodeFrame(Uint8Array.from(xhr.responseText, c => c.charCodeAt(0) & 255)) as EngineSnapshotReply;
    },
    async snapshot(request: EngineSnapshotRequest): Promise<EngineSnapshotReply> {
      const response = await fetch('/__powermove/native-snapshot', { method: 'POST', body: payload(request) as BodyInit });
      if (!response.ok) throw new Error(`native-snapshot-unavailable: HTTP ${response.status}`);
      return decodeFrame(new Uint8Array(await response.arrayBuffer())) as EngineSnapshotReply;
    }
  };
  const render = new EngineRenderTransport({ native, documentOnly: true, session: () => null,
    revision: () => { PM.EngineDoc?.barrier?.flush(); return PM.EngineDoc?.barrier?.revision ?? null; },
    composition: () => String(PM.proj.compId ?? PM.proj.id), document: () => String(PM.proj.id), viewSeq: () => null,
    fallbackRunner: () => runner, svgRasters: () => svg, usesBlender: () => usesBlender(PM.proj), log,
    windowDocument: () => {
      const doc = PM.EngineDoc, barrier = doc?.barrier;
      if (!doc?.enabled || !barrier || !doc.ready) return null;
      barrier.flush();
      return { id: String(PM.proj.id), revision: barrier.revision, full: () => {
        const snapshot = doc.snapshot(); if (!snapshot) throw new Error('native-document-unavailable');
        return { json: new TextEncoder().encode(snapshot.json), hash: snapshot.hash, revision: snapshot.revision };
      } };
    }
  });
  render.provideCompat('svgImage', request => svg.produce(request));
  (PM as any).EngineRender = render;
  (PM as any).EngineSvgRasters = svg;
  installAppCaptures(PM, render);
  let bound: HTMLCanvasElement | null = null, output: HTMLCanvasElement | null = null, originalVisibility = '';
  const syncStyle = () => {
    if (!bound || !output) return;
    const style = getComputedStyle(bound);
    output.className = bound.className;
    output.style.cssText = `position:absolute;display:block;left:${bound.offsetLeft}px;top:${bound.offsetTop}px;width:${style.width};height:${style.height};background:${style.backgroundColor};${bound.style.cssText}`;
    output.style.visibility = originalVisibility; output.style.pointerEvents = 'none';
  };
  let pending: { time: number; options: any } | null = null, drawing = false;
  const bind = (canvas: HTMLCanvasElement) => {
    if (canvas === bound) return;
    if (bound) bound.style.visibility = originalVisibility;
    output?.remove(); bound = canvas;
    output = document.createElement('canvas'); output.dataset.nativeCpu = 'true';
    originalVisibility = canvas.style.visibility; syncStyle();
    canvas.after(output); canvas.style.visibility = 'hidden';
  };
  (PM as any).EngineCanvas = { bind, resize: syncStyle, get bound() { return bound; } };
  const draw = async () => {
    if (drawing) return; drawing = true;
    try {
      while (pending) {
        const frame = pending; pending = null;
        const project = PM.proj, canvas = bound, target = output;
        if (!canvas || !target || !canvas.isConnected || !render.ready || !project.id || PM.isHomeProject?.()) continue;
        const viewport = PM.GL.previewViewport;
        const view = viewport ?? { x: 0, y: 0, width: project.w, height: project.h };
        const canvasWidth = canvas.width, canvasHeight = canvas.height, version = PM.animVersion?.(), quality = PM.quality;
        const scaleX = canvasWidth / view.width, scaleY = canvasHeight / view.height;
        const width = Math.max(1, Math.round(project.w * scaleX)), height = Math.max(1, Math.round(project.h * scaleY));
        const image = await render.snapshot({ time: frame.time, width, height, pixels: 'opaque', background: frame.options.bg !== false });
        if (PM.proj !== project || output !== target || PM.GL.previewViewport !== viewport || canvas.width !== canvasWidth || canvas.height !== canvasHeight) continue;
        const offscreen = document.createElement('canvas'); offscreen.width = image.width; offscreen.height = image.height;
        offscreen.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(image.data), image.width, image.height), 0, 0);
        syncStyle(); target.width = canvas.width; target.height = canvas.height;
        const ctx = target.getContext('2d')!; ctx.clearRect(0, 0, target.width, target.height);
        const sx = width / project.w, sy = height / project.h;
        ctx.drawImage(offscreen, view.x * sx, view.y * sy, view.width * sx, view.height * sy, 0, 0, target.width, target.height);
        PM.bus.emit('preview:presented', { viewport, time: frame.time, version, project, quality });
        syncStyle();
      }
    } catch (error) { log('error', String(error)); }
    finally { drawing = false; if (pending) void draw(); }
  };
  PM.GL.render = (time: number, options: any = {}) => { pending = { time, options }; void draw(); return true; };
  PM.GL.renderToPixels = (time: number, width: number, height: number, options: any = {}) => {
    const image = render.snapshotSync({ time, width, height, composition: options.comp?.id,
      pixels: options.opaque ? 'opaque' : 'premultiplied', background: options.bg !== false, draft3d: options.draft3d === true });
    const data = new Uint8Array(image.data);
    // Today's renderToPixels is bottom-up except its topDownOpaque variant.
    if (!options.topDownOpaque) for (let y = 0; y < Math.floor(height / 2); y++) {
      const row = data.slice(y * width * 4, (y + 1) * width * 4), other = height - 1 - y;
      data.copyWithin(y * width * 4, other * width * 4, (other + 1) * width * 4); data.set(row, other * width * 4);
    }
    return data;
  };
}
