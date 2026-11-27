import cases from '../../fixtures/text-level2.json';
import { newProject } from '../doc';
import { project, type CorpusProject } from './common';

/** Native-only semantics: these documents must never route to the legacy
 * renderer. Shared with the native semantic and app end-to-end tests. */
export function textLevel2Projects(): CorpusProject[] {
  return cases.map(item => {
    const b = newProject(item.id, { name: item.title, w: 640, h: 360, dur: 3 });
    const { id, title, samples, ...content } = item;
    const { d, ...controls } = content;
    const layer = b.add(b.layer('text', { name: title, d: { ...d, ...controls },
      p: { 'position.x': 100, 'position.y': 80 } }));
    for (const key of ['boxWidth', 'boxHeight']) {
      if (typeof layer.d[key] === 'number') layer.d[key] = b.ch(layer.d[key]);
    }
    layer.compat = 2;
    return project(b, { group: 'Text', title, samples, alpha: true,
      covers: ['compat level 2', 'native text layout, drawing, bounds and picks'],
      notes: 'Native-only semantics. The semantic gate compares against CoreText and exact transform invariants; legacy captures are diagnostic, not level 2 references.' });
  });
}
