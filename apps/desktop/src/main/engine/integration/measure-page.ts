/*
 * The page of the production frame bridge's S1 measurement
 * (scripts/frame-bridge-measure.mjs; measure-main.ts drives it).
 *
 * Two viewers on the same harness, as amendment 5's gate compares them:
 *
 * - `engine`: the production path. The viewer's `ViewerPresentation`
 *   presents the interactive engine's frames (bridge test source, 4K at
 *   60 fps) through the preload receiver, with its presenter's per-frame
 *   trace on.
 * - `today`: today's viewer, which draws in the renderer at the
 *   animation frame: a WebGPU canvas the same size draws the content frame
 *   the transport clock says is current (the frame code, a ramp, a moving
 *   bar, a full-screen pass), as the WebGL viewer renders at rAF. The
 *   engine still plays and renders into its pool (each frame is released
 *   on arrival), so both runs carry the same engine GPU load and use the
 *   same clock.
 *
 * Pointer-to-pixel. Synthetic pointer moves (coordinates encode their
 * sequence numbers) change the view: a pan for a drag, the editor camera
 * for orbit and walk (the engine's cube field). Today's path draws the
 * dragged marker itself at the next rAF.
 */
import { mediaTimeAt, TICKS_PER_SECOND, type ClockSample, type EditorCamera } from '../../../extensions/viewer/engine-present/protocol';
import { ViewerPresentation, type EngineViewerService, type ViewParams } from '../../../extensions/viewer/engine-present/presentation';
import { ViewSequencer, displayIsEdr, canvasColorSpace } from '../../../extensions/viewer/engine-present/view-state';
import { log, PageEngine, type PageChannel } from './page-engine';

/* eslint-disable @typescript-eslint/no-explicit-any */
const query = new URLSearchParams(location.search);
const displayHz = Number(query.get('hz')) || null;
const edrSetting = query.get('edr') === 'off' ? 'off' : 'auto';
/** Engine gestures read `pointerrawupdate` (not aligned to rAF) instead of `pointermove`. */
const rawInput = query.get('raw') === '1';
const COMPOSITION_WIDTH = 3840;

const engine = new PageEngine(displayHz);
const stage = document.getElementById('stage')!;
const overlay = document.getElementById('overlay')!;

type Mode = 'engine' | 'today';
type InputMode = 'drag' | 'orbit' | 'walk' | 'baseline';

let mode: Mode = 'engine';
let presentation: ViewerPresentation | null = null;
let channel: PageChannel | null = null;
let clock: ClockSample | null = null;
let hostOffsetUs: number | null = null;
const clocks: ClockSample[] = [];
let view: ViewParams = { zoom: 0.5, pan: [0, 0], resolution: 'full', media: 'automatic', playEveryFrame: false };

/* ── today's viewer: draws in the renderer at rAF ────────────────────── */

const TODAY_SHADER = /* wgsl */ `
struct U { size: vec2f, seq: u32, marker: u32, time: f32, pad: f32 };
@group(0) @binding(0) var<uniform> u: U;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  let x = floor(pos.x); let y = floor(pos.y);
  var c = vec3f(0.18);
  let cell = max(1.0, floor(u.size.x / 64.0));
  if (y < 32.0 && x < 32.0 * cell) { c = vec3f(f32((u.seq >> u32(x / cell)) & 1u)); }
  else if (y >= 64.0 && y < 320.0) { c = vec3f((x + 0.5) / u.size.x); }
  else if (y >= 832.0 && y < 1088.0) {
    let bx = (u.time * 960.0) % u.size.x;
    c = select(vec3f(0.2), vec3f(1.0), x >= bx && x < bx + 64.0);
  }
  var v = pos.xy / u.size;
  for (var i = 0u; i < 8u; i++) { v = fract(v * 1.618 + vec2f(sin(v.y * 6.28 + u.time), cos(v.x * 6.28))); }
  c = c + vec3f(v * 0.001, 0.0);
  let mx = f32(u.marker % 1200u) * 2.0 + 400.0;
  if (x >= mx && x < mx + 96.0 && y >= 1032.0 && y < 1128.0) { c = vec3f(1.0, 0.5, 0.0); }
  return vec4f(c, 1.0);
}`;

class TodayViewer {
  draws: [rafMs: number, frame: number, ptsTicks: number, rev: number][] = [];
  rafs: number[] = [];
  private lastFrame = -1;
  private lastRev = -1;
  rev = 0;
  private raf = 0;
  private constructor(private readonly device: any, private readonly context: any, private readonly pipeline: any, private readonly uniform: any, private readonly group: any, private readonly canvas: HTMLCanvasElement) {}

