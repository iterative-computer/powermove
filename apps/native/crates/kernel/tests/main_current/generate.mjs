// Writes projects and profile records exactly as the shipped app at current
// main saves them (see main-ref.mjs). Usage: bun generate.mjs [--check]
// --check regenerates into a scratch folder and fails on any byte drift.
// Content is synthetic: invented names, a generated 2x2 PNG, no user data.
import {mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {deflateSync} from 'node:zlib';
import {MAIN, here, editor, load, writePmv4, fileDocument} from './main-ref.mjs';

const target = path.join(here, '../fixtures/main-current');
const check = process.argv.includes('--check');
const out = check ? mkdtempSync(path.join(tmpdir(), 'pm-main-current-')) : target;
mkdirSync(out, {recursive: true});

const {convertFigma} = await load('shared/figma/convert.ts');
const {allFeatures} = await load('shared/figma/__fixtures__/all-features.ts');
const {materializeFigma, applyFigmaPlan} = await load('renderer/src/legacy/core/figma/convert.ts');
const {makeAnimator, ANIMATOR_PRESETS, setAnimatorMode} = await load('extensions/inspector/text-animators.ts');
const {applyMemoryOperation} = await load('renderer/src/core/memory.ts');
const {createStore} = await load('main/storage.ts');
const {StorageUpgrade} = await load('main/storage-upgrade.ts');

/** A deterministic 2x2 RGBA PNG (owned, embedded media). */
function png() {
  const crc = buf => { let c = ~0; for (const b of buf) { c ^= b; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; } return ~c >>> 0; };
  const chunk = (type, data) => { const b = Buffer.alloc(12 + data.length); b.writeUInt32BE(data.length); b.write(type, 4, 'ascii'); data.copy(b, 8); b.writeUInt32BE(crc(b.subarray(4, 8 + data.length)), 8 + data.length); return b; };
  const head = Buffer.alloc(13); head.writeUInt32BE(2); head.writeUInt32BE(2, 4); head.set([8, 6, 0, 0, 0], 8);
  const rows = Buffer.from([0, 255, 64, 0, 255, 0, 128, 255, 255, 0, 255, 255, 255, 255, 0, 0, 0, 255]);
  return new Uint8Array(Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', head), chunk('IDAT', deflateSync(rows, {level: 9})), chunk('IEND', Buffer.alloc(0))]));
}

// A profile (userData) written by main's FileStore as the editor works.
const profile = path.join(out, 'profile');
rmSync(profile, {recursive: true, force: true});
const disk = createStore(path.join(profile, 'store'), {writeDelayMs: 0, now: () => Date.UTC(2026, 10, 3), randomSuffix: () => 'fixture'});
await disk.load();
const P = editor({name: 'Main current', w: 640, h: 360, fps: 30, dur: 4}, disk);
const edit = (...commands) => {
  const result = P.Edit.apply(commands.length === 1 ? commands[0] : commands);
  if (!result.ok) throw new Error(`${commands.map(c => c.type)}: ${result.message ?? result.error}`);
  return result;
};
const id = name => P.proj.layers.find(l => l.name === name).id;
const mutate = (label, action) => {
  const result = P.Edit.mutate(label, action);
  if (!result.ok) throw new Error(`${label}: ${result.message ?? result.error}`);
};
const add = (layerType, name, extra = {}) => { edit({type: 'add_layer', layerType, name, from: 0, duration: 4, ...extra}); return id(name); };

// Media: one owned image (embedded range) and one linked original (sourcePath,
// no range: ordinary native saves keep the link, bf26d9b3e).
const image = png();
// Import writes asset records outside Undo, in `assetIdentity`'s shape
// (legacy/gl/raster.ts importAsset).
const identity = (id, name, kind, extra) => ({id, name, kind, fingerprint: `fp-${id}`, storageKey: `media:${id}`, ...extra,
  channels: 0, sampleRate: 0, playbackProxy: false, playbackProxyVersion: 0, persisted: true});
P.proj.assets.owned = identity('owned', 'Swatch.png', 'image', {size: image.length, dur: 0, w: 2, h: 2, format: 'png'});
P.proj.assets.linked = {...identity('linked', 'Linked clip.mov', 'video', {sourcePath: '/Volumes/Synthetic/Linked clip.mov',
  size: 4096, dur: 2, w: 1920, h: 1080, fps: 30}), hasAudio: false, audioDur: 0, futureAssetField: {kept: true}};
const picture = add('image', 'Owned image', {content: {asset: 'owned'}});
add('video', 'Linked video', {content: {asset: 'linked'}});

// Effect revisions: current, pinned legacy revision 1, a future revision and an
// effect type no installed extension provides (Effect.version, bf26d9b3e).
const shape = add('shape', 'Effected shape');
edit({type: 'add_effect', target: shape, effect: 'blur'},
  {type: 'add_effect', target: shape, effect: 'blur', version: 1},
  {type: 'add_effect', target: shape, effect: 'shadow'},
  {type: 'add_effect', target: shape, effect: 'gradient', version: 1});
mutate('Future effects', () => {
  const layer = P.proj.layers.find(l => l.id === shape);
  layer.fx[0].version = 99; // a revision newer than this build knows
  layer.fx.push({id: 'fx-future', type: 'futureEffect', version: 3, on: true, open: true,
    p: {amount: {v: 0.25, kf: [{t: 1, v: 0.5, i: 'k-future'}], expr: null}}, futureEffectField: [1, 2]});
});

// Effect inputs (stage/space, path, asset), effectSpace and the four-input limit.
const source = add('shape', 'Input source');
const pathOwner = add('shape', 'Path owner');
const inputs = add('shape', 'Input consumer');
edit({type: 'add_effect', target: inputs, effect: 'displacementMap'},
  {type: 'add_effect', target: inputs, effect: 'stroke'},
  {type: 'add_effect', target: inputs, effect: 'texturize'},
  {type: 'add_effect', target: inputs, effect: 'setChannels'});
const fx = P.proj.layers.find(l => l.id === inputs).fx;
const pathId = P.proj.layers.find(l => l.id === pathOwner).masks?.[0]?.id ?? 'path-1';
edit({type: 'set_effect', target: inputs, effect: fx[0].id, patch: {inputs: {map: {layerId: source, stage: 'masks', space: 'source'}}}},
  {type: 'set_effect', target: inputs, effect: fx[2].id, patch: {inputs: {map: {layerId: source, stage: 'effects'}, texture: {kind: 'asset', assetId: 'owned'}}}},
  {type: 'set_effect', target: inputs, effect: fx[3].id, patch: {inputs: {redLayer: {layerId: source, stage: 'source', space: 'composition'}}}},
  {type: 'set_layer', target: inputs, patch: {effectSpace: 'source', separateDimensions: false}});
mutate('Path input', () => { fx[1].inputs = {...fx[1].inputs, path: {kind: 'path', layerId: pathOwner, pathId}}; });

// Text timing v2 (stagger animators created by the inspector's presets).
const text = add('text', 'Stagger title', {content: {text: 'Main fixture'}});
mutate('Text animators', () => {
  const layer = P.proj.layers.find(l => l.id === text);
  const rise = makeAnimator(P.P, 'ta-in', 'Rise', ANIMATOR_PRESETS.find(p => p.id === 'rise'), 48);
  const out = makeAnimator(P.P, 'ta-out', 'Fade out', ANIMATOR_PRESETS.find(p => p.id === 'fade'), 48);
  out.direction = 'out'; out.p.delay = P.P(0.25);
  const wave = makeAnimator(P.P, 'ta-wave', 'Wave', ANIMATOR_PRESETS.find(p => p.id === 'custom-stagger'), 48);
  setAnimatorMode(P.P, wave, 'wave');
  layer.d.animators = [rise, out, {...makeAnimator(P.P, 'ta-legacy', 'Animated out', ANIMATOR_PRESETS[0], 48), direction: 'out', legacyStartDuration: 4, anchor: 'out'}, wave];
});

// 3D groups, switch versions and the new blend names.
const a = add('shape', 'Member A'), b = add('shape', 'Member B');
const group = edit({type: 'group_layers', targets: [a, b], name: '3D rig'}).data?.[0]?.id ?? id('3D rig');
edit({type: 'set_layer', target: group, patch: {threeD: true}}, {type: 'set_layer', target: a, patch: {threeD: true}});
for (const [i, blend] of ['colorburn', 'colordodge', 'hardlight', 'exclusion', 'hue', 'saturation', 'color', 'luminosity', 'linearburn'].entries())
  edit({type: 'set_layer', target: add('solid', `Blend ${blend}`, {from: i * 0.1}), patch: {blend}});
// Several track mattes on one layer (multiple mattes, 841dc9dfd).
edit({type: 'set_layer', target: b, patch: {matteSources: [{source: a, mode: 'alpha'}, {source: id('Blend hue'), mode: 'luma-inverted'}]}});
mutate('Soft light v2', () => { const l = P.proj.layers.find(l => l.id === picture); l.blend = 'softlight'; l.blendVersion = 2; });

// Figma editable content (main's own all-features fixture through its importer).
const figma = allFeatures();
figma.nodes = figma.nodes.filter(n => !['Glass effect', 'Texture effect'].includes(n.name));
applyFigmaPlan(P, materializeFigma(P, convertFigma(figma, P.proj), 0));

// Nested composition, library section and project memory (32ffd5432).
const sectionLayers = [structuredClone(P.proj.layers.find(l => l.id === text))];
edit({type: 'create_section', section: {id: 'S-reusable', name: 'Reusable', layers: sectionLayers, tags: ['text'], at: Date.now(), versions: []}},
  {type: 'update_section', sectionId: 'S-reusable', layers: sectionLayers, at: Date.now()});
P.precompose([source], 'Nested inputs');
const precomp = P.proj.layers.find(l => l.type === 'precomp');
edit({type: 'set_layer', target: precomp.id, patch: {continuousRaster: true}});
let memory = [];
let n = 0;
const ctx = {now: Date.UTC(2026, 10, 2), uid: () => `mem-${++n}`, author: 'user'};
for (const op of [{operation: 'add', kind: 'goal', text: 'Keep the authored timing exact'},
  {operation: 'add', kind: 'constraint', text: 'No new colours', why: 'Brand'},
  {operation: 'add', kind: 'open', text: 'Review the nested inputs'}, {operation: 'update', id: 'mem-3', resolved: true}])
  memory = applyMemoryOperation(memory, op, ctx).entries;
// The memory tool writes notes without an Undo entry (assistant/harness.ts);
// the Settings switch then disables memory without deleting it.
P.proj.memory = memory;
P.proj.memoryEnabled = false;

// Unknown fields at every level a future build may add them.
// Unknown fields at every level a future build may add them (written
// directly: by definition no command of this build knows them).
P.proj.futureProjectField = {nested: [1, {deep: 'x'}]};
mutate('Future layer fields', () => {
  P.proj.layers[0].futureLayerField = 'kept';
  P.proj.layers[0].d.futureContentField = 7;
  for (const comp of Object.values(P.proj.comps || {})) comp.futureCompField = true;
});
// Setup steps would make a multi-megabyte history; a saved file carries a
// few ordinary edits instead, one of them undone (a pending redo).
P.proj.edits = [];
P.hist.clear();
// A recovery checkpoint retires the setup journal (projects.ts recoverAsync).
P.Projects.put(P.proj);
P.store.del(P.Projects.JOURNAL + P.proj.id);
edit({type: 'set_layer', target: shape, patch: {name: 'Effected shape (edited)'}});
edit({type: 'set_effect', target: shape, effect: P.proj.layers.find(l => l.id === shape).fx[1].id, patch: {enabled: false}});
edit({type: 'set_layer', target: shape, patch: {name: 'Renamed then undone'}});
P.hist.undo();

if (!P.proj.assets.owned || !P.proj.assets.linked || P.proj.memory?.length !== 3 || !P.proj.futureProjectField)
  throw new Error('fixture setup lost project fields');
for (const [name, value] of Object.entries({
  scene3dDefaultsVersion: P.proj.scene3dDefaultsVersion, switch3DVersion: P.proj.switch3DVersion,
  timingVersion: P.proj.layers.find(l => l.id === text).d.animators[0].timingVersion,
  blendVersion: P.proj.layers.find(l => l.id === picture).blendVersion}))
  if (value !== 2) throw new Error(`main no longer writes ${name}: 2`);

// Project-saved workspace from before dock migration 4 (bottom dock), with a
// custom dock and tab; current main migrates it on restore (4-6).
const ws = {name: 'Custom', agentDockMigration: 3, docks: [{id: 'left', size: 312, tabs: ['layers', 'customPanel'], active: 'customPanel'},
  {id: 'right', size: 280, tabs: ['inspector']}], hiddenPanels: ['mixer'], futureWorkspaceField: 1};
const history = P.hist.export({copy: true});
const document = fileDocument(P.version, P.proj, history, ws);
await writePmv4(path.join(out, 'current.pmv'), document,
  [{id: 'owned', revision: 'rev-owned-1', type: 'image/png', data: image}]);

// Profile: recovery slot, separate session history, journal and takes through
// main's Projects/takes (FileStore writes large records in the gzip
// envelope), then the storage-upgrade checkpoint paused at its browser stage.
P.Projects.put(P.proj);
P.Projects.putState(P.proj.id, {history, workspace: ws, lastActiveAt: Date.UTC(2026, 10, 3)});
P.takes.save('Before review');
// Hosts without separate history keep one projectState record (still read).
P.store.separateHistory = false;
P.Projects.putState(P.proj.id, {history, workspace: ws, lastActiveAt: Date.UTC(2026, 10, 3)});
await disk.flushAll({strict: true});
const upgrade = new StorageUpgrade({root: profile, measure: async () => 1 << 20, exclusive: action => action(),
  recovery: async () => {}, sources: async () => {}, cleanup: async () => {}});
const paused = await upgrade.request('app://powermove', {}, {action: 'start'});
if (paused.state !== 'browser') throw new Error(`storage upgrade did not pause at browser: ${paused.state}`);
for (const key of ['project', 'projectHistory', 'projectState', 'projectJournal'])
  if (!readdirSync(path.join(profile, 'store')).some(f => f.startsWith(key + '.'))) throw new Error(`main wrote no ${key} record`);
writeFileSync(path.join(out, 'README.md'), `# Current-main fixtures

Generated by \`tests/main_current/generate.mjs\` with the shipped app's own code at
main \`${MAIN}\`: commands, Figma importer, memory operations, PMV4 writer and
storage codec. Synthetic content only. Regenerate with \`bun generate.mjs\`;
\`bun generate.mjs --check\` fails on any drift.
`);

if (check) {
  const files = dir => readdirSync(dir, {recursive: true}).filter(f => !f.endsWith('/')).sort();
  const drift = [...new Set([...files(target), ...files(out)])].filter(f => {
    try { return !readFileSync(path.join(target, f)).equals(readFileSync(path.join(out, f))); } catch { return true; }
  }).filter(f => !['profile', 'profile/store'].includes(f));
  rmSync(out, {recursive: true, force: true});
  if (drift.length) throw new Error(`fixtures drift from main: ${drift.join(', ')}`);
}
process.stderr.write(`main-current fixtures ${check ? 'verified' : 'written'} (${MAIN.slice(0, 9)})\n`);
