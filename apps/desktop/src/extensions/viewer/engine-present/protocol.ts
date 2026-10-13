/*
 * The slice of contract 2 (frames) and contract 6 (clock) the viewer's
 * engine presentation reads. Extensions may import only `powermove` and
 * their own folder, so these mirror the generated engine-protocol types
 * (src/shared/engine-protocol) field for field; the kernel's engine
 * connection hands the presenter decoded messages of these shapes.
 */

/** Ticks per second (pm-time's flick-compatible base, 1/705,600,000 s). */
export const TICKS_PER_SECOND = 705_600_000;

export type SurfaceFormat = 'bgra8Unorm' | 'rgba16Float';
export type ViewerColorSpace = 'srgb' | 'displayP3';
export type PreviewResolution = 'automatic' | 'full' | 'half' | 'quarter';
export type MediaPreference = 'automatic' | 'originals' | 'proxies';
export type Shading3d = 'wireframe' | 'solid' | 'material' | 'rendered';

export type Rect = { x: number; y: number; width: number; height: number };
export type Quad = [[number, number], [number, number], [number, number], [number, number]];

export type PathCommand =
  | { op: 'moveTo'; x: number; y: number }
  | { op: 'lineTo'; x: number; y: number }
  | { op: 'cubicTo'; x1: number; y1: number; x2: number; y2: number; x: number; y: number }
  | { op: 'close' };

export type OverlayLayer = {
  layer: string;
  world2d: number[] | null;
  world3d: number[] | null;
  bounds: Quad;
  paths: PathCommand[][];
};

export type OverlayGeometry = {
  viewport: Rect;
  layers: OverlayLayer[];
  caret: { top: [number, number]; bottom: [number, number] } | null;
};

export type FrameQuality = {
  resolution: PreviewResolution;
  representation: 'original' | 'reducedResolution' | 'proxy' | 'optimizedMedia' | 'keptRender';
  draft3d: boolean;
  cached: boolean;
};

export type FrameDescriptor = {
  poolIndex: number;
  poolGeneration: number;
  seq: number;
  pts: number;
  revision: number;
  colorSpace: ViewerColorSpace;
  format: SurfaceFormat;
  quality: FrameQuality;
  overlay: OverlayGeometry | null;
  depthId: { poolIndex: number; seq: number } | null;
  viewSeq: number;
  /** Ticks; `null` for frames shown at the next refresh. */
  duration: number | null;
};

export type SurfacePool = {
  generation: number;
  surfaces: { poolIndex: number; size: { width: number; height: number }; format: SurfaceFormat }[];
};

export type ClockSample = {
  seq: number;
  epoch: number;
  state: 'stopped' | 'playing' | 'shuttling';
  hostTime: number;
  mediaTime: number;
  rate: { num: number; den: number };
  outputLatencyUs: number;
  sampleRate: number;
  loopRange: { start: number; end: number } | null;
};

export type EditorCamera = {
  view: number[];
  projection:
    | { kind: 'perspective'; fovYDegrees: number; near: number; far: number }
    | { kind: 'orthographic'; halfHeight: number; near: number; far: number };
};

export type ViewOverride = { camera: EditorCamera | null; shading: Shading3d; xray: boolean; localView: string[] };

export type ViewerState = {
  composition: string;
  size: { width: number; height: number };
  devicePixelRatio: number;
  zoom: number;
  pan: [number, number];
  resolution: PreviewResolution;
  media: MediaPreference;
  colorSpace: ViewerColorSpace;
  view: ViewOverride | null;
  playEveryFrame: boolean;
  edr: boolean;
  viewSeq: number;
};

export type FrameRelease = { poolIndex: number; seq: number };

/** Messages the presenter consumes from the engine connection. */
export type EngineFrameMessage =
  | { type: 'frame'; frame: FrameDescriptor }
  | { type: 'surfacePool'; pool: SurfacePool }
  | { type: 'clock'; sample: ClockSample };

/** Whether a clock sample's line is moving. */
export function running(sample: ClockSample | null): boolean {
  return !!sample && sample.state !== 'stopped' && sample.rate.num !== 0;
}

/**
 * Media ticks at host µs `at` on a sample's line (contract 6
 * `ClockSample::media_time_at`), wrapped into the loop range. Float math:
 * exact enough for presentation decisions (well under a microsecond).
 */
export function mediaTimeAt(sample: ClockSample, at: number): number {
  if (!running(sample)) return sample.mediaTime;
  const elapsedUs = at - sample.hostTime;
  let t = sample.mediaTime + Math.floor((elapsedUs * TICKS_PER_SECOND * sample.rate.num) / (1_000_000 * sample.rate.den));
  const loop = sample.loopRange;
  if (loop && loop.end > loop.start) {
    const length = loop.end - loop.start;
    t = loop.start + ((((t - loop.start) % length) + length) % length);
  }
  return t;
}
