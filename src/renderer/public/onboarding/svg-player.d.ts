export interface OnboardingSvgPlayer {
  readonly renderer: 'svg';
  readonly element: SVGSVGElement;
  readonly engine: any;
  readonly hdrOutput: null | {
    canvas: HTMLCanvasElement;
    evidence: Record<string, unknown>;
    statistics: { frames: number; pixelWidth: number; pixelHeight: number; configuration: Record<string, unknown> };
    present(): Promise<void>;
    probeExtendedScene(): Promise<{ max: number; extended: boolean }>;
    destroy(): void;
  };
  readonly duration: number;
  readonly currentTime: number;
  readonly playing: boolean;
  play(): void;
  pause(): void;
  seek(seconds: number): Promise<void>;
  destroy(): void;
}

export interface OnboardingSvgPlayerOptions {
  svg: SVGSVGElement;
  scene: any;
  baseURL?: string | URL;
  audio?: boolean;
  hdr?: boolean;
  autoplay?: boolean;
  onComplete?: () => void;
  onError?: (error: Error) => void;
}

export const EXPECTED_DURATION: number;
export const SDR_EMISSIVE_GAIN: number;
export const WARM_WHITE_TINT: number;
export function createOnboardingSvgPlayer(options: OnboardingSvgPlayerOptions): Promise<OnboardingSvgPlayer>;
export function pathData(engine: any, layer: any, path: any, time: number): string;
export function pathMatrix(engine: any, layer: any, path: any, time: number): number[];
export function combinedPathMatrix(engine: any, layer: any, path: any, time: number): number[];
export function unmixWhite(color: string): { color: string; opacity: number };
export function tintVisibleGlow(color: string, amount?: number): string;
export function gradientState(engine: any, layer: any, effect: any, time: number): any;
export function sourceGate(engine: any, layer: any, path: any, effect: any, time: number): number;
