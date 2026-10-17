/*
 * Writes `engine/crates/pm-glsl/tests/fixtures/probes.json`: the 394-probe
 * coverage sweep with WebGL's verdict per probe (drawn, refused at link, or
 * linked with a failing draw), from the newest shader corpus capture's
 * `webgl-metal/status.json`. pm-glsl's `probes` test checks the pipeline's
 * verdict on every probe without a capture (amendment 9: the sweep runs as
 * a regression suite); pixels are compared by pm-render's `effects_webgl`.
 *
 * The probes share two long blocks (`show` overloads and the prelude); they
 * are stored once and each probe's program is rebuilt from placeholders and
 * checked against its SHA-256.
 *
 * Usage: bun engine/crates/pm-render/tools/effects/expectations.ts [--corpus <dir>]
 */
import { createHash } from 'node:crypto';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

import { REPO } from './preambles';
import { PRELUDE, SHOW, probes, program } from './probes';

const args = process.argv.slice(2);
const ROOT = join(homedir(), 'Library', 'Application Support', 'Powermove Engine Corpus');
const corpusDir = args.includes('--corpus')
  ? resolve(args[args.indexOf('--corpus') + 1]!)
  : join(ROOT, (await readdir(ROOT)).filter((d) => d.startsWith('glsl-')).sort().at(-1) ?? 'missing');
const status = JSON.parse(await readFile(join(corpusDir, 'webgl-metal', 'status.json'), 'utf8'));
const TEX = PRELUDE('texture(u_tex, v_uv)');
const UV = PRELUDE('vec4(v_uv, 0.25, 1.0)');
const out = probes.map((probe) => {
  const id = `probe/${probe.group}/${probe.name}`;
  const frag = program(probe);
  const s = status.cases[id];
  if (!s) throw new Error(`${id}: no WebGL verdict in ${corpusDir}`);
  const webgl = !s.compiled ? 'refused' : s.glError ? 'drawError' : 'drawn';
  const template = frag.split(SHOW).join('@SHOW@').split(TEX).join('@PRELUDE_TEX@').split(UV).join('@PRELUDE_UV@');
  return { id, webgl, template, sha256: createHash('sha256').update(frag).digest('hex') };
});
const fixture = { source: 'engine/crates/pm-render/tools/effects/probes.ts', webgl: status.renderer, browser: status.browser, show: SHOW, preludeTex: TEX, preludeUv: UV, probes: out };
const path = join(REPO, 'engine', 'crates', 'pm-glsl', 'tests', 'fixtures', 'probes.json');
await writeFile(path, JSON.stringify(fixture, null, 1) + '\n');
console.log(`${out.length} probes (${out.filter((p) => p.webgl === 'drawn').length} drawn, ${out.filter((p) => p.webgl === 'refused').length} refused, ${out.filter((p) => p.webgl === 'drawError').length} draw errors) → ${path}`);
