/*
 * Generates the agent-authored part of the shader gate corpus
 * (`prompts.ts`) and writes each answer to `generated/<name>.json`.
 *
 * Provenance. The in-app agent harness (`src/main/claude/runner.ts`) needs a
 * running Powermove (its MCP tools, the project workspace, render_frames),
 * which cannot run headlessly here without a window. This script therefore
 * runs the same model through the same CLI the harness drives (`claude
 * --print`, model `opus`) with the harness's own system prompt
 * (`agentInstructions()` from `src/main/codex/instructions.ts`, plus the
 * adapter's JSON-schema suffix), the API pack it points the agent at
 * (`powermove-api/EXTENSIONS.md` and the samples, readable with the Read
 * tool), and the shader-layer runtime the Shader panel shows authors
 * (`PM.SHADER_HEADER`, `PM.SHADER_TEMPLATE`). Differences from an in-app
 * run: no Powermove MCP tools (no render_frames or validate_effect loop), and
 * the answer is returned as JSON instead of as project commands or a staged
 * extension. Every output records the CLI version, the model, and the
 * instructions' hash.
 *
 * Usage: bun engine/crates/pm-render/tools/shaders/generate.ts [--only <name>] [--jobs 3] [--force]
 */
import { createHash } from 'node:crypto';
import { cp, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { CONTRACT, PROMPTS, type Prompt } from './prompts';
import { SHADER_HEADER, SHADER_TEMPLATE } from './sources';
import { REPO } from '../effects/preambles';

const APP = join(REPO, 'apps', 'desktop');
const OUT = join(import.meta.dir, 'generated');
const args = process.argv.slice(2);
const only = args.includes('--only') ? args[args.indexOf('--only') + 1]! : null;
const jobs = args.includes('--jobs') ? Number(args[args.indexOf('--jobs') + 1]) : 3;
const force = args.includes('--force');

for (const feature of CONTRACT) {
  if (!PROMPTS.some((p) => p.covers.includes(feature))) throw new Error(`no prompt covers ${feature}`);
}

const { agentInstructions } = await import(join(APP, 'src/main/codex/instructions.ts'));
const version = String(Bun.spawnSync(['claude', '--version']).stdout).trim();

const SCHEMA = {
  type: 'object',
  properties: {
    kind: { type: 'string', enum: ['shader', 'extension', 'effect', 'transition'] },
    code: { type: 'string', description: 'Shader layer: the GLSL body stored in the layer (d.code). Empty otherwise.' },
    definitionJson: { type: 'string', description: 'Extension layer, effect, or transition: the definition object passed to api.layers.register / api.effects.register / api.transitions.register, as JSON. Empty for shader layers.' },
    summary: { type: 'string' },
  },
  required: ['kind', 'code', 'definitionJson', 'summary'],
  additionalProperties: false,
};

const RUNTIME = `SHADER LAYER RUNTIME (what the Shader panel shows authors)
A shader layer stores only its GLSL body in d.code. Powermove compiles PM.SHADER_HEADER + "\\n" + d.code as GLSL ES 3.00 and draws it over the layer's rectangle. Uniforms annotated with "// @param default min max" (or "// @param #RRGGBB" for vec3 colors, "// @param true" for bool) get Inspector controls.

PM.SHADER_HEADER:
${SHADER_HEADER}
PM.SHADER_TEMPLATE (the starting point of a new shader layer):
${SHADER_TEMPLATE}`;

function request(p: Prompt): string {
  const deliver = p.kind === 'shader'
    ? 'Deliver a shader layer: return kind "shader" and the layer\'s GLSL body in code (definitionJson empty).'
    : p.kind === 'extension'
      ? 'Deliver the layer definition: return kind "extension" and the object you would pass to api.layers.register as JSON in definitionJson (renderer kind "fragment"; id qualified as "gen.<name>").'
      : `Deliver the definition: return kind "${p.kind}" and the object you would pass to api.${p.kind === 'effect' ? 'effects' : 'transitions'}.register as JSON in definitionJson (id qualified as "gen.<name>").`;
  return `${p.prompt}\n\n${deliver} This run has no live Powermove tools attached; return the deliverable in the result instead of applying it.`;
}

async function generate(p: Prompt): Promise<void> {
  const file = join(OUT, `${p.name}.json`);
  if (!force && existsSync(file)) return;
  const workspace = await mkdtemp(join(tmpdir(), 'pm-shader-gen-'));
  try {
    await cp(join(APP, 'docs'), join(workspace, 'powermove-api'), { recursive: true });
    const instructions = agentInstructions({
      projectName: 'Shader corpus', artifactPath: join(workspace, 'artifacts'), access: 'project',
      extensionsDir: join(workspace, 'extensions'), request: p.prompt,
    });
    const system = `${instructions}\n\nReturn the final answer only through the requested JSON schema.\n\n${RUNTIME}`;
    const started = Date.now();
    const proc = Bun.spawn([
      'claude', '--print', '--model', 'opus', '--output-format', 'json', '--no-session-persistence',
      '--json-schema', JSON.stringify(SCHEMA), '--system-prompt', system,
      '--tools', 'Read,Glob,Grep', '--allowedTools', 'Read,Glob,Grep', '--add-dir', workspace,
    ], { cwd: workspace, stdin: new TextEncoder().encode(request(p)), stdout: 'pipe', stderr: 'pipe' });
    const [stdout, stderr, code] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited]);
    if (code !== 0) throw new Error(`claude exited ${code}: ${stderr.slice(0, 400)}`);
    const result = JSON.parse(stdout);
    const answer = result.structured_output ?? JSON.parse(result.result);
    if (answer.kind !== p.kind) throw new Error(`${p.name}: asked for ${p.kind}, got ${answer.kind}`);
    const definition = answer.definitionJson ? JSON.parse(answer.definitionJson) : null;
    await writeFile(file, JSON.stringify({
      name: p.name, kind: p.kind, covers: p.covers, prompt: p.prompt,
      provenance: {
        via: 'claude --print with the in-app agent instructions (no Powermove MCP tools)',
        cli: version, model: Object.keys(result.modelUsage ?? {})[0] ?? 'opus',
        instructionsSha256: createHash('sha256').update(system).digest('hex'),
        seconds: (Date.now() - started) / 1000, generated: new Date().toISOString(),
      },
      code: answer.code || null, definition, summary: answer.summary,
    }, null, 1) + '\n');
    console.log(`${p.name}: ok (${((Date.now() - started) / 1000).toFixed(0)} s)`);
  } catch (error) {
    console.log(`${p.name}: FAILED ${String(error).slice(0, 300)}`);
  } finally {
    await rm(workspace, { recursive: true, force: true });
  }
}

await mkdir(OUT, { recursive: true });
const queue = PROMPTS.filter((p) => !only || p.name === only);
await Promise.all(Array.from({ length: Math.max(1, jobs) }, async () => {
  for (let p = queue.shift(); p; p = queue.shift()) await generate(p);
}));
const done = PROMPTS.filter((p) => existsSync(join(OUT, `${p.name}.json`))).length;
console.log(`${done} of ${PROMPTS.length} generated → ${OUT}`);

