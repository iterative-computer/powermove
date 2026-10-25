import { animationProjects } from './animation';
import type { CorpusProject } from './common';
import { compositingProjects } from './compositing';
import { compcloseProjects } from './compclose';
import { compclose3Projects } from './compclose3';
import { compositeProjects } from './composite';
import { effectProjects } from './effects';
import { layerProjects } from './layers';
import { motionProjects } from './motion';
import { renderProjects } from './render';
import { renderEffectProjects } from './render-effects';
import { shapeProjects } from './shapes';
import { spaceProjects } from './space';
import { textProjects } from './text';
import { textLevel2Projects } from './text-level2';
import { timeProjects } from './time';

export type { CorpusProject } from './common';
export { textLevel2Projects };

/** Every synthetic corpus project, including native level 2 typography,
 * in a stable order. Level 2 pixels also have a dedicated CoreText gate. */
export function allProjects(): CorpusProject[] {
  const projects = [
    ...layerProjects(), ...shapeProjects(), ...textProjects(), ...textLevel2Projects(), ...animationProjects(),
    ...compositingProjects(), ...effectProjects(), ...timeProjects(), ...spaceProjects(), ...renderProjects(), ...renderEffectProjects(), ...compositeProjects(), ...motionProjects(), ...compcloseProjects(), ...compclose3Projects(),
  ];
  const ids = new Set<string>();
  for (const item of projects) {
    if (ids.has(item.id)) throw new Error(`Duplicate corpus project id ${item.id}`);
    ids.add(item.id);
    if (!item.samples.length) throw new Error(`${item.id} has no sample times`);
    for (const t of item.samples) if (!(t >= 0 && t <= item.doc.dur)) throw new Error(`${item.id} samples ${t}s outside its ${item.doc.dur}s duration`);
  }
  return projects;
}
