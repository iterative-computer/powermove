// Regenerate with: bun engine/crates/pm-geom3d/tests/fixtures/generate-scene-validation.ts
// These are synthetic scenes. The acceptance oracle is today's parseScene.
import { parseScene } from '../../../../../apps/desktop/src/renderer/src/core/scene3d/schema';

type Scene = Record<string, any>;
const base = (): Scene => ({ objects: [{ id: 'a', source: { primitive: 'box' } }] });
const cases: { name: string; scene: Scene; approved?: boolean }[] = [];
function add(name: string, edit: (scene: Scene) => void, approved = false) {
  const scene = base(); edit(scene); cases.push({ name, scene, approved });
}
const obj = (s: Scene) => s.objects[0];
const material = (s: Scene) => (obj(s).material ??= {});
const shader = () => ({ id: 'mat', name: 'Material', graph: {
  nodes: [{ id: 'out', type: 'ShaderNodeOutputMaterial' }], links: [] as any[]
} });

add('sparse valid', () => {});
add('all defaults valid', s => Object.assign(s, parseScene(s)));
add('valid animated composition channels', s => {
  obj(s).p = { x: { v: 2, kf: [{ t: 0, v: 2, note: 'passthrough' }], expr: null }, visible: { v: true, kf: [], expr: null } };
  material(s).p = { opacity: { v: .5, kf: [{ t: -3, v: .75 }], expr: null } };
  s.lights = [{ id: 'sun', p: { intensity: { v: 3, kf: [{ t: 1, v: 5 }], expr: null } } }];
  s.camera = { p: { near: { v: .01, kf: [], expr: null }, far: { v: 1000, kf: [], expr: null } } };
  s.environment = { p: { ambient: { v: .5, kf: [], expr: null } } };
});
add('valid mesh with groups and slots', s => {
  obj(s).source = { mesh: { positions: [0,0,0, 1,0,0, 0,1,0], indices: [0,1,2], groups: [{ start: 0, count: 3, materialIndex: 0 }], normals: [0,0,1, 0,0,1, 0,0,1], uvs: [0,0, 1,0, 0,1] } };
  obj(s).slots = [{ id: 'slot', name: 'Slot', material: {} }];
});
add('valid lathe', s => { obj(s).source = { lathe: [[0,0],[1,1]], segments: 8 }; });
add('valid extrude', s => { obj(s).source = { extrude: [[0,0],[1,0],[0,1]], depth: 1, bevel: .2 }; });
add('valid asset source and shader graph', s => { obj(s).source = { assetId: 'asset' }; material(s).shader = shader(); });
add('valid shader input', s => {
  const sh = shader(); sh.graph.nodes.push({ id: 'value', type: 'ShaderNodeValue' });
  Object.assign(sh, { inputs: [{ id: 'value', label: 'Value', kind: 'number', node: 'value', socket: 0, min: 0, max: 1, step: .1 }], p: { value: { v: .5, kf: [], expr: null } } });
  material(s).shader = sh;
});

add('scene strict key', s => { s.foreign = true; });
add('object strict key', s => { obj(s).foreign = true; });
add('light strict key', s => { s.lights = [{ id: 'sun', foreign: true }]; });
add('camera strict key', s => { s.camera = { foreign: true }; });
add('environment strict key', s => { s.environment = { foreign: true }; });
add('source strict key', s => { obj(s).source.foreign = true; });
add('primitive params strict key', s => { obj(s).source.parameters = { foreign: true }; });
add('material strict key', s => { material(s).foreign = true; });
add('placement strict key', s => { material(s).placement = { foreign: true }; });
add('maps strict key', s => { material(s).maps = { foreign: 'asset' }; });
add('slot strict key', s => { obj(s).slots = [{ id: 'slot', name: 'Slot', material: {}, foreign: true }]; });
add('shader strict key', s => { material(s).shader = { ...shader(), foreign: true }; });
add('shader graph strict key', s => { const sh = shader(); (sh.graph as any).foreign = true; material(s).shader = sh; });
add('shader node strict key', s => { const sh = shader(); (sh.graph.nodes[0] as any).foreign = true; material(s).shader = sh; });
add('shader property strict key', s => { const sh = shader(); (sh.graph.nodes[0] as any).properties = { foreign: true }; material(s).shader = sh; });

add('object transform range', s => { obj(s).p = { x: 100001 }; });
add('object rotation range', s => { obj(s).p = { rx: 360001 }; });
add('material opacity range', s => { material(s).p = { opacity: 2 }; });
add('material placement range', s => { material(s).placement = { scaleX: 0 }; });
add('light intensity range', s => { s.lights = [{ id: 'sun', p: { intensity: 10001 } }]; });
add('camera fov range', s => { s.camera = { p: { fov: 151 } }; });
add('environment exposure range', s => { s.environment = { p: { exposure: 0 } }; });
add('primitive dimension range', s => { obj(s).source.parameters = { radius: -1 }; });
add('primitive segments integer', s => { obj(s).source.parameters = { segments: 2.5 }; });
add('invalid color', s => { material(s).p = { color: '#GGGGGG' }; });
add('unknown primitive', s => { obj(s).source.primitive = 'knot'; });
add('unknown light type', s => { s.lights = [{ id: 'sun', type: 'laser' }]; });
add('invalid identifier', s => { obj(s).id = 'bad id'; });
add('invalid channel keyframe', s => { obj(s).p = { x: { v: 1, kf: [{ t: -1, v: 1 }] } }; });

