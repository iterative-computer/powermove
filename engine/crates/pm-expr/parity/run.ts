#!/usr/bin/env bun
// pm-expr parity run: bundles legacy/core/expression.ts, generates cases,
// evaluates them in V8 (Electron as Node, the runtime Powermove ships) and in
// JavaScriptCore (bun, informational), sweeps the Math functions, and checks
// the native build (`cargo test --test parity`) and the wasm32 build
// (wasm.mjs) against the results. Source engine/env.sh first.
//
//   bun engine/crates/pm-expr/parity/run.ts [--fuzz N] [--seed S] [--math N]
//       [--corpus DIR] [--no-corpus] [--keep] [--write-fixture]
//
// --write-fixture regenerates tests/fixtures/parity-{cases,expected}.jsonl from
// public cases only (never corpus data). Private corpus cases are read from
// ~/Library/Application Support/Powermove Engine Corpus/<newest> by default.

import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

import { CONTEXTS, EDGE_CASES, fuzzCases, harvestCorpus, harvestRepository, matrixCases, type Case, type ContextSpec } from './cases';

const here = dirname(new URL(import.meta.url).pathname);
const crateDir = resolve(here, '..');
const repoRoot = resolve(crateDir, '../../..');
const engineDir = join(repoRoot, 'engine');

const args = process.argv.slice(2);
const flag = (name: string) => args.includes(name);
const option = (name: string, fallback: string) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1]! : fallback;
};

const fuzzCount = Number(option('--fuzz', '20000'));
const seed = Number(option('--seed', '20261012'));
const mathCount = Number(option('--math', '400000'));
const writeFixture = flag('--write-fixture');

function sh(command: string, argv: string[], env: Record<string, string> = {}, cwd = repoRoot): void {
  const result = spawnSync(command, argv, { cwd, stdio: 'inherit', env: { ...process.env, ...env } });
  if (result.status !== 0) throw new Error(`${command} ${argv.join(' ')} exited with ${result.status}`);
}

const out = mkdtempSync(join(tmpdir(), 'pm-expr-parity-'));
console.log(`parity: working in ${out}`);

// 1. Bundle expression.ts as plain ESM.
const source = join(repoRoot, 'apps/desktop/src/renderer/src/legacy/core/expression.ts');
const built = await Bun.build({ entrypoints: [source], format: 'esm', target: 'node', minify: false });
if (!built.success) throw new Error(built.logs.join('\n'));
const modulePath = join(out, 'expression.mjs');
writeFileSync(modulePath, await built.outputs[0]!.text());

// 2. Cases.
function writeCases(path: string, contexts: ContextSpec[], cases: Case[]): void {
  const lines = [JSON.stringify({ contexts })];
  for (const c of cases) lines.push(JSON.stringify(c));
  writeFileSync(path, lines.join('\n') + '\n');
}

const publicCases: Case[] = [];
const contexts = CONTEXTS.map((c) => c);
for (const s of EDGE_CASES) {
  // Long sources exercise the limits, which do not depend on the context.
  const ctxs = s.length > 200 ? [0] : contexts.map((_, i) => i);
  for (const ctx of ctxs) publicCases.push({ source: s, ctx, origin: 'edge' });
}
for (const [s, file] of harvestRepository(repoRoot)) {
  for (const ctx of [0, 1, 4]) publicCases.push({ source: s, ctx, origin: `repo:${file}` });
}
const matrix = matrixCases();
// The fixture keeps every pair of atoms with a rotating third of the operators;
// the full run checks the whole matrix on four contexts.
matrix.forEach((s, i) => {
  if (i % 3 === Math.floor(i / 13) % 3) publicCases.push({ source: s, ctx: [1, 3, 4, 6][i % 4]!, origin: 'matrix' });
});
const fixtureFuzz = fuzzCases(2000, seed);
fixtureFuzz.forEach((s, i) => publicCases.push({ source: s, ctx: i % contexts.length, origin: 'fuzz' }));

const allCases: Case[] = [...publicCases];
matrix.forEach((s, i) => allCases.push({ source: s, ctx: [1, 3, 4, 6][(i + 1) % 4]!, origin: 'matrix-full' }));
fuzzCases(fuzzCount, seed + 1).forEach((s, i) => allCases.push({ source: s, ctx: (i * 7) % contexts.length, origin: 'fuzz-large' }));

