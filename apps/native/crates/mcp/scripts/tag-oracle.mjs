// Execute the frozen public catalog, rather than maintaining a second schema.
import { execFileSync } from 'node:child_process';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../..');
export const tag = file => execFileSync('git', ['show', `v1.2.0:${file}`], { cwd: root, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 });
export function evaluate(source, names, bindings = {}) {
  const code = stripTypeScriptTypes(source.replace(/^import .*;\n/gm, '').replace(/^export /gm, ''), { mode: 'strip' });
  return vm.runInNewContext(`${code}\n;({${names.join(',')}})`, bindings);
}
const base = 'apps/desktop/src/main/agent-tools/';
const shared = evaluate(tag('apps/desktop/src/shared/agent-orchestration.ts').split('export const AGENT_ORCHESTRATION_INSTRUCTIONS')[0], ['ORCHESTRATION_TOOL_NAMES']);
const orch = evaluate(tag(base + 'orchestration-spec.ts'), ['ORCHESTRATION_TOOLS'], shared);
const manifest = tag('packages/registry/src/manifest.ts');
const EXTENSION_ID = vm.runInNewContext(manifest.match(/export const EXTENSION_ID = (.*);/)[1]);
const agent = evaluate(tag(base + 'spec.ts'), ['POWERMOVE_AGENT_TOOLS', 'POWERMOVE_LIVE_INSPECTION_TOOL_NAMES', 'POWERMOVE_APP_AGENT_TOOLS'], { ...shared, ...orch, EXTENSION_ID, COMMAND_JSON_LIMIT: Number(tag('apps/desktop/src/shared/edit-limits.ts').match(/= ([\d_]+)/)[1].replaceAll('_','')) });
const external = evaluate(tag(base + 'external-spec.ts'), ['EXTERNAL_MCP_TOOLS', 'EXTERNAL_MCP_INSTRUCTIONS']);
const harness=tag('apps/desktop/src/renderer/src/legacy/assistant/harness.ts');
const fields=evaluate(harness.slice(harness.indexOf('const FIELDS:'),harness.indexOf('const clone =')),['FIELDS']).FIELDS;
const easing=tag('apps/desktop/src/renderer/src/legacy/core/easing.ts');
const presets=evaluate(easing.slice(easing.indexOf('const PRESETS:'),easing.indexOf('const Ease:')),['PRESETS']).PRESETS;
export const catalog = JSON.parse(JSON.stringify({ easePresets:presets, commandFields:fields, tools: [...external.EXTERNAL_MCP_TOOLS, ...agent.POWERMOVE_AGENT_TOOLS], inspection: agent.POWERMOVE_LIVE_INSPECTION_TOOL_NAMES, app: agent.POWERMOVE_APP_AGENT_TOOLS.map(t=>t.name), instructions: external.EXTERNAL_MCP_INSTRUCTIONS }));
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const target = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../fixtures/catalog-v1.2.0.json');
  if (process.argv.includes('--write')) fs.writeFileSync(target, JSON.stringify(catalog, null, 2) + '\n');
  else {
    if (JSON.stringify(JSON.parse(fs.readFileSync(target))) !== JSON.stringify(catalog)) throw Error('Frozen catalog drift');
    process.stdout.write(`${catalog.tools.length} tool names, descriptions and input schemas identical to v1.2.0\n`);
  }
}
