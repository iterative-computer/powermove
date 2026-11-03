import { encodeProjectContainer, decodeProjectContainer } from '../../../../../shared/project-container';
import { it, expect } from 'vitest';
import { makePM } from '../../__tests__/make-pm';
import { convertFigma } from '../../../../../shared/figma/convert';
import type { FigmaDoc, FigmaNode } from '../../../../../shared/figma/doc';
import { applyFigmaPlan, materializeFigma } from './convert';
import { sanitizeProject } from '../../../core/validate/project';
const n = (id = '1', color = { r: 1, g: 0, b: 0 }): FigmaNode => ({
  id,
  fileKey: 'synthetic-file',
  type: 'RECTANGLE',
  name: 'Figma card',
  w: 100,
  h: 60,
  absoluteTransform: [1, 0, 0, 1, 0, 0],
  fills: [{ type: 'SOLID', color }],
  strokes: [],
  effects: [],
  children: [],
});
const d = (nodes: FigmaNode[]): FigmaDoc => ({
  format: 'powermove-figma',
  version: 1,
  source: 'plugin',
  nodes,
  images: {},
});
function editor() {
  const P = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  P.proj = P.mkProject({ w: 640, h: 360, dur: 8 });
  return P;
}
it('keeps animation, expressions, timing, parent, custom names and effects in one undo step; provenance round-trips', () => {
  const P = editor(),
    plan = (nodes: FigmaNode[]) => materializeFigma(P, convertFigma(d(nodes), P.proj), 0);
  applyFigmaPlan(P, plan([n(), n('missing')]));
  const old = P.proj.layers[1];
  old.name = 'My animation';
  old.from = 2;
  old.dur = 4;
  old.parent = 'user-parent';
  P.setKey(old, 'position.x', 2, 20);
  P.setKey(old, 'position.x', 3, 200);
  old.p['position.y'].expr = 'time*10';
  old.fx = [{ id: 'user-effect', type: 'blur', on: true, p: { amount: P.P(2) } }];
  const before = P.serialize();
  const incoming = n('1', { r: 0, g: 0, b: 1 });
  incoming.w = 180;
  const result = applyFigmaPlan(P, plan([incoming, n('new')]), true);
  const after = P.proj.layers.find((l: any) => l.id === old.id);
  expect(after).toMatchObject({
    id: old.id,
    name: 'My animation',
    from: 2,
    dur: 4,
    parent: 'user-parent',
    d: { w: 180, color: '#0000FFFF' },
  });
  expect(after.p['position.x'].kf).toHaveLength(2);
  expect(after.p['position.y'].expr).toBe('time*10');
  expect(after.fx[0].id).toBe('user-effect');
  expect(P.proj.layers).toHaveLength(3);
  expect(result.missing).toEqual(['Figma card']);
  expect(
    sanitizeProject(
      decodeProjectContainer(encodeProjectContainer(JSON.parse(P.serialize()), [])).document.proj,
    ).layers.find((l) => l.id === old.id)?.figmaSource,
  ).toEqual(after.figmaSource);
  expect(P.hist.undo()).toBe(true);
  expect(P.serialize()).toBe(before);
  expect(P.hist.redo()).toBe(true);
  expect(P.proj.layers.find((l: any) => l.id === old.id).d.color).toBe('#0000FFFF');
});
it('preserves animated content channels and supports a separate copy', () => {
  const P = editor(),
    plan = () => materializeFigma(P, convertFigma(d([n()]), P.proj), 0);
  applyFigmaPlan(P, plan());
  P.proj.layers[0].d.color = { v: '#ff0000', kf: [{ t: 0, v: '#ff0000' }], expr: null };
  applyFigmaPlan(P, plan(), true);
  expect(P.proj.layers[0].d.color.kf).toHaveLength(1);
  applyFigmaPlan(P, plan());
  expect(P.proj.layers).toHaveLength(2);
});
it('places new bottom siblings below matched layers', () => {
  const P = editor(),
    plan = (nodes: FigmaNode[]) => materializeFigma(P, convertFigma(d(nodes), P.proj), 0);
  applyFigmaPlan(P, plan([n()]));
  applyFigmaPlan(P, plan([n('bottom'), n()]), true);
  expect(P.proj.layers.map((l: any) => l.figmaSource.nodeId)).toEqual(['1', 'bottom']);
});
