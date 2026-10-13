/* The font registry export, exercised from JavaScript as Electron main uses
   it: enumeration, project fonts, registry answers for pm-wasm's queries,
   subsets, the fallback manifest, errors, and main-thread cost.

   Usage: bun scripts/fonts-smoke.mjs [addon]
          ELECTRON_RUN_AS_NODE=1 <Electron> scripts/fonts-smoke.mjs [addon]
   (build the addon first: scripts/build.sh) */
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const addon = process.argv[2] ?? join(here, '../../../target/pm-bridge/pm_bridge.node');
const { FontRegistry } = createRequire(import.meta.url)(addon);
const runtime = globalThis.Bun ? `Bun ${Bun.version}` : `Node ${process.versions.node}${process.versions.electron ? `, Electron ${process.versions.electron}` : ''}`;

let checks = 0;
const check = (name, ok, detail = '') => {
  if (!ok) throw new Error(`${name} failed ${detail}`);
  checks++;
  console.log(`ok ${name}`);
};
const throws = (fn) => { try { fn(); return false; } catch { return true; } };
const bits = (n) => { const v = new DataView(new ArrayBuffer(4)); v.setFloat32(0, n); return v.getUint32(0); };
const family = (name, weight = 400, italic = false, size = 16) => ({ kind: 'family', name, weight: bits(weight), italic, size: bits(size) });
const ms = (t) => Math.round((performance.now() - t) * 100) / 100;

let t = performance.now();
const r = new FontRegistry();
const timings = { create: ms(t) };

t = performance.now();
const names = r.familyNames();
timings.familyNames = ms(t);
check('family names', names.includes('Helvetica Neue') && names.includes('Georgia'), `${names.length}`);
const info = r.familyInfo('Helvetica Neue');
check('family info', info && info.styles.length >= 8 && !info.project && typeof info.styles[0].weight === 'number');
check('unknown family', r.familyInfo('No Such Family 7f3a') === null);

// Answers in pm-text's serde shape, with the generation they were given under.
t = performance.now();
const reply = JSON.parse(r.answer(JSON.stringify([family('Helvetica Neue'), family('Helvetica Neue', 700), family('No Such Family 7f3a')])));
timings.answerFamilies = ms(t);
const face = (q) => reply.answers.find((a) => a.query.name === q.name && a.query.weight === q.weight)?.face;
const regular = face(family('Helvetica Neue'));
const bold = face(family('Helvetica Neue', 700));
check('family answers', regular != null && bold != null && regular !== bold && face(family('No Such Family 7f3a')) == null, JSON.stringify(reply.answers));
check('answer faces', reply.faces.length === 2 && reply.faces.every((f) => typeof f.blob === 'string' && f.blob.length === 32 && f.postscriptName), JSON.stringify(reply.faces.map((f) => f.postscriptName)));
check('answer generation', reply.generation === r.generation);

// A fallback query the layout makes for a character the primary lacks.
t = performance.now();
const fallback = JSON.parse(r.answer(JSON.stringify([{ kind: 'fallback', weight: bits(400), italic: false, size: bits(16), ch: 0x4e16, primary: regular, emoji: false }])));
timings.answerFallback = ms(t);
const cjk = fallback.faces.find((f) => f.id === fallback.answers[0].face);
// CoreText's cascade picks the face (it depends on the user's languages).
check('fallback answer', cjk && cjk.id !== regular && fallback.faces.some((f) => f.id === regular), JSON.stringify(cjk?.postscriptName));
check('unknown primary face', throws(() => r.answer(JSON.stringify([{ kind: 'fallback', weight: bits(400), italic: false, size: bits(16), ch: 0x4e16, primary: 1e6, emoji: false }]))));
check('malformed queries', throws(() => r.answer('[{"kind":"nope"}]')) && throws(() => r.answer('not json')));

// Subsets for every face named, and the system UI font (the largest).
const system = JSON.parse(r.answer(JSON.stringify([family('system-ui')])));
check('system UI font', system.faces.length === 1, JSON.stringify(system.answers));
const sizes = {};
for (const f of [...reply.faces, cjk, ...system.faces]) {
  t = performance.now();
  const s = r.subset(f.blob, f.index);
  const took = ms(t);
  t = performance.now();
  r.subset(f.blob, f.index);
  sizes[f.postscriptName] = { bytes: s.length, firstMs: took, cachedMs: ms(t) };
  check(`subset ${f.postscriptName}`, Buffer.isBuffer(s) && s.length > 1000 && [0x00010000, 0x4f54544f, 0x74727565].includes(s.readUInt32BE(0)));
}
check('unknown blob', throws(() => r.subset('0'.repeat(32), 0)));

// Project fonts shadow installed families and bump the generation.
const georgia = readFileSync('/System/Library/Fonts/Supplemental/Georgia.ttf');
const generation = r.generation;
r.addProjectFont('Chalkduster', georgia, { weight: '400', style: 'normal' });
check('generation bumps', r.generation > generation);
const shadowed = JSON.parse(r.answer(JSON.stringify([family('Chalkduster')])));
check('project font shadows', shadowed.faces[0]?.postscriptName === 'Georgia' && shadowed.faces[0].custom, JSON.stringify(shadowed.faces[0]?.postscriptName));
check('project family listed', r.familyInfo('Chalkduster')?.project === true);
check('invalid descriptors', throws(() => r.addProjectFont('X', georgia, { weight: 'heavy' })));
check('not a font', throws(() => r.addProjectFont('X', new Uint8Array([1, 2, 3, 4]))));
r.clearProjectFonts();
const restored = JSON.parse(r.answer(JSON.stringify([family('Chalkduster')])));
check('project fonts cleared', restored.faces[0]?.postscriptName !== 'Georgia');

// Level 2 inputs.
t = performance.now();
const manifest = JSON.parse(r.fallbackManifest('system-ui'));
timings.fallbackManifest = ms(t);
check('fallback manifest', manifest.scripts.some((s) => s.script === 'Hani') && manifest.emoji === 'Apple Color Emoji' && manifest.faces.length > 10);
const faces = JSON.parse(r.familyFaces('Georgia'));
check('family faces', faces.length === 4 && faces.every((f) => f.blob && f.postscriptName));

console.log(JSON.stringify({ runtime, checks, timings, subsets: sizes }, null, 2));