  static async create(): Promise<TodayViewer> {
    const gpu = (navigator as any).gpu;
    const adapter = await gpu.requestAdapter({ powerPreference: 'high-performance' });
    const device = await adapter.requestDevice();
    const canvas = document.createElement('canvas');
    canvas.width = 3840;
    canvas.height = 2160;
    canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%';
    stage.insertBefore(canvas, overlay);
    const context = canvas.getContext('webgpu') as any;
    const format = gpu.getPreferredCanvasFormat();
    context.configure({ device, format, alphaMode: 'opaque' });
    const module = device.createShaderModule({ code: TODAY_SHADER });
    const pipeline = device.createRenderPipeline({ layout: 'auto', vertex: { module, entryPoint: 'vs' }, fragment: { module, entryPoint: 'fs', targets: [{ format }] }, primitive: { topology: 'triangle-list' } });
    const uniform = device.createBuffer({ size: 32, usage: 0x40 | 0x08 });
    const group = device.createBindGroup({ layout: pipeline.getBindGroupLayout(0), entries: [{ binding: 0, resource: { buffer: uniform } }] });
    return new TodayViewer(device, context, pipeline, uniform, group, canvas);
  }

  start(): void {
    const loop = (t: number): void => {
      this.raf = requestAnimationFrame(loop);
      this.frame(t);
    };
    this.raf = requestAnimationFrame(loop);
  }

  /** Draws when the clock moved to a new content frame or input moved the marker. */
  private frame(rafMs: number): void {
    this.rafs.push(rafMs);
    const frameTicks = TICKS_PER_SECOND / 60;
    let frame = this.lastFrame;
    if (clock && hostOffsetUs !== null) {
      const media = mediaTimeAt(clock, rafMs * 1000 + hostOffsetUs);
      frame = Math.floor(media / frameTicks);
    }
    if (frame === this.lastFrame && this.rev === this.lastRev) return;
    this.lastFrame = frame;
    this.lastRev = this.rev;
    const data = new ArrayBuffer(32);
    new Float32Array(data, 0, 2).set([3840, 2160]);
    new Uint32Array(data, 8, 2).set([frame >>> 0, this.rev >>> 0]);
    new Float32Array(data, 16, 1).set([(frame * frameTicks) / TICKS_PER_SECOND]);
    this.device.queue.writeBuffer(this.uniform, 0, data);
    const encoder = this.device.createCommandEncoder();
    const pass = encoder.beginRenderPass({ colorAttachments: [{ view: this.context.getCurrentTexture().createView(), loadOp: 'clear', storeOp: 'store', clearValue: { r: 0, g: 0, b: 0, a: 1 } }] });
    pass.setPipeline(this.pipeline);
    pass.setBindGroup(0, this.group);
    pass.draw(3);
    pass.end();
    this.device.queue.submit([encoder.finish()]);
    this.draws.push([rafMs, frame, frame * frameTicks, this.rev]);
  }

  dispose(): void {
    cancelAnimationFrame(this.raf);
    this.canvas.remove();
    this.device.destroy();
  }
}

let today: TodayViewer | null = null;
const sequencer = new ViewSequencer();

function sendTodayState(): void {
  const rect = stage.getBoundingClientRect();
  const { state, changed } = sequencer.next({
    ...view,
    composition: 'bridge-test',
    cssWidth: rect.width,
    cssHeight: rect.height,
    devicePixelRatio: Math.min(devicePixelRatio || 1, 2),
    edr: displayIsEdr(window),
    colorSpace: canvasColorSpace(window)
  });
  if (changed) channel?.sendViewerState(state);
}

function onClock(sample: ClockSample, arrivedMs: number): void {
  clock = sample;
  clocks.push(sample);
  const offset = sample.hostTime - arrivedMs * 1000;
  if (hostOffsetUs === null || offset < hostOffsetUs) hostOffsetUs = offset;
}

/* ── input ────────────────────────────────────────────────────────── */

let inputMode: InputMode | null = null;
/** Per dispatch: [rev, seqs, event timeStamp ms, handled ms]. */
let inputs: [number, number[], number, number][] = [];

function lookAt(yaw: number, pitch: number, distance: number, walk: number): number[] {
  const p = Math.max(-1.4, Math.min(1.4, pitch));
  const dir = [Math.cos(p) * Math.sin(yaw), Math.sin(p), Math.cos(p) * Math.cos(yaw)];
  const target = [-dir[0]! * walk, 1, -dir[2]! * walk];
  const eye = target.map((t, i) => t + dir[i]! * distance);
  const norm = (v: number[]) => {
    const l = Math.hypot(...v);
    return v.map((x) => x / l);
  };
  const cross = (a: number[], b: number[]) => [a[1]! * b[2]! - a[2]! * b[1]!, a[2]! * b[0]! - a[0]! * b[2]!, a[0]! * b[1]! - a[1]! * b[0]!];
  const dot = (a: number[], b: number[]) => a[0]! * b[0]! + a[1]! * b[1]! + a[2]! * b[2]!;
  const f = norm(target.map((t, i) => t - eye[i]!));
  const s = norm(cross(f, [0, 1, 0]));
  const u = cross(s, f);
  return [s[0]!, u[0]!, -f[0]!, 0, s[1]!, u[1]!, -f[1]!, 0, s[2]!, u[2]!, -f[2]!, 0, -dot(s, eye), -dot(u, eye), dot(f, eye), 1];
}

