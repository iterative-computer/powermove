/*
 * Summarizes an S3 run as Markdown tables: compile and match rates per source
 * group and path, every failure with its cause, contract coverage of the
 * generated corpus, timings, and cross-implementation WebGL sensitivity.
 *
 * Usage: bun scripts/report.ts [--results work/results] [--webgl work/webgl/metal]
 * Exit code 1 when a section 11 contract row has no generated case.
 */
import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

import { CONTRACT_ROWS } from '../corpus/generated';

const SPIKE = join(import.meta.dir, '..');
const args = process.argv.slice(2);
const opt = (name: string, fallback: string) => (args.includes(name) ? args[args.indexOf(name) + 1]! : fallback);
const results = join(SPIKE, opt('--results', 'work/results'));
const webglDir = join(SPIKE, opt('--webgl', 'work/webgl/metal'));

interface Metrics { pixels: number; exact_pixels: number; over_one: number; max_diff_8bit: number; max_abs_float: number; nan_mismatch: number; pass: boolean }
interface Variant { variant: string; metrics?: Metrics; note?: string }
interface Result { id: string; group: string; kind: string; path: string; compiled: boolean; failure?: { stage: string; message: string; diagnostics: { line: number | null; message: string }[] }; polyfilled: string[]; renamed: string[]; timings_ms: Record<string, number>; variants: Variant[] }

const corpus = JSON.parse(await readFile(join(SPIKE, 'work/corpus.json'), 'utf8'));
const status = JSON.parse(await readFile(join(webglDir, 'status.json'), 'utf8'));
const read = async (path: string): Promise<Result[]> =>
  (await readFile(path, 'utf8')).split('\n').filter(Boolean).map((line) => JSON.parse(line));
const paths: Record<string, Result[]> = {};
for (const name of ['naga', 'spvc']) {
  const file = join(results, `${name}.jsonl`);
  if (existsSync(file)) paths[name] = await read(file);
}

const groups: string[] = [...new Set<string>(corpus.cases.map((c: any) => c.group))];
const webglOk = (id: string) => status.cases[id]?.compiled === true;
const caseMatched = (r: Result) => r.compiled && r.variants.length > 0 && r.variants.every((v) => v.metrics?.pass);
/** Compile parity: the engine compiles exactly the programs WebGL links. */
const agrees = (r: Result) => r.compiled === webglOk(r.id);
const pct = (n: number, d: number) => (d === 0 ? 'n/a' : `${((100 * n) / d).toFixed(1)}%`);
const out: string[] = [];
const table = (head: string[], rows: (string | number)[][]) => {
  out.push(`| ${head.join(' | ')} |`, `|${head.map(() => '---').join('|')}|`);
  for (const row of rows) out.push(`| ${row.join(' | ')} |`);
  out.push('');
};

out.push(`Corpus: ${corpus.cases.length} cases, ${corpus.cases.reduce((n: number, c: any) => n + c.variants.length, 0)} variants. WebGL reference: ${status.renderer} (Chromium ${status.browser}).`, '');

for (const [name, rs] of Object.entries(paths)) {
  out.push(`### Path: ${name === 'naga' ? 'naga (glslang → SPIR-V split → naga → MSL)' : 'SPIRV-Cross (glslang → SPIR-V → SPIRV-Cross MSL → passthrough)'}`, '');
  const rows = groups.map((group) => {
    const g = rs.filter((r) => r.group === group);
    const valid = g.filter((r) => webglOk(r.id));
    const compiledValid = valid.filter((r) => r.compiled);
    const variants = valid.flatMap((r) => r.variants);
    const matchedVariants = variants.filter((v) => v.metrics?.pass);
    return [group, g.length, valid.length, `${compiledValid.length}/${valid.length}`, `${g.filter(agrees).length}/${g.length}`,
      `${valid.filter(caseMatched).length}/${valid.length}`, `${matchedVariants.length}/${variants.length}`];
  });
  const all = rs;
  const valid = all.filter((r) => webglOk(r.id));
  const variants = valid.flatMap((r) => r.variants);
  rows.push(['**all**', all.length, valid.length, `${valid.filter((r) => r.compiled).length}/${valid.length} (${pct(valid.filter((r) => r.compiled).length, valid.length)})`,
    `${all.filter(agrees).length}/${all.length}`, `${valid.filter(caseMatched).length}/${valid.length} (${pct(valid.filter(caseMatched).length, valid.length)})`,
    `${variants.filter((v) => v.metrics?.pass).length}/${variants.length} (${pct(variants.filter((v) => v.metrics?.pass).length, variants.length)})`]);
  table(['Group', 'Cases', 'WebGL links', 'Engine compiles (of WebGL-valid)', 'Compile parity', 'Cases matching', 'Variants matching'], rows);

  const exact = variants.filter((v) => v.metrics && v.metrics.exact_pixels === v.metrics.pixels).length;
  const zero = variants.filter((v) => v.metrics && v.metrics.max_diff_8bit === 0).length;
  out.push(`Exactness (${name}): ${exact}/${variants.length} variants bit-identical in RGBA16F; ${zero}/${variants.length} with 8-bit difference 0; worst passing max difference ${Math.max(0, ...variants.filter((v) => v.metrics?.pass).map((v) => v.metrics!.max_diff_8bit))}.`, '');
}

