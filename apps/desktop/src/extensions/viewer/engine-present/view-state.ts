/*
 * The viewer state the engine renders for (contract 2 `ViewerState`;
 * plan section 18 "View override"): canvas size in device pixels, device
 * pixel ratio, zoom, pan, preview resolution, media preference, canvas
 * color space, the view override (editor camera, shading, X-ray, Local
 * View), Play Every Frame, whether the display shows EDR values, and the
 * view sequence each descriptor echoes.
 */
import type { EditorCamera, MediaPreference, PreviewResolution, Shading3d, ViewerCanvas, ViewerColorSpace, ViewerState } from './protocol';

export type ViewInput = {
  composition: string;
  /** Canvas size in CSS pixels. */
  cssWidth: number;
  cssHeight: number;
  devicePixelRatio: number;
  /** CSS pixels per composition pixel. */
  zoom: number;
  /** CSS pixels. */
  pan: [number, number];
  resolution: PreviewResolution;
  media: MediaPreference;
  colorSpace: ViewerColorSpace;
  edr: boolean;
  playEveryFrame: boolean;
  /** The 3D viewport's view override; absent for 2D compositions. */
  camera?: EditorCamera | null;
  shading?: Shading3d;
  xray?: boolean;
  localView?: string[];
  /** The 3D chrome wants each frame's depth and object-id surface. */
  depthId?: boolean;
  /** Today's viewer canvas for this view (`todaysCanvas`, decision V1). */
  canvas?: ViewerCanvas | null;
};

const MAX_EDGE = 16384;

/** Device pixels for a CSS size, clamped to the engine's 1..16384. */
export function deviceSize(cssWidth: number, cssHeight: number, dpr: number): { width: number; height: number } {
  const edge = (css: number): number => Math.min(MAX_EDGE, Math.max(1, Math.round(css * (dpr > 0 ? dpr : 1))));
  return { width: edge(cssWidth), height: edge(cssHeight) };
}

/** A canvas the engine accepts (`ViewerState::validate`). */
function validCanvas(canvas: ViewerCanvas | null | undefined): canvas is ViewerCanvas {
  const rect = (r: ViewerCanvas['crop']) => [r.x, r.y, r.width, r.height].every(Number.isFinite) && r.width > 0 && r.height > 0;
  const edge = (v: number) => Number.isInteger(v) && v >= 1 && v <= MAX_EDGE;
  return !!canvas && edge(canvas.width) && edge(canvas.height) && rect(canvas.crop) && rect(canvas.css);
}

function sameState(a: ViewerState, b: ViewerState): boolean {
  const { viewSeq: _a, ...restA } = a;
  const { viewSeq: _b, ...restB } = b;
  return JSON.stringify(restA) === JSON.stringify(restB);
}

/**
 * Numbers viewer states: a state equal to the last one keeps its sequence
 * (and is not sent again); any change takes the next.
 */
export class ViewSequencer {
  private last: ViewerState | null = null;
  private seq = 0;

  /** The state for `input`, and whether it changed since the last call. */
  next(input: ViewInput): { state: ViewerState; changed: boolean } {
    // A 3D composition always carries its shading: Rendered and Material
    // Preview draw Blender surfaces while paused (contract 11), so
    // Rendered is not the same as no override.
    const view =
      input.camera || input.shading || input.xray || input.localView?.length
        ? {
            camera: input.camera ?? null,
            shading: input.shading ?? 'rendered',
            // X-ray applies only in draft shading (contract 2 invariant 6).
            xray: !!input.xray && (input.shading === 'wireframe' || input.shading === 'solid'),
            localView: input.localView ?? []
          }
        : null;
    const candidate: ViewerState = {
      composition: input.composition,
      size: deviceSize(input.cssWidth, input.cssHeight, input.devicePixelRatio),
      devicePixelRatio: input.devicePixelRatio > 0 && Number.isFinite(input.devicePixelRatio) ? input.devicePixelRatio : 1,
      zoom: input.zoom > 0 && Number.isFinite(input.zoom) ? input.zoom : 1,
      pan: [Number.isFinite(input.pan[0]) ? input.pan[0] : 0, Number.isFinite(input.pan[1]) ? input.pan[1] : 0],
      resolution: input.resolution,
      media: input.media,
      colorSpace: input.colorSpace,
      view,
      playEveryFrame: input.playEveryFrame,
      edr: input.edr,
      viewSeq: this.seq,
      ...(input.depthId ? { depthId: true } : {}),
      ...(validCanvas(input.canvas) ? { canvas: { width: input.canvas.width, height: input.canvas.height, crop: { ...input.canvas.crop }, css: { ...input.canvas.css } } } : {})
    };
    if (this.last && sameState(this.last, candidate)) return { state: this.last, changed: false };
    this.seq += 1;
    candidate.viewSeq = this.seq;
    this.last = candidate;
    return { state: candidate, changed: true };
  }

  get current(): ViewerState | null {
    return this.last;
  }
}

type MediaQueryHost = { matchMedia?: (query: string) => { matches: boolean } };

/** The display can show values above 1.0 (CSS `dynamic-range: high`). */
export function displayIsEdr(host: MediaQueryHost = globalThis as MediaQueryHost): boolean {
  return !!host.matchMedia?.('(dynamic-range: high)').matches;
}

/** The canvas color space for the display: display-p3 on wide-gamut displays. */
export function canvasColorSpace(host: MediaQueryHost = globalThis as MediaQueryHost): ViewerColorSpace {
  return host.matchMedia?.('(color-gamut: p3)').matches ? 'displayP3' : 'srgb';
}