function cameraFor(seq: number, kind: 'orbit' | 'walk'): EditorCamera {
  const yaw = kind === 'orbit' ? seq * 0.01 : 0.6;
  const walk = kind === 'walk' ? (seq % 600) * 0.05 : 0;
  return { view: lookAt(yaw, 0.35, 40, walk), projection: { kind: 'perspective', fovYDegrees: 50, near: 0.1, far: 500 } };
}

function onPointer(event: PointerEvent): void {
  if (!inputMode) return;
  const handled = performance.now();
  const events = event.getCoalescedEvents?.().length ? event.getCoalescedEvents() : [event];
  const seqs = events.map((e) => Math.round(e.clientX - 200) + Math.round(e.clientY - 300) * 1200);
  const seq = seqs[seqs.length - 1]!;
  let rev: number;
  if (inputMode === 'baseline') {
    today!.rev = seq + 1;
    rev = seq + 1;
  } else {
    if (inputMode === 'drag') view = { ...view, pan: [(seq % 400) - 200, 0] };
    else view = { ...view, camera: cameraFor(seq, inputMode), shading: 'solid' };
    presentation!.update(view);
    rev = presentation!.state!.viewSeq;
  }
  inputs.push([rev, seqs, event.timeStamp, handled]);
}
window.addEventListener('pointermove', (event) => {
  if (!rawInput || inputMode === 'baseline') onPointer(event);
});
window.addEventListener('pointerrawupdate' as 'pointermove', (event) => {
  if (rawInput && inputMode !== 'baseline') onPointer(event as PointerEvent);
});

/* ── the API measure-main.ts calls ────────────────────────────────── */

const api = {
  async start(m: Mode): Promise<void> {
    mode = m;
    if (m === 'engine') {
      // The presenter gets every message; the harness keeps the clock samples.
      const tapped: EngineViewerService = {
        source: engine.source,
        composition: () => engine.composition(),
        displayHz: () => engine.displayHz(),
        onReady: (listener) => engine.onReady(listener),
        open: (handlers) =>
          engine.open({
            message: (message, arrivedMs) => {
              if (message.type === 'clock') onClock(message.sample, arrivedMs);
              handlers.message(message, arrivedMs);
            },
            closed: handlers.closed
          })
      };
      presentation = new ViewerPresentation({ service: tapped, stage, before: overlay, trace: true, edr: edrSetting, log: (level, message) => log(`${level}: ${message}`) });
      presentation.update(view);
      new ResizeObserver(() => presentation?.update(view)).observe(stage);
    } else {
      today = await TodayViewer.create();
      const open = (): void => {
        const ch = engine.open({
          message: (message, arrivedMs) => {
            if (message.type === 'clock') onClock(message.sample, arrivedMs);
            else if (message.type === 'frame') ch?.release({ poolIndex: message.frame.poolIndex, seq: message.frame.seq });
          },
          closed: () => log('the session closed')
        }) as PageChannel | null;
        if (!ch) return;
        channel = ch;
        sendTodayState();
      };
      engine.onReady(open);
      open();
      new ResizeObserver(() => sendTodayState()).observe(stage);
      today.start();
    }
  },
  ready(): boolean {
    if (mode === 'engine') return !!presentation?.presenting;
    return !!channel && channel.session() !== null;
  },
  transport(command: any): boolean {
    const ch = mode === 'engine' ? engine.channels.at(-1) : channel;
    if (!ch || ch.session() === null) return false;
    ch.transport(command);
    return true;
  },
  mark(name: string): number {
    performance.mark(name);
    return performance.now();
  },
  now: (): number => performance.now(),
  /** Empties the per-frame records so a recording starts clean. */
  reset(): void {
    presentation?.takeTrace();
    if (today) {
      today.draws = [];
      today.rafs = [];
    }
    inputs = [];
  },
  setInputMode(m: InputMode | null): void {
    inputMode = m;
    if (m === 'drag' || m === null) view = { zoom: 0.5, pan: [0, 0], resolution: 'full', media: 'automatic', playEveryFrame: false };
    if (m === 'orbit' || m === 'walk') view = { ...view, camera: cameraFor(0, m), shading: 'solid' };
    presentation?.update(view);
  },
  collect() {
    const trace = presentation?.takeTrace() ?? null;
    return {
      mode,
      presenter: presentation?.stats() ?? null,
      engineTrace: trace,
      today: today ? { draws: today.draws, rafs: today.rafs } : null,
      clocks: clocks.splice(0),
      inputs,
      state: presentation?.state ?? sequencer.current,
      receiver: engine.source.stats()
    };
  },
  /** Agent tool traffic (the S1 load): a layer listing for a request. */
  agent(request: { id: number }): { id: number; layers: unknown[] } {
    const layers = [];
    for (let i = 0; i < 600; i++) layers.push({ id: `layer-${i}`, name: `Layer ${i}`, in: i * 0.1, out: i * 0.1 + 5, props: { x: i, y: i * 2, opacity: 1, effects: ['blur', 'glow'] } });
    return { id: request.id, layers };
  }
};

(window as unknown as { __measure: typeof api }).__measure = api;
log('measure page ready');