out.push('### Failures and divergences', '');
const failureRows: string[][] = [];
for (const [name, rs] of Object.entries(paths)) {
  for (const r of rs) {
    const webgl = webglOk(r.id);
    if (!webgl && !r.compiled) {
      const lines = (r.failure?.diagnostics ?? []).map((d) => (d.line == null ? 'generated code' : `author line ${d.line}`));
      failureRows.push([name, r.id, `rejected by WebGL and engine alike (engine: ${r.failure?.stage}${lines.length ? ', ' + [...new Set(lines)].join(', ') : ''})`, (status.cases[r.id]?.log ?? '').split('\n')[0]!.slice(0, 120)]);
      continue;
    }
    if (!webgl && r.compiled) { failureRows.push([name, r.id, 'engine accepts a program WebGL rejects', (status.cases[r.id]?.log ?? '').split('\n')[0]!.slice(0, 120)]); continue; }
    if (!r.compiled) { failureRows.push([name, r.id, `compile failure at ${r.failure?.stage}`, (r.failure?.message ?? '').replace(/\s+/g, ' ').slice(0, 160)]); continue; }
    for (const v of r.variants) {
      if (!v.metrics) failureRows.push([name, r.id, `${v.variant}: no comparison`, v.note ?? '']);
      else if (!v.metrics.pass) failureRows.push([name, r.id, `${v.variant}: pixel mismatch`, `max ${v.metrics.max_diff_8bit} CV; ${v.metrics.over_one} px > 1 CV (${pct(v.metrics.over_one, v.metrics.pixels)}); max |Δ| ${v.metrics.max_abs_float.toFixed(4)}`]);
    }
  }
}
table(['Path', 'Case', 'Outcome', 'Detail'], failureRows.map((r) => r.map((c) => c.replace(/\|/g, '\\|'))));

const polyfilled = (paths.naga ?? []).filter((r) => r.polyfilled?.length).map((r) => `${r.id} (${r.polyfilled.join(', ')})`);
const renamed = (paths.naga ?? []).filter((r) => r.renamed?.length).map((r) => `${r.id} (${r.renamed.join(', ')})`);
out.push(`Pipeline rewrites on the naga path: polyfilled built-ins in ${polyfilled.length} cases${polyfilled.length ? ': ' + polyfilled.join('; ') : ''}. Renamed GLSL 4.50 identifiers in ${renamed.length} cases${renamed.length ? ': ' + renamed.join('; ') : ''}.`, '');

out.push('### Contract coverage (generated corpus)', '');
const generated = corpus.cases.filter((c: any) => c.group === 'generated');
let uncovered = 0;
table(['Section 11 row', 'Generated cases'], Object.entries(CONTRACT_ROWS).map(([row, tags]) => {
  const ids = generated.filter((c: any) => (c.covers ?? []).some((t: string) => tags.includes(t))).map((c: any) => c.id.replace('generated/', ''));
  if (ids.length === 0) uncovered++;
  return [row, ids.length ? `${ids.length}: ${ids.join(', ')}` : '**none**'];
}));

out.push('### Translation timings (median / max ms per case)', '');
const median = (xs: number[]) => { const s = [...xs].sort((a, b) => a - b); return s.length ? s[Math.floor(s.length / 2)]! : NaN; };
const stageRows: (string | number)[][] = [];
for (const [name, rs] of Object.entries(paths)) {
  const keys = [...new Set(rs.flatMap((r) => Object.keys(r.timings_ms)))].filter((k) => !k.startsWith('render-')).sort();
  for (const key of keys) {
    const xs = rs.map((r) => r.timings_ms[key]).filter((x): x is number => typeof x === 'number');
    stageRows.push([name, key, median(xs).toFixed(2), Math.max(...xs).toFixed(2)]);
  }
}
table(['Path', 'Stage', 'Median', 'Max'], stageRows);

const crossFile = join(results, 'webgl-metal-vs-swiftshader.jsonl');
if (existsSync(crossFile)) {
  const rows = (await readFile(crossFile, 'utf8')).split('\n').filter(Boolean).map((l) => JSON.parse(l)).filter((r) => r.metrics);
  const failing = rows.filter((r) => !r.metrics.pass);
  out.push('### Cross-implementation sensitivity (WebGL on ANGLE Metal vs WebGL on SwiftShader, same tolerance)', '');
  out.push(`${rows.length - failing.length}/${rows.length} variants agree; ${new Set(failing.map((r) => r.id)).size} cases differ between the two WebGL implementations.`, '');
  table(['Group', 'Variants agreeing'], groups.map((g) => {
    const gr = rows.filter((r) => r.group === g);
    return [g, `${gr.filter((r) => r.metrics.pass).length}/${gr.length}`];
  }));
}

console.log(out.join('\n'));
if (uncovered) {
  console.error(`${uncovered} contract rows have no generated case`);
  process.exit(1);
}
