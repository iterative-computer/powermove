/** A portable Powermove scene, as written by the desktop app's web export (`scene.json`). */
export interface WebScene {
  format: 'powermove-web';
  version: 1;
  project: any;
  assets: Record<string, string>;
  fonts: Array<{ family: string; src: string; weight: string; style: string; unicodeRange?: string }>;
  effects: any[];
  transitions: any[];
  layerTypes: any[];
  warnings: string[];
}

export interface PlayerOptions {
  canvas: HTMLCanvasElement;
  /** A scene object, or a URL to a scene JSON file (resolved against `baseURL`). */
  scene: string | URL | WebScene;
  baseURL?: string | URL;
  loop?: boolean;
  autoplay?: boolean;
  audio?: boolean;
  /** Ignore the composition background, preserving layer alpha. */
  transparent?: boolean;
  onError?: (error: Error) => void;
}

export interface PowermovePlayer {
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

/** Create a WebGL2 player bound to `options.canvas`. Rejects if WebGL2, fonts, or media are unavailable. */
export function createPlayer(options: PlayerOptions): Promise<PowermovePlayer>;

/**
 * A private scene-evaluation engine (property evaluation, easing, hierarchy, content resolution).
 * Never installs window.PM or editor UI and does not create a canvas. The engine is the desktop
 * app's loosely typed legacy core, so it is exposed as `any`.
 */
export function createEngine(project: any): any;
