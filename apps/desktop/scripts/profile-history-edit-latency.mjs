// Compare undo commit latency using a copied project and a history.ts implementation.
// node scripts/profile-history-edit-latency.mjs /path/to/history.ts /path/to/project.json
import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import { existsSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
registerHooks({ resolve(specifier, context, next) {
  if (specifier.startsWith('.') && context.parentURL) {
    const url = new URL(specifier + '.ts', context.parentURL);
    if (existsSync(url)) return next(url.href, context);
  }
  return next(specifier, context);
} });
if (process.argv.length !== 4) throw new Error('Expected history.ts path and copied project JSON');
const { install } = await import(pathToFileURL(process.argv[2]).href);
const source = JSON.parse(readFileSync(process.argv[3], 'utf8'));
const project = source.proj || source;
const results = {};
for (const mode of ['scoped-property', 'structural-layers']) {
  let id = 0;
  const PM = { proj: structuredClone(project), sel: { layers: [], keys: [] }, uid: p => `${p}-${++id}`,
    replaceProject(next) { PM.proj = next; }, touch() {}, invalidate() {}, toast() {}, autosave() {},
    bus: { emit() {} }, store: { get: (_key, fallback) => fallback, set() {} } };
  install(PM);
  const samples = [];
  for (let i = 0; i < 16; i++) {
    const start = performance.now();
    if (mode === 'scoped-property') {
      PM.hist.beginScoped('Adjust opacity'); PM.hist.track([['layers', 0, 'p', 'opacity']]);
      PM.proj.layers[0].p.opacity.v = i; PM.hist.commit();
    } else {
      PM.hist.beginScoped('Reorder layers'); PM.hist.track([['layers']]);
      PM.proj.layers.reverse(); PM.hist.commit();
    }
    const elapsed = performance.now() - start;
    if (i >= 4) samples.push(elapsed);
    await new Promise(resolve => setImmediate(resolve));
  }
  const final = JSON.stringify(PM.proj);
  for (let i = 0; i < 16; i++) assert.equal(PM.hist.undo(), true);
  assert.equal(JSON.stringify(PM.proj), JSON.stringify(project));
  for (let i = 0; i < 16; i++) assert.equal(PM.hist.redo(), true);
  assert.equal(JSON.stringify(PM.proj), final);
  const sorted = [...samples].sort((a,b) => a-b);
  results[mode] = { samples, medianMs: (sorted[5] + sorted[6]) / 2, maxMs: sorted.at(-1) };
}
console.log(JSON.stringify({ layers: project.layers.length, results }));
