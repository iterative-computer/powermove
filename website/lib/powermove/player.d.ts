export interface PlayerOptions {
  canvas: HTMLCanvasElement;
  scene: string | URL | object;
  baseURL?: string | URL;
  loop?: boolean;
  autoplay?: boolean;
  audio?: boolean;
  transparent?: boolean;
  onError?: (error: Error) => void;
}
export interface PowermovePlayer {
  readonly duration: number;
  readonly currentTime: number;
  readonly playing: boolean;
  readonly parameters: Record<string, { value: unknown; control: string; label?: string }>;
  loop: boolean;
  play(): void;
  pause(): void;
  seek(seconds: number): Promise<void>;
  setParameter(name: string, value: unknown): Promise<void>;
  setText(layerId: string, text: string): Promise<void>;
  destroy(): void;
}
export function createPlayer(options: PlayerOptions): Promise<PowermovePlayer>;
/** Scene evaluator exported by the original runtime; does not create a canvas. */
export function createEngine(project: any): any;
