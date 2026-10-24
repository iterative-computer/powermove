import { describe, expect, it } from 'vitest';
import { figmaNode, guid, pathBlob } from './__fixtures__/clipboard';
import { makePM } from '../../__tests__/make-pm';
import { applyFigmaPlan, planFigmaImport } from './convert';
import { sanitizeProject } from '../../../core/validate/project';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 800, h: 600, dur: 10 }); PM.time = 3;
  return PM;
}

describe('Figma layer conversion', () => {
  it('preserves nested rotated group geometry, stacking, fill alpha, and one undo/redo', () => {
    const PM = editor();
    const nodes = [
      figmaNode(1, 'GROUP', { name: 'Card', size: { x: 200, y: 100 }, fillPaints: [], transform: { m00: 0, m01: -1, m02: 1200, m10: 1, m11: 0, m12: 500 }, opacity: .5 }),
      figmaNode(2, 'RECTANGLE', { name: 'Bottom', parentIndex: { guid: guid(1), position: '!' }, opacity: .8 }),
      figmaNode(3, 'ELLIPSE', { name: 'Top', parentIndex: { guid: guid(1), position: '"' },
        arcData: { startingAngle: 0, endingAngle: 2 * Math.PI, innerRadius: 0 }, transform: { m00: 1, m01: 0, m02: 100, m10: 0, m11: 1, m12: 40 } }),
    ];
    const before = PM.serialize();
    const plan = planFigmaImport(PM, { nodes, blobs: [] }, PM.time);
    expect(plan.issues).toEqual([]); expect(PM.proj.layers).toHaveLength(0);
    applyFigmaPlan(PM, plan);
    expect(PM.proj.layers.map((layer: any) => layer.name)).toEqual(['Card', 'Top', 'Bottom']);
    const [group, top, bottom] = PM.proj.layers;
    expect(top.group).toBe(group.id); expect(bottom.group).toBe(group.id);
    expect(top.d.shape).toBe('ellipse'); expect(bottom.d.color).toBe('#FF4000FF');
    expect(bottom.from).toBe(3); expect(bottom.dur).toBe(7);
    PM.beginEval(3);
    const m = PM.worldMatrix(bottom, 3);
    // Primitive local bounds are centred; its top-left is at (-50, -30).
    expect(m[0] * -50 + m[2] * -30 + m[4]).toBeCloseTo(450);
    expect(m[1] * -50 + m[3] * -30 + m[5]).toBeCloseTo(200);
    expect(PM.worldOpacity(bottom, 3)).toBeCloseTo(.4);
    expect(PM.hist.undo()).toBe(true); expect(PM.serialize()).toBe(before);
    expect(PM.hist.redo()).toBe(true); expect(PM.proj.layers).toHaveLength(3);
    expect(sanitizeProject(JSON.parse(PM.serialize()).proj).layers[1]!.group).toBe(group.id);
  });
  it('keeps frame backgrounds beneath children and applies frame opacity once', () => {
    const PM = editor();
    const plan = planFigmaImport(PM, { nodes: [figmaNode(1, 'FRAME', { opacity: .5, frameMaskDisabled: true }),
      figmaNode(2, 'RECTANGLE', { parentIndex: { guid: guid(1), position: '!' } })], blobs: [] }, 2);
    expect(plan.issues).toEqual([]); applyFigmaPlan(PM, plan);
    expect(PM.proj.layers.map((l: any) => l.type)).toEqual(['group', 'shape', 'shape']);
    expect(PM.proj.layers[2].name).toBe('Layer 1 background');
    expect(PM.worldOpacity(PM.proj.layers[2], 2)).toBeCloseTo(.5);
  });
  it('imports editable typography with the original baseline, alignment and spacing', () => {
    const PM = editor(); PM.Fonts = { families: ['Arial'] };
    const node = figmaNode(1, 'TEXT', { fontName: { family: 'Arial', style: 'Semi Bold Italic' }, fontSize: 20,
      textData: { characters: 'Hello\nWorld' }, textAlignHorizontal: 'CENTER', letterSpacing: { value: 10, units: 'PERCENT' },
      lineHeight: { value: 30, units: 'PIXELS' }, derivedTextData: { baselines: [{ firstCharacter: 0, endCharacter: 5, position: { x: 0, y: 22 }, width: 50, lineHeight: 30, lineAscent: 20 }] } });
    const plan = planFigmaImport(PM, { nodes: [node], blobs: [] }, 3);
    expect(plan.issues).toEqual([]);
    const layer = plan.layers[0];
    expect(layer.d).toMatchObject({ text: 'Hello\nWorld', font: 'Arial', size: 20, weight: 600, italic: true, tracking: 2, leading: 1.5, align: 'center', boxWidth: 100 });
    expect(layer.p['position.x'].v).toBe(400);
    expect(layer.p['position.y'].v).toBeCloseTo(275.6);
  });
  it('turns vector commands into editable vertices without changing coordinates', () => {
    const PM = editor();
    const plan = planFigmaImport(PM, { nodes: [figmaNode(1, 'VECTOR', { fillGeometry: [{ commandsBlob: 0, windingRule: 'NONZERO' }] })], blobs: [pathBlob()] }, 3);
    expect(plan.issues).toEqual([]);
    expect(plan.layers[0].d.paths[0].vertices.map((v: any) => [v.p.x.v, v.p.y.v])).toEqual([[0, 0], [100, 0], [0, 60]]);
    expect(plan.layers[0].d.paths[0].p.closed.v).toBe(true);
  });
  it.each([
    { type: 'INSTANCE' }, { effects: [{ type: 'DROP_SHADOW' }] }, { mask: true },
    { fillPaints: [{ type: 'GRADIENT_LINEAR' }] }, { strokePaints: [{ type: 'SOLID' }], strokeAlign: 'INSIDE' },
    { type: 'TEXT', fontName: { family: 'Missing font', style: 'Regular' } },
    { type: 'TEXT', textData: { characters: 'Mixed', characterStyleIDs: [0, 1] } },
    { type: 'VECTOR', fillGeometry: [{ commandsBlob: 9 }] },
  ])('requires fallback before applying unsupported features: %j', patch => {
    const PM = editor(); PM.Fonts = { families: ['Arial'] };
    const plan = planFigmaImport(PM, { nodes: [figmaNode(1, 'RECTANGLE', patch as any)], blobs: [] }, 0);
    expect(plan.issues.length).toBeGreaterThan(0);
    expect(() => applyFigmaPlan(PM, plan)).toThrow('fallback');
    expect(PM.proj.layers).toHaveLength(0);
  });
  it('rejects cycles instead of dropping layers silently', () => {
    const PM = editor();
    const nodes = [figmaNode(1, 'GROUP', { parentIndex: { guid: guid(2), position: '!' } }), figmaNode(2, 'GROUP', { parentIndex: { guid: guid(1), position: '!' } })];
    expect(() => planFigmaImport(PM, { nodes, blobs: [] }, 0)).toThrow();
  });
});