let corpusDir = option('--corpus', '');
if (!corpusDir && !flag('--no-corpus')) {
  const base = join(homedir(), 'Library/Application Support/Powermove Engine Corpus');
  if (existsSync(base)) {
    const versions = readdirSync(base).filter((n) => !n.startsWith('.')).sort();
    if (versions.length) corpusDir = join(base, versions[versions.length - 1]!);
  }
}
let corpusCount = 0;
if (corpusDir && existsSync(corpusDir)) {
  const corpus = harvestCorpus(corpusDir);
  const offset = contexts.length;
  contexts.push(...corpus.contexts);
  for (const [s, ctx] of corpus.sources) allCases.push({ source: s, ctx: ctx + offset, origin: 'corpus' });
  corpusCount = corpus.sources.length;
}
console.log(`parity: ${publicCases.length} public cases, ${allCases.length - publicCases.length - corpusCount} more fuzz cases, ${corpusCount} corpus cases`);

writeCases(join(out, 'cases.jsonl'), contexts, allCases);
writeCases(join(out, 'fixture-cases.jsonl'), CONTEXTS, publicCases);

// 3. Reference results: V8 (authoritative) and JavaScriptCore (informational).
const electron = join(repoRoot, 'apps/desktop/node_modules/.bin/electron');
if (!existsSync(electron)) throw new Error('Electron is missing: run `bun install` in the repository root');
const reference = join(here, 'reference.mjs');
const v8 = { ELECTRON_RUN_AS_NODE: '1' };
sh(electron, [reference, modulePath, 'cases', join(out, 'cases.jsonl'), join(out, 'expected.jsonl')], v8);
sh(electron, [reference, modulePath, 'cases', join(out, 'fixture-cases.jsonl'), join(out, 'fixture-expected.jsonl')], v8);
sh(process.execPath, [reference, modulePath, 'cases', join(out, 'cases.jsonl'), join(out, 'expected-jsc.jsonl')]);
sh(electron, [reference, modulePath, 'math', out, String(mathCount), String(seed)], v8);

const v8Lines = readFileSync(join(out, 'expected.jsonl'), 'utf8').split('\n');
const jscLines = readFileSync(join(out, 'expected-jsc.jsonl'), 'utf8').split('\n');
let jscDiff = 0;
v8Lines.forEach((line, i) => { if (line !== jscLines[i]) jscDiff += 1; });
console.log(`parity: V8 and JavaScriptCore disagree on ${jscDiff} of ${allCases.length} cases (informational)`);

if (writeFixture) {
  const fixtures = join(crateDir, 'tests/fixtures');
  copyFileSync(join(out, 'fixture-cases.jsonl'), join(fixtures, 'parity-cases.jsonl'));
  copyFileSync(join(out, 'fixture-expected.jsonl'), join(fixtures, 'parity-expected.jsonl'));
  console.log('parity: fixture rewritten');
}

// 4. The Rust side.
sh('nice', ['-n', '15', 'cargo', 'test', '-p', 'pm-expr', '--release', '--test', 'parity', '--', '--nocapture', '--test-threads=4'],
  { PM_EXPR_PARITY_DIR: out }, engineDir);

// 5. The same cases and sweep through the wasm32 build: with `js-pow` under V8
// (every result must match), and the default build under bun (pow may differ).
const wasmBuild = (features: string[]) =>
  sh('nice', ['-n', '15', 'cargo', 'build', '-q', '-p', 'pm-expr', '--release', '--example', 'wasm_parity',
    '--target', 'wasm32-unknown-unknown', ...features], {}, engineDir);
const wasmFile = join(engineDir, 'target/wasm32-unknown-unknown/release/examples/wasm_parity.wasm');
wasmBuild(['--features', 'js-pow']);
sh(electron, [join(here, 'wasm.mjs'), wasmFile, out, '--js-pow'], v8);
wasmBuild([]);
sh(process.execPath, [join(here, 'wasm.mjs'), wasmFile, out]);

if (!flag('--keep')) rmSync(out, { recursive: true, force: true });
else console.log(`parity: kept ${out}`);
