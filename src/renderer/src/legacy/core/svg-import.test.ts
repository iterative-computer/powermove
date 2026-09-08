import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { Window } from 'happy-dom';
import { materializeSvgPaths, parseSvg, parseSvgPathData } from './svg-import';

const previousDOMParser = globalThis.DOMParser;

beforeAll(() => {
  globalThis.DOMParser = new Window().DOMParser as unknown as typeof DOMParser;
});

afterAll(() => {
  globalThis.DOMParser = previousDOMParser;
});

const logo = `<svg width="141" height="116" viewBox="0 0 141 116" fill="none" xmlns="http://www.w3.org/2000/svg">
<path d="M33.3885 106.544L5.19736 115.218C1.26974 116.426 -1.70853 111.636 1.11257 108.648L28.2054 79.9531C42.9061 64.3828 53.2117 45.1902 58.0699 24.3349L63.0183 3.09249C63.9876 -1.06813 69.929 -1.01597 70.8251 3.16102L74.1448 18.6367C79.385 43.0646 92.058 65.2717 110.424 82.2092L138.829 108.405C141.918 111.254 139.086 116.33 135.039 115.198L101.016 105.677C78.8565 99.7767 55.3816 99.7767 33.3885 106.544Z" fill="white"/>
</svg>`;

describe('SVG shape import', () => {
  it('turns the supplied Powermove logo into centered editable Bezier vertices', () => {
    const imported = parseSvg(logo);

    expect(imported).toMatchObject({ width: 141, height: 116, warnings: [] });
    expect(imported.paths).toHaveLength(1);
    const path = imported.paths[0]!;
    expect(path).toMatchObject({ closed: true, fill: '#FFFFFF', fillEnabled: true, strokeWidth: 0 });
    expect(path.vertices.length).toBeGreaterThan(8);
    expect(path.vertices.some(point => point.inX !== 0 || point.outX !== 0)).toBe(true);
    const xs = path.vertices.map(point => point.x);
    const ys = path.vertices.map(point => point.y);
    expect(Math.min(...xs)).toBeLessThan(-69);
    expect(Math.max(...xs)).toBeGreaterThan(68);
    expect(Math.min(...ys)).toBeLessThan(-54);
    expect(Math.max(...ys)).toBeGreaterThan(56);
  });

  it('supports relative, smooth, quadratic, and elliptical-arc path commands', () => {
    const paths = parseSvgPathData('m10 10 h20 v10 q10 10 20 0 t20 0 c5 0 5 10 10 10 s5 -10 10 -10 a10 5 30 0 1 20 10 z');

    expect(paths).toHaveLength(1);
    const path = paths[0]!;
    expect(path.closed).toBe(true);
    expect(path.vertices.length).toBeGreaterThanOrEqual(9);
    expect(path.vertices.every(point => Object.values(point).every(Number.isFinite))).toBe(true);
  });

  it('bakes nested SVG transforms into editable geometry and materializes unique native channels', () => {
    const imported = parseSvg(`<svg width="100" height="100" viewBox="0 0 100 100"><g transform="translate(10 20) scale(2)"><rect id="card" x="5" y="5" width="10" height="20" fill="#f00"/></g></svg>`);
    let counter = 0;
    const PM = { uid: (prefix: string) => `${prefix}-${++counter}`, P: (value: unknown) => ({ v: value, kf: [], expr: null }) };
    const first = materializeSvgPaths(PM, imported);
    const second = materializeSvgPaths(PM, imported);

    const firstPath = first[0]!, secondPath = second[0]!;
    expect(firstPath.name).toBe('card');
    expect(firstPath.p.fill.v).toBe('#FF0000');
    expect(firstPath.vertices.map(vertex => [vertex.p.x.v, vertex.p.y.v])).toEqual([[-30, -20], [-10, -20], [-10, 20], [-30, 20]]);
    expect(secondPath.id).not.toBe(firstPath.id);
    expect(secondPath.vertices[0]!.id).not.toBe(firstPath.vertices[0]!.id);
  });
});
