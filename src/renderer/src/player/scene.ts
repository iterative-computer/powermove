import type { Project } from '../core/types/project';

export interface WebScene {
  format: 'powermove-web';
  version: 1;
  project: Project;
  assets: Record<string, string>;
  fonts: Array<{ family: string; src: string; weight: string; style: string; unicodeRange?: string }>;
  effects: any[];
  transitions: any[];
  layerTypes: any[];
  warnings: string[];
}

export function validateScene(scene: WebScene): void {
  if (scene?.format !== 'powermove-web' || scene.version !== 1) throw new Error('Unsupported Powermove web scene version');
  const p = scene.project;
  if (!p || !Array.isArray(p.layers) || ![p.w, p.h, p.fps, p.dur].every(n => Number.isFinite(n) && n > 0)
    || p.w > 16384 || p.h > 16384) throw new Error('Invalid composition dimensions, duration, or frame rate');
}
