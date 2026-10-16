import { animationProjects } from './animation';
import { parseRender3D } from '../catalog';
import type { CorpusProject } from './common';
import { compositingProjects } from './compositing';
import { effectProjects } from './effects';
import { layerProjects } from './layers';
import { renderProjects } from './render';
import { renderEffectProjects } from './render-effects';
import { shapeProjects } from './shapes';
import { spaceProjects } from './space';
import { textProjects } from './text';
import { timeProjects } from './time';

export type { CorpusProject } from './common';

/** Every synthetic corpus project, in a stable order. */
export function allProjects(): CorpusProject[] {
  const projects = [
    ...layerProjects(), ...shapeProjects(), ...textProjects(), ...animationProjects(),
    ...compositingProjects(), ...effectProjects(), ...timeProjects(), ...spaceProjects(), ...renderProjects(), ...renderEffectProjects(),
  ];
  const ids = new Set<string>();
  for (const item of projects) {
    if (ids.has(item.id)) throw new Error(`Duplicate corpus project id ${item.id}`);
    ids.add(item.id);
    if (!item.samples.length) throw new Error(`${item.id} has no sample times`);
    for (const t of item.samples) if (!(t >= 0 && t <= item.doc.dur)) throw new Error(`${item.id} samples ${t}s outside its ${item.doc.dur}s duration`);
    if (item.blender) parseRender3D({ ...item.blender, enabled: true });
  }
  return projects;
}
