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
  /** A scrub frame in which a video layer shows the nearest decoded frame
   *  while its exact frame decodes; the exact frame follows
   *  (docs/engine/scrub.md). */
  standIn?: boolean;
};

export type Scene3dMeta = { composition: string; layers: string[]; warnings: string[]; error: string | null };
export type ChromeSelection = { selected: string[]; active: string[] };
export type ChromeGlyph = { id:string; kind:'point'|'sun'|'spot'|'area'|'camera'; position:[number,number,number]; target:[number,number,number]|null; up:[number,number,number]; fov:number; aspect:number; angle:number; size:[number,number] };
export type DepthIdSurface = { poolIndex:number; seq:number; poolGeneration:number; encoding?:string; coveragePoolIndex?:number; legend?:string[]; glyphs?:ChromeGlyph[]; selection?:ChromeSelection };

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
  /** The frame-aligned lossless depth and ID/selection pair in the depth
   *  pool (`chromePlanesV1`), when the viewer asked for it. */
  depthId: DepthIdSurface | null;
  scene3dMeta?: Scene3dMeta[];
  viewSeq: number;
  /** Ticks; `null` for frames shown at the next refresh. */
  duration: number | null;
};

export type SurfacePool = {
  generation: number;
  surfaces: { poolIndex: number; size: { width: number; height: number }; format: SurfaceFormat }[];
  /** `depthId`: the depth and object-id pool, never the frames' pool. */
  role?: 'color' | 'depthId' | null;
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
  /** The display's refresh grid (amendment 18); a change alone keeps `viewSeq`. */
  vsync?: VsyncHint | null;
  /** Ask for each frame's depth and object-id surface (the 3D chrome). */
  depthId?: boolean | null;
  chromeSelection?: ChromeSelection;
};

/** Refreshes (BeginFrame times) fall at `hostTime + n × periodUs` (host µs). */
export type VsyncHint = { hostTime: number; periodUs: number };

export type FrameRelease = { poolIndex: number; seq: number };

/** Messages the presenter consumes from the engine connection. */
export type EngineFrameMessage =
  | { type: 'frame'; frame: FrameDescriptor }
  | { type: 'surfacePool'; pool: SurfacePool }
  | { type: 'clock'; sample: ClockSample };

/**
 * What the engine connection tells the presentation besides frames: the
 * engine cannot draw the composition (capability routing, plan section 21;
 * `reason` names what), so its frames leave the screen until the engine
 * draws again. An exclusive presentation ("Use New Engine" on, amendment
 * 21) shows the reason on the stage; nothing hands the stage to the
 * current engine. `seq` is the newest frame seq the engine had given the
 * view when it decided: frames at or below it are stale.
 */
export type EngineRouteMessage = { type: 'routed'; route: 'legacy'; reason: string; revision?: number; seq?: number };

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
