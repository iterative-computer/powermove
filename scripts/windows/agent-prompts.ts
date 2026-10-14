/**
 * Writes the agent prompt text and editor tool catalog the Rust host reads at
 * runtime (`<resources>/agent/prompts.json`). The TypeScript sources stay the
 * single owner of this text; the host fills the `{{pm:*}}` placeholders.
 *
 *   bun scripts/windows/agent-prompts.ts <output.json>
 */
import { writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';

import { agentInstructions, agentResultSchema, AGENT_SHELL_NETWORK_INSTRUCTIONS } from '../../apps/desktop/src/main/codex/instructions';
import {
  OUTSIDE_SANDBOX_CALL_MS,
  OUTSIDE_SANDBOX_TOOL,
  POWERMOVE_AGENT_TOOLS,
  POWERMOVE_APP_AGENT_TOOLS,
  POWERMOVE_LIVE_INSPECTION_TOOL_NAMES,
  POWERMOVE_STORE_TOOL_NAMES
} from '../../apps/desktop/src/main/agent-tools/spec';
import { AGENT_TESTING_INSTRUCTIONS } from '../../apps/desktop/src/shared/agent-testing';
import { AGENT_ORCHESTRATION_INSTRUCTIONS, ORCHESTRATION_TOOL_NAMES } from '../../apps/desktop/src/shared/agent-orchestration';
import { AGENT_MEDIA_TOOL_NAMES } from '../../apps/desktop/src/shared/media-tools';
import { AGENT_RESPONSE_STYLE } from '../../apps/desktop/src/shared/response-style';
import { EDITOR_EXTENSION_INSTRUCTIONS, EFFECT_AUTHORING_INSTRUCTIONS } from '../../apps/desktop/src/shared/effect-authoring';

const output = process.argv[2];
if (!output) throw new Error('usage: bun agent-prompts.ts <output.json>');

const placeholders = {
  projectName: '{{pm:projectName}}',
  artifactPath: '{{pm:artifactPath}}',
  extensionsDir: '{{pm:extensionsDir}}'
};

/* agentTaskProfile() picks guidance from the request; render every profile so
   the host only has to classify the request. Keys are extensions|footage|creative bits. */
const sample = { extensions: 'sandbox', footage: 'trim', creative: 'animate' };
const project: Record<string, Record<string, string>> = {};
for (const access of ['project', 'computer'] as const) {
  project[access] = {};
  for (let bits = 0; bits < 8; bits += 1) {
    const words = [bits & 4 ? sample.extensions : '', bits & 2 ? sample.footage : '', bits & 1 ? sample.creative : ''].filter(Boolean);
    const key = `${bits & 4 ? 1 : 0}${bits & 2 ? 1 : 0}${bits & 1 ? 1 : 0}`;
    project[access][key] = agentInstructions({ ...placeholders, access, request: words.join(' ') || 'x' });
  }
}
const app = agentInstructions({ ...placeholders, access: 'project', context: 'app' });

const prompts = {
  version: 1,
  instructions: { project, app },
  resultSchema: agentResultSchema(),
  testingInstructions: AGENT_TESTING_INSTRUCTIONS,
  shellNetworkInstructions: AGENT_SHELL_NETWORK_INSTRUCTIONS,
  orchestrationInstructions: AGENT_ORCHESTRATION_INSTRUCTIONS,
  responseStyle: AGENT_RESPONSE_STYLE,
  effectAuthoringInstructions: EFFECT_AUTHORING_INSTRUCTIONS,
  editorExtensionInstructions: EDITOR_EXTENSION_INSTRUCTIONS,
  tools: POWERMOVE_AGENT_TOOLS,
  appToolNames: POWERMOVE_APP_AGENT_TOOLS.map(tool => tool.name),
  inspectionToolNames: [...POWERMOVE_LIVE_INSPECTION_TOOL_NAMES],
  orchestrationToolNames: [...ORCHESTRATION_TOOL_NAMES],
  storeToolNames: [...POWERMOVE_STORE_TOOL_NAMES],
  mediaToolNames: [...AGENT_MEDIA_TOOL_NAMES],
  outsideSandboxTool: OUTSIDE_SANDBOX_TOOL,
  outsideSandboxCallMs: OUTSIDE_SANDBOX_CALL_MS
};

mkdirSync(path.dirname(output), { recursive: true });
writeFileSync(output, `${JSON.stringify(prompts, null, 1)}\n`);
console.log(`Wrote ${output}`);
