/*
 * Joins the engine's outcomes (`s3-shaders fuzz --dir <dir>` → engine.jsonl,
 * or a mutants/bombs jsonl given with --engine) with WebGL's link status
 * (`scripts/webgl-compile.ts` → webgl-<angle>.json) and classifies every input:
 *
 *   both-accept, both-reject             agreement
 *   engine-only-accept                   engine more permissive than WebGL
 *   webgl-only-accept                    a WebGL-valid program the engine fails (pipeline bug)
 *   engine-abnormal                      crash or timeout in the engine
 *   webgl-no-answer                      WebGL hung or lost its context
 *
 * Usage: bun scripts/compile-parity.ts --dir work/fuzz/probes [--engine <jsonl>] [--angle metal] [--list]
 */
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const SPIKE = join(import.meta.dir, '..');
const args = process.argv.slice(2);
const opt = (name: string, fallback: string) => (args.includes(name) ? args[args.indexOf(name) + 1]! : fallback);
const dir = join(SPIKE, opt('--dir', 'work/fuzz/probes'));
const enginePath = args.includes('--engine') ? join(SPIKE, opt('--engine', '')) : join(dir, 'engine.jsonl');
const webgl = JSON.parse(readFileSync(join(dir, `webgl-${opt('--angle', 'metal')}.json`), 'utf8')).results as Record<string, { linked: boolean | null; log?: string; ms: number; outcome?: string }>;
const engine = readFileSync(enginePath, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));

const name = (i: number) => (existsSync(join(dir, `${i}.name`)) ? readFileSync(join(dir, `${i}.name`), 'utf8') : '');
const classes = new Map<string, number>();
const rows: string[] = [];
for (const e of engine) {
  const w = webgl[String(e.index)];
  const accepted = e.naga === 'ok';
  // Classified on the naga path (the candidate); SPIRV-Cross disagreements are tagged.
  let cls: string;
  if (/^(timeout|crash|panic)/.test(e.naga)) cls = 'engine-abnormal';
  else if (!w || w.linked === null) cls = 'webgl-no-answer';
  else if (w.linked && accepted) cls = 'both-accept';
  else if (!w.linked && !accepted) cls = 'both-reject';
  else if (accepted) cls = 'engine-only-accept';
  else cls = 'webgl-only-accept';
  if ((e.spvc === 'ok') !== accepted || /^(timeout|crash|panic)/.test(e.spvc)) cls += ` [spvc ${e.spvc.split(' ')[0]}]`;
  classes.set(cls, (classes.get(cls) ?? 0) + 1);
  if (args.includes('--list') || !cls.startsWith('both') || cls.includes('[')) {
    const webglText = !w ? 'missing' : w.linked === null ? (w.outcome ?? w.log ?? 'no answer') : w.linked ? 'links' : `rejects: ${(w.log ?? '').replace(/\s+/g, ' ').slice(0, 90)}`;
    rows.push(`| ${e.index} | ${name(e.index).slice(0, 40)} | ${cls} | ${webglText.replace(/\|/g, '\\|')} (${Math.round(w?.ms ?? 0)} ms) | naga ${e.naga}, spvc ${e.spvc} (${Math.round(e.ms)} ms) ${(e.detail ?? e.note ?? '').replace(/\s+/g, ' ').slice(0, 90).replace(/\|/g, '\\|')} |`);
  }
}
console.log(`${engine.length} inputs: ${[...classes].map(([k, v]) => `${k} ${v}`).join(', ')}`);
if (rows.length) console.log(['', '| # | Input | Class | WebGL | Engine |', '|---|---|---|---|---|', ...rows].join('\n'));