add('mesh nontriples', s => { obj(s).source = { mesh: { positions: [0,0,0, 1,0,0, 0,1,0, 2] } }; });
add('mesh index out of bounds', s => { obj(s).source = { mesh: { positions: [0,0,0, 1,0,0, 0,1,0], indices: [0,1,3] } }; });
add('mesh normal count', s => { obj(s).source = { mesh: { positions: [0,0,0, 1,0,0, 0,1,0], normals: [0,0,1] } }; });
add('mesh uv count', s => { obj(s).source = { mesh: { positions: [0,0,0, 1,0,0, 0,1,0], uvs: [0,0] } }; });
add('mesh group range', s => { obj(s).source = { mesh: { positions: [0,0,0, 1,0,0, 0,1,0], groups: [{ start: 0, count: 6, materialIndex: 0 }] } }; });
add('lathe negative radius', s => { obj(s).source = { lathe: [[-1,0],[1,1]] }; });
add('extrude too few points', s => { obj(s).source = { extrude: [[0,0],[1,0]], depth: 1 }; });
add('duplicate IDs', s => { s.lights = [{ id: 'a' }]; });
add('parent cycle', s => { obj(s).parent = 'b'; s.objects.push({ id: 'b', parent: 'a', source: { primitive: 'box' } }); });
add('missing parent', s => { obj(s).parent = 'missing'; });
add('geometry budget', s => { s.objects = Array.from({ length: 9 }, (_, i) => ({ id: `o${i}`, source: { primitive: 'icosahedron', parameters: { detail: 5 } } })); });
add('camera far before near', s => { s.camera = { p: { near: 10, far: 5 } }; });
add('shader missing output', s => { const sh = shader(); sh.graph.nodes[0].type = 'ShaderNodeValue'; material(s).shader = sh; });
add('shader duplicate node', s => { const sh = shader(); sh.graph.nodes.push({ id: 'out', type: 'ShaderNodeValue' }); material(s).shader = sh; });
add('shader link unknown node', s => { const sh = shader(); sh.graph.links.push({ from: 'missing', output: 0, to: 'out', input: 0 }); material(s).shader = sh; });
add('shader duplicate incoming socket', s => { const sh = shader(); sh.graph.links.push({ from: 'out', output: 0, to: 'out', input: 0 }, { from: 'out', output: 1, to: 'out', input: '0' }); material(s).shader = sh; });
add('shader graph cycle', s => { const sh = shader(); sh.graph.nodes.push({ id: 'other', type: 'ShaderNodeValue' }); sh.graph.links.push({ from: 'out', output: 0, to: 'other', input: 0 }, { from: 'other', output: 0, to: 'out', input: 0 }); material(s).shader = sh; });
add('shader missing exposed input', s => { const sh = shader(); Object.assign(sh, { inputs: [{ id: 'value', label: 'Value', kind: 'number', node: 'out', socket: 0 }] }); material(s).shader = sh; });
add('shader unknown exposed input', s => { const sh = shader(); Object.assign(sh, { p: { unknown: { v: 1, kf: [] } } }); material(s).shader = sh; });
add('shader exposed input out of range', s => { const sh = shader(); Object.assign(sh, { inputs: [{ id: 'value', label: 'Value', kind: 'number', node: 'out', socket: 0, min: 0, max: 1 }], p: { value: { v: 2, kf: [] } } }); material(s).shader = sh; });

add('approved long display names', s => { obj(s).name = 'x'.repeat(161); s.lights = [{ id: 'sun', name: 'y'.repeat(161) }]; }, true);
add('approved 513 objects', s => { s.objects = Array.from({ length: 513 }, (_, i) => ({ id: `o${i}`, source: { primitive: 'box' } })); }, true);
add('approved 17 lights', s => { s.lights = Array.from({ length: 17 }, (_, i) => ({ id: `l${i}` })); }, true);
add('approved count plus foreign source', s => { s.objects = Array.from({ length: 513 }, (_, i) => ({ id: `o${i}`, source: { primitive: i === 7 ? 'knot' : 'box' } })); });

const out = cases.map(({ name, scene, approved }) => {
  let parsed = true;
  try { parseScene(scene); } catch { parsed = false; }
  if (approved && parsed) throw new Error(`${name} did not exercise an approved parser cap`);
  return { name, scene, accepted: parsed || approved === true };
});
await Bun.write(`${import.meta.dir}/scene-validation.json`, `${JSON.stringify(out, null, 2)}\n`);
console.log(`${out.length} parseScene oracle vectors; ${out.filter(x => x.accepted).length} accepted`);
