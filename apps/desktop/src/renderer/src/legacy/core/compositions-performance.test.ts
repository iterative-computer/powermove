import { describe, expect, it } from 'vitest';
import { install, rootComp, rotateProject } from './compositions';

function fixture(count = 200, layersPerComp = 20, instrument = true) {
  let layerReads = 0;
  const layers = (comp: number) => Array.from({ length: layersPerComp }, (_, index) => {
    const type = index % 3 ? 'solid' : 'precomp';
    const layer = { type, d: { comp: `comp-${(comp + index + 1) % count}` } };
    if (instrument) Object.defineProperty(layer, 'type', { get() { layerReads++; return type; } });
    return layer;
  });
  const comps = Object.fromEntries(Array.from({ length: count - 1 }, (_, index) => [
    `comp-${index + 1}`, { name: `Comp ${count - index - 1}`, layers: layers(index + 1), w: 1920, h: 1080, fps: 30, dur: 10, bg: '#000' },
  ]));
  const PM: any = { proj: { id: 'project', compId: 'comp-0', compName: `Comp ${count}`,
    layers: layers(0), comps, w: 1920, h: 1080, fps: 30, dur: 10, bg: '#000' } };
  install(PM);
  return { PM, layerReads: () => layerReads };
}

/** Original list semantics, including duplicate instances and natural-name ties. */
function referenceList(project: any) {
  if (!project) return [];
  const ids = [project.compId, ...Object.keys(project.comps || {})];
  const get = (id: string) => id === project.compId ? rootComp(project) : project.comps[id];
  const layers = (id: string) => id === project.compId ? project.layers : project.comps[id]?.layers || [];
  return ids.map(id => {
    const comp = get(id);
    let uses = 0;
    for (const owner of ids) for (const layer of layers(owner)) {
      if (layer.type === 'precomp' && layer.d?.comp === id) uses++;
    }
    return { id, name: String(comp.name || 'Comp'), w: comp.w, h: comp.h, fps: comp.fps,
      dur: comp.dur, bg: comp.bg, open: id === project.compId, layers: (comp.layers || []).length, uses };
  }).sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' }));
}

describe('composition listing cost and compatibility', () => {
  it('reads each layer once when listing hundreds of compositions', () => {
    const { PM, layerReads } = fixture();
    expect(PM.Comps.list()).toHaveLength(200);
    expect(layerReads()).toBe(4000);
  });

  it('matches the original list across live edits, natural-name ties and composition rotations', () => {
    const { PM } = fixture(40, 12);
    const names = ['Comp 10', 'comp 2', 'COMP 02', 'Álbum 8', 'album 8', '标题 11', '', 'comp 1.2'];
    let seed = 0x6b17;
    const random = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed; };
    const ids = [PM.proj.compId, ...Object.keys(PM.proj.comps)];
    for (let run = 0; run < 120; run++) {
      const id = ids[random() % ids.length]!;
      const comp = id === PM.proj.compId ? PM.proj : PM.proj.comps[id];
      const name = names[random() % names.length];
      if (id === PM.proj.compId) comp.compName = name; else comp.name = name;
      const layer = comp.layers[random() % comp.layers.length];
      const target = random() % (ids.length + 2);
      layer.d.comp = target < ids.length ? ids[target] : target === ids.length ? 'missing' : 3;
      if (run % 5 === 0) PM.proj = rotateProject(PM.proj, ids[random() % ids.length]!);
      expect(PM.Comps.list()).toEqual(referenceList(PM.proj));
    }
  });

  it('counts repeated instances, self references and unusual identifiers without inherited-key collisions', () => {
    const { PM } = fixture(2, 0);
    PM.proj.compId = '__proto__'; PM.proj.compName = 'Root';
    PM.proj.comps = Object.fromEntries(['constructor', 'toString', 'x'].map(id => [id, { name: id, layers: [] }]));
    PM.proj.layers = [
      { type: 'precomp', d: { comp: 'constructor' } }, { type: 'precomp', d: { comp: 'constructor' } },
      { type: 'precomp', d: { comp: '__proto__' } }, { type: 'precomp', d: { comp: 'missing' } },
      { type: 'solid', d: { comp: 'x' } }, { type: 'precomp', d: {} },
    ];
    PM.proj.comps.x.layers = [{ type: 'precomp', d: { comp: '__proto__' } }, { type: 'precomp', d: { comp: 3 } }];
    expect(PM.Comps.list()).toEqual(referenceList(PM.proj));
    expect(PM.Comps.list().find((comp: any) => comp.id === 'constructor')?.uses).toBe(2);
    expect(PM.Comps.list().find((comp: any) => comp.id === '__proto__')?.uses).toBe(2);
    PM.proj = null;
    expect(PM.Comps.list()).toEqual([]);
  });
});

(process.env.PM_COMPOSITIONS_BENCH === '1' ? it : it.skip)('benchmarks the complete composition list against the original algorithm', () => {
  for (const count of [20, 200, 500]) {
    const { PM } = fixture(count, 20, false);
    const timings: Record<string, number[]> = { before: [], after: [] };
    for (let run = 0; run < 11; run++) for (const name of run % 2 ? ['after', 'before'] : ['before', 'after']) {
      const start = performance.now();
      const list = name === 'before' ? referenceList(PM.proj) : PM.Comps.list();
      if (run >= 2) timings[name]!.push(performance.now() - start);
      expect(list).toHaveLength(count);
    }
    const median = (values: number[]) => values.sort((a, b) => a - b)[Math.floor(values.length / 2)];
    process.stdout.write('COMPOSITION_LIST ' + JSON.stringify({ count, layers: count * 20,
      beforeMs: median(timings.before!), afterMs: median(timings.after!) }) + '\n');
  }
});
