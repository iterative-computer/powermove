/*
 * Writes `engine/crates/pm-render/tests/fixtures/parse-uniforms.json`:
 * `PM.parseUniforms(code)` (the app's own function, `legacy/gl/shaders.ts`)
 * over the shader presets, the generated agent shader layers, and edge
 * cases of its regular expression and `parseFloat`, so the Rust port
 * (`shader_layers::uniforms`) is held to the same answers.
 *
 * Usage: bun engine/crates/pm-render/tools/shaders/parse-uniforms-fixture.ts
 */
import { existsSync } from 'node:fs';
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

import { SHADER_PRESETS, parseUniforms } from './sources';
import { REPO } from '../effects/preambles';

const EDGES = [
  'uniform float uA;\n// @param 2 1 4\nuniform float uB;',
  'uniform float uA; // @param\nuniform float uB; // @param 1e3 -1e-2 .5',
  'uniform float uA; //   @param   0x10 Infinity -Infinity',
  'uniform vec3 uC; // @param #abc\nuniform vec3 uD; // @param abc\nuniform vec3 uE; // @param #',
  'uniform bool uT; // @param TRUE\nuniform bool uU; // @param true false\nuniform bool uV;',
  'uniform  int\tuN ;// @param -3\r\nuniform vec4 uW; // @param 1 2 3 4',
  'xuniform float uX; // @param 7\nuniform float iTime; // @param 1\nuniform float uniformValue; // @param 2',
  'uniform float uMyLongName_2; // @param 5.5.5 3e 1e+',
  'uniform float uSpace; // @param 1 2 3',
  'uniform float uBom;﻿// @param 4',
  'uniform float uNel;\u0085// @param 4',
  'uniform mat2 uM; // @param 1\nuniform float uAfter; // @param 9',
  'uniform float uDup; // @param 1\nuniform float uDup; // @param 2',
  'uniform float uNegDef; // @param -0.5\nuniform float uNan; // @param nope 2',
];

const codes: Array<{ name: string; code: string }> = [];
for (const [name, code] of Object.entries(SHADER_PRESETS)) codes.push({ name: `preset/${name}`, code });
const generated = join(import.meta.dir, 'generated');
for (const file of existsSync(generated) ? (await readdir(generated)).sort() : []) {
  const g = JSON.parse(await readFile(join(generated, file), 'utf8'));
  if (g.kind === 'shader' && typeof g.code === 'string') codes.push({ name: `generated/${g.name}`, code: g.code });
}
EDGES.forEach((code, i) => codes.push({ name: `edge/${i}`, code }));

const cases = codes.map(({ name, code }) => ({
  name, code,
  defs: parseUniforms(code).map((d: any) => ({
    name: d.name, type: d.type, label: d.label, control: d.control,
    // JSON has no NaN or Infinity: numbers as strings.
    def: typeof d.def === 'number' ? String(d.def) : d.def,
    min: d.min === undefined ? null : String(d.min),
    max: d.max === undefined ? null : String(d.max),
  })),
}));
const out = join(REPO, 'engine/crates/pm-render/tests/fixtures/parse-uniforms.json');
await writeFile(out, JSON.stringify({ cases }, null, 1) + '\n');
console.log(`${cases.length} cases → ${out}`);
