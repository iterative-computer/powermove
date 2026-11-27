/** Supplement the existing shader collector without editing or executing
 * installed sources outside its read-only activation sandbox.
 * bun .../supplement.ts <base corpus directory> <output directory> [extensions root]
 * Output must be outside the repository. stdout contains counts only.
 * The additional variants are compilation/draw smoke inputs, not API traces.
 */
import { readdir, readFile, mkdir, writeFile, realpath, copyFile } from 'node:fs/promises';
import { join, resolve, sep } from 'node:path';
import { activateModule } from '../../pm-render/tools/shaders/activate';
import { PREAMBLES, REPO, snapshots } from '../../pm-render/tools/effects/preambles';

const [baseArg, outArg, extensions] = process.argv.slice(2);
if (!baseArg || !outArg) throw new Error('base corpus and output directories required');
const base = await realpath(baseArg), out = resolve(outArg);
const repo = await realpath(REPO);
await mkdir(out, { recursive: true });
const realOut = await realpath(out);
if (realOut === repo || realOut.startsWith(repo + sep)) throw new Error('private corpus output must be outside the repository');
const corpus = JSON.parse(await readFile(join(base, 'corpus.json'), 'utf8'));
const existing = new Set<string>(corpus.cases.map((c: any) => c.frag));
const extra: any[] = [];
function append(def: any, kind: string, group: string, isPrivate: boolean) {
  const snapshot = kind === 'effect' ? snapshots.effectSnapshot(def, 'local', PREAMBLES)
    : kind === 'transition' ? snapshots.transitionSnapshot(def, 'local', PREAMBLES)
    : snapshots.layerSnapshot(def, 'local', PREAMBLES);
  if (!snapshot.glsl || existing.has(snapshot.glsl.source)) return;
  existing.add(snapshot.glsl.source);
  const seed = corpus.cases.find((c: any) => c.kind === (kind === 'layer' ? 'extension' : kind));
  if (!seed) throw new Error('base corpus lacks the required smoke variant');
  const variant = structuredClone(seed.variants[0]);
  const pass = structuredClone(variant.passes[0]);
  variant.passes = Array.from({ length: snapshot.flags.passes || 1 }, () => structuredClone(pass));
  extra.push({ id: `supplement/${extra.length}`, group, kind: seed.kind, private: isPrivate,
    label: 'Supplementary shader', source: 'supplementary input', definition: snapshot,
    frag: snapshot.glsl.source, lineOffset: snapshot.glsl.bodyLineOffset, variants: [variant] });
}
// Schema validation can reject a registration before its GLSL is inventoried.
// Probe that GLSL without modifying the author's text or validating its UI schema.
const generated = join(REPO, 'engine/crates/pm-render/tools/shaders/generated');
for (const name of (await readdir(generated)).filter(n => n.endsWith('.json')).sort()) {
  const input = JSON.parse(await readFile(join(generated, name), 'utf8'));
  if (input.kind === 'extension') append(input.definition, 'layer', 'schema-rejected', false);
}
// The original collector only visits index.ts. Respect explicit manifest JS entries too.
if (extensions) {
  for (const directory of await readdir(extensions, { withFileTypes: true })) {
    if (!directory.isDirectory()) continue;
    const dir = join(extensions, directory.name);
    if (await Bun.file(join(dir, 'index.ts')).exists()) continue;
    if (!await Bun.file(join(dir, 'manifest.json')).exists()) continue;
    const manifest = JSON.parse(await readFile(join(dir, 'manifest.json'), 'utf8'));
    if (typeof manifest.entry !== 'string') continue;
    const captured = await activateModule(join(dir, manifest.entry));
    for (const registration of captured.registered) append(registration.def, registration.kind, 'local-extra', true);
  }
}
for (const image of corpus.images) {
  const target = resolve(out, image.file);
  if (!target.startsWith(out + sep)) throw new Error('unsafe corpus image path');
  await mkdir(resolve(target, '..'), { recursive: true });
  await copyFile(join(base, image.file), target);
}
corpus.cases = extra;
// Never propagate a collector's private diagnostic strings to aggregate output.
corpus.notes = [];
await writeFile(join(out, 'corpus.json'), JSON.stringify(corpus));
const counts: Record<string, number> = {};
for (const c of extra) counts[c.group] = (counts[c.group] ?? 0) + 1;
console.log(JSON.stringify(counts));
