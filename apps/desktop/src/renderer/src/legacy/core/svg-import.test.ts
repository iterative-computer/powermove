import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { Window } from 'happy-dom';
import { materializeSvgPaths, parseSvg, parseSvgPathData,svgImportRepair } from './svg-import';

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
  it('resolves class CSS, inheritance, selector precedence, inline styles and currentColor',()=>{
    const imported=parseSvg(`<svg viewBox="0 0 100 100"><style>.red{fill:#e60012} g .red{fill:#123456} #specific{fill:#abcdef} .forced{fill:red!important}</style><g color="#fedcba"><path class="red" id="specific" d="M0 0H20V20Z"/><path class="red" style="fill:#ffffff" d="M25 0H45V20Z"/><path fill="currentColor" d="M50 0H70V20Z"/><path class="forced" style="fill:blue" d="M75 0H95V20Z"/></g></svg>`);
    expect(imported.paths.map(p=>p.fill)).toEqual(['#ABCDEF','#FFFFFF','#FEDCBA','#FF0000']);expect(imported.warnings).toEqual([]);
  });
  it('retains compound element identity and SVG fills on open contours',()=>{
    const imported=parseSvg(`<svg viewBox="0 0 100 100"><path fill="#e60012" fill-rule="evenodd" d="M0 0H40V40H0Z M10 10H30V30H10Z"/><path fill="#e60012" d="M50 0H90V40H50"/></svg>`);
    expect(imported.paths[0]!.fillGroup).toBe(imported.paths[1]!.fillGroup);
    expect(imported.paths[2]!.fillGroup).not.toBe(imported.paths[0]!.fillGroup);
    expect(imported.paths[2]).toMatchObject({closed:false,fillEnabled:true,fillOpen:true});
    expect(imported.paths[0]!.fillRule).toBe('evenodd');
  });
  it('imports inherited caps, joins and miter limits as native path channels',()=>{
    const imported=parseSvg(`<svg viewBox="0 0 100 100"><g fill="none" stroke="red" stroke-width="8" stroke-linecap="round" stroke-linejoin="round" stroke-miterlimit="7"><path d="M10 10L50 70L90 10"/></g></svg>`);
    const PM={uid:()=>'',P:(v:unknown)=>({v,kf:[],expr:null})};
    const path=materializeSvgPaths(PM,imported)[0]!;
    expect(path.p.cap.v).toBe('ROUND');expect(path.p.join.v).toBe('round');expect(path.p.miterLimit.v).toBe(7);
  });
  it('repairs only untouched legacy imports and keeps all path/vertex identities',()=>{
    const source=`<svg viewBox="0 0 100 100"><style>.red{fill:#e60012}</style><path class="red" d="M0 0H40V40H0Z M10 10H10V30H30V10Z"/></svg>`;
    let next=0;const PM={uid:()=>String(++next),P:(v:unknown)=>({v,kf:[],expr:null})};
    const paths=materializeSvgPaths(PM,parseSvg(source,{legacy:true}));
    for(const path of paths)for(const key of ['cap','join','miterLimit'])delete path.p[key];
    const layer={d:{paths,svgSourceAsset:'asset'},lock:false};
    const repair=svgImportRepair(layer,source,parseSvg(source))!;
    expect(repair.svgImportVersion).toBe(2);
    expect((repair.paths as any[])[0].p.fill.v).toBe('#E60012');
    expect((repair.paths as any[]).map(p=>p.id)).toEqual(paths.map(p=>p.id));
    expect((repair.paths as any[])[0].vertices).toBe(paths[0]!.vertices);
    const subset={...layer,d:{...layer.d,paths:[paths[1]]}};
    const subsetRepair=svgImportRepair(subset,source,parseSvg(source))!;
    expect((subsetRepair.paths as any[])).toHaveLength(1);expect((subsetRepair.paths as any[])[0].id).toBe(paths[1]!.id);
    expect((subsetRepair.paths as any[])[0].p.fill.v).toBe('#E60012');
    paths[0]!.p.fill.v='#ffffff';expect(svgImportRepair(layer,source,parseSvg(source))).toBeNull();
  });
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

it('separates RGB and alpha for extrusion and other RGB consumers',()=>{
  const parsed=parseSvg(`<svg viewBox="0 0 10 10"><rect width="10" height="10" fill="#12345680" stroke="transparent"/><rect width="5" height="5" fill="transparent"/><rect width="2" height="2" fill="rgba(255,0,0,0.25)"/></svg>`);
  expect(parsed.paths[0]).toMatchObject({fill:'#123456',stroke:'#000000',strokeOpacity:0});
  expect(parsed.paths[0]!.fillOpacity).toBeCloseTo(128/255*100);
  expect(parsed.paths[1]).toMatchObject({fill:'#000000',fillOpacity:0});
  expect(parsed.paths[2]).toMatchObject({fill:'#FF0000',fillOpacity:25});
});
it('skips conditional CSS blocks and consumes statement at-rules independently',()=>{
  const parsed=parseSvg(`<svg viewBox="0 0 10 10"><style>@import url("ignored.css"); .x{fill:red} @media (prefers-color-scheme:dark){.x{fill:white}} @supports (display:grid){.x{fill:blue}} .y{fill:green}</style><rect class="x" width="10" height="10"/><rect class="y" width="2" height="2"/></svg>`);
  expect(parsed.paths.map(p=>p.fill)).toEqual(['#FF0000','#008000']);
});
