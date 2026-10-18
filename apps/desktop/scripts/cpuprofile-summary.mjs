/*
 * Summarizes a V8 CPU profile (`.cpuprofile`, Chrome DevTools' format, as
 * `Profiler.stop` returns it): total sampled time, idle and GC, the
 * functions with the most self time, and the most inclusive time (each
 * function counted once per stack, so recursion is not double counted).
 * docs/engine/perf.md records profiles made with it.
 *
 *   node scripts/cpuprofile-summary.mjs <file.cpuprofile> [--top 40] [--match text] [--json out.json]
 *
 * `--match` keeps only the inclusive rows whose name or file contains the
 * text (case-insensitive), to follow one call path.
 */
import fs from 'node:fs';

const args = process.argv.slice(2);
const file = args.find((a) => !a.startsWith('--') && !/^\d+$/.test(a));
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : fallback;
};
if (!file) {
  console.error('usage: cpuprofile-summary.mjs <file.cpuprofile> [--top N] [--match text] [--json out.json]');
  process.exit(2);
}
const top = Number(opt('top', '40'));
const match = opt('match', '')?.toLowerCase() ?? '';
const profile = JSON.parse(fs.readFileSync(file, 'utf8'));

const nodes = new Map(profile.nodes.map((n) => [n.id, n]));
const parent = new Map();
for (const n of profile.nodes) for (const c of n.children ?? []) parent.set(c, n.id);

// Time per sample: the deltas (µs) that follow each sample.
const selfUs = new Map();
const deltas = profile.timeDeltas ?? [];
for (let i = 0; i < profile.samples.length; i++) {
  const id = profile.samples[i];
  const dt = deltas[i + 1] ?? 0;
  selfUs.set(id, (selfUs.get(id) ?? 0) + dt);
}

const label = (n) => {
  const f = n.callFrame;
  const name = f.functionName || '(anonymous)';
  const url = (f.url || '').replace(/^.*\/(out|src|node_modules)\//, '$1/');
  return url ? `${name} ${url}:${f.lineNumber + 1}` : name;
};

const self = new Map();
const inclusive = new Map();
let total = 0;
for (const [id, us] of selfUs) {
  total += us;
  const n = nodes.get(id);
  const own = label(n);
  self.set(own, (self.get(own) ?? 0) + us);
  const seen = new Set();
  for (let cur = id; cur !== undefined; cur = parent.get(cur)) {
    const l = label(nodes.get(cur));
    if (seen.has(l)) continue;
    seen.add(l);
    inclusive.set(l, (inclusive.get(l) ?? 0) + us);
  }
}

const ms = (us) => (us / 1000).toFixed(1);
const pct = (us) => ((100 * us) / Math.max(1, total)).toFixed(1);
const named = (name) => self.get(name) ?? 0;
const rows = (map) => [...map].sort((a, b) => b[1] - a[1]);
const summary = {
  file,
  totalMs: +ms(total),
  idleMs: +ms(named('(idle)')),
  gcMs: +ms(named('(garbage collector)')),
  programMs: +ms(named('(program)')),
  self: rows(self).slice(0, top).map(([name, us]) => ({ name, ms: +ms(us), pct: +pct(us) })),
  inclusive: rows(inclusive)
    .filter(([name]) => !match || name.toLowerCase().includes(match))
    .slice(0, top)
    .map(([name, us]) => ({ name, ms: +ms(us), pct: +pct(us) })),
};

console.log(`${file}: ${summary.totalMs} ms sampled; idle ${summary.idleMs}, GC ${summary.gcMs}, program ${summary.programMs}`);
console.log('\nself time');
for (const r of summary.self) console.log(`${String(r.ms).padStart(9)} ms ${String(r.pct).padStart(5)}%  ${r.name}`);
console.log(`\ninclusive time${match ? ` (matching "${match}")` : ''}`);
for (const r of summary.inclusive) console.log(`${String(r.ms).padStart(9)} ms ${String(r.pct).padStart(5)}%  ${r.name}`);
const out = opt('json', '');
if (out) fs.writeFileSync(out, JSON.stringify(summary, null, 2));
