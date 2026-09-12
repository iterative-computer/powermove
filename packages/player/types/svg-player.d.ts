import type { WebScene } from './player';

export interface SvgPlayerOptions {
  /** The target `<svg>`; the player sets its viewBox and appends one `<g>` it owns. */
  svg: SVGSVGElement;
  scene: WebScene;
  /** Base for resolving `scene.fonts[].src`; defaults to `document.baseURI`. */
  baseURL?: string | URL;
  autoplay?: boolean;
  /** Defaults to true. */
  loop?: boolean;
  /** Skip the composition background rect. */
  transparent?: boolean;
  onError?: (error: Error) => void;
}

export interface SvgPlayer {
  readonly renderer: 'svg';
  readonly element: SVGSVGElement;
  readonly duration: number;
  readonly currentTime: number;
  readonly playing: boolean;
  readonly parameters: Record<string, { value: unknown; control: string; label?: string; [key: string]: unknown }>;
  loop: boolean;
  play(): void;
  pause(): void;
  seek(seconds: number): Promise<void>;
  setParameter(name: string, value: unknown): Promise<void>;
  setText(layerId: string, text: string): Promise<void>;
  destroy(): void;
}

/**
 * Render a scene as native SVG. Only a drawing backend: animation and hierarchy evaluation is shared
 * with the WebGL player. Supports text, solid and shape layers plus affine groups; rejects scenes the
 * SVG backend cannot represent (see the desktop app's `svg-compatibility.ts`). Loads fonts before resolving.
 */
export function createSvgPlayer(options: SvgPlayerOptions): Promise<SvgPlayer>;
