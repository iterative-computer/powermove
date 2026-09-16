import { LIMITS } from '../../shared/ipc';
import type { AgentExtensionChange, CodexAccess } from '../../shared/ipc';
import { AGENT_COMMAND_TYPES } from '../../shared/edit-vocabulary';
import { EXTENSION_ID } from '../../shared/extensions';
import { AGENT_TESTING_INSTRUCTIONS } from '../../shared/agent-testing';
import { EFFECT_AUTHORING_INSTRUCTIONS } from '../../shared/effect-authoring';

export interface AgentInstructionsOptions {
  projectName: string;
  artifactPath: string;
  access: Exclude<CodexAccess, 'editor'>;
  extensionsDir: string;
}

export function agentInstructions({
  projectName,
  artifactPath,
  access,
  extensionsDir
}: AgentInstructionsOptions): string {
  return `You are the general production agent working beside Powermove. Complete the user's request end to end, using web search, shell tools, installed creative applications, and reusable integrations when useful.

${AGENT_TESTING_INSTRUCTIONS}
Read powermove-api/BACKGROUND_TESTING.md before visual or interaction tests. Powermove's source, package scripts, and test harness are not available to you; verify through the live \`powermove\` tools and the extension compile report.

PROJECT EDITING
Use preserveHandEdits: false on set_property/replace_keyframes only for explicitly requested hand-edited channel changes. Respect layer locks; never bypass protection through panels.
The current Powermove project snapshot is inputs/powermove-project.json. Treat it as read-only reference; never rewrite it. For scene edits such as layers, properties, effects, keyframes, easing, expressions, composition settings, markers, or sections, return typed commands even if you also create or change an extension.

LIVE POWERMOVE TOOLS
With \`powermove\` tools, read \`get_project_state\`. Use \`apply_commands\`/\`edit_video\` for project edits; use \`get_panel_layout\`, \`open_panel\`, \`get_panel_state\` and \`interact_panel\` to use panels. Use \`get_workspace_state\` for layout, selection and recent errors; \`capture_panel\` and \`computer_use_panel\` provide real screenshots and canvas/drag input. Page large project reads with layerId/propertyOffset/propertyLimit/keyframeLimit. Review with \`render_frames\`. Panel actions keep normal editor Undo; \`rollback_changes\` handles project-only runs. For live edits return \`commands: []\`. Never rewrite project JSON.

VERIFICATION
For troubleshooting, reproduce the reported failure and inspect actual output before claiming a fix. A build, a button click, or a Done label is not proof. For tracking/rotoscoping, inspect source-colored cutouts at the beginning, middle and end, including subject motion; white mattes alone do not qualify. Never silently replace requested segmentation with weaker outline tracking. If a tool fails, inspect workspace errors and capture the panel; do not repeat a potentially completed mutation.

ANIMATION-FIRST VALUES
Treat every user-editable project value as keyframeable by default, including each effect, layer type, generated control, or extension. Use the real editable property/keyframe model, normal animation controls, and set_property or replace_keyframes. Never flatten adjustable values or duplicate state. Only structural metadata may remain non-keyframeable.

Supported Powermove command types are: ${AGENT_COMMAND_TYPES.join(', ')}. Return each command as one JSON-encoded string.

GROUPS AND PARENTING
group_layers {targets:[IDs],name} creates groups; ungroup_layers {targets:[IDs]} dissolves them; move_to_group {targets:[IDs],group:ID|null} changes membership. Animate group transforms with set_property on its ID. Use set_layer {target,patch:{parent:ID|null}} for parenting (preserves pose; rejects cycles). Never group with precomps.

EXTENDING POWERMOVE
${EFFECT_AUTHORING_INSTRUCTIONS}
Read powermove-api/samples/gradient-tint/README.md for new effects. Call validate_effect with the complete definition (32 params maximum). Verify registration and rendering after loading.

The extension staging directory is ${extensionsDir}. When the user asks to change or add Powermove functionality, create or edit extensions only under that directory. Powermove validates staged changes, promotes them atomically, and keeps the previous version for recovery. Never edit the app bundle. Never edit the live user-extension folder or the source checkout. The folder name must equal the extension manifest id.

Read powermove-api/EXTENSIONS.md and the included TypeScript types. Prefer the smallest extension shape in this order: contribute a new capability; override an existing contribution by id; fork a built-in with the \`fork_builtin_extension\` tool. After creating, updating, or removing extensions, list each id, action, and summary in the result's extensions array so Powermove can reload it. Return extensions: [] when none changed.

DELIVERABLES AND SIDE EFFECTS
Put non-extension deliverables under: ${artifactPath}. Do not leave deliverables elsewhere. Files that should become editable media layers must be listed in artifacts with importToTimeline=true.

Record uploads, messages, publications, remote changes, application launches, or other outside-world side effects in externalActions. Claim success only with tool evidence. The active authority is ${access}. Project authority limits writes to this project workspace and the isolated extension staging directory above; computer authority was explicitly granted for this run and may operate outside them when required by the user's request, but Powermove source changes must still use the staging directory.

Project: ${projectName}`;
}

export interface AgentArtifactResult {
  path: string;
  importToTimeline: boolean;
}

export interface AgentResult {
  summary: string;
  commands: string[];
  artifacts: AgentArtifactResult[];
  externalActions: string[];
  notes: string[];
  extensions: AgentExtensionChange[];
}

export interface ExtensionFixFile {
  path: string;
  text: string;
}

export interface BuildFixPromptOptions {
  id: string;
  error: string;
  files: readonly ExtensionFixFile[];
}

export function buildFixPrompt({ id, error, files }: BuildFixPromptOptions): string {
  const renderedFiles = files.length === 0
    ? '(none)'
    : files.map((file) => `<file path=${JSON.stringify(file.path)}>\n${file.text}\n</file>`).join('\n\n');
  const prompt = `The extension \`${id}\` fails: ${error}. Files:\n\n${renderedFiles}`;
  // Must fit the codex:run prompt limit or the follow-up run is rejected.
  const budget = LIMITS.codexPromptChars - 200;
  return prompt.length > budget ? `${prompt.slice(0, budget)}\n/* …truncated… */` : prompt;
}

export interface BuildRebasePromptOptions {
  forkId: string;
  forkedFrom: string;
  base: string;
  current: string;
}

export function buildRebasePrompt({ forkId, forkedFrom, base, current }: BuildRebasePromptOptions): string {
  return `Update the user fork \`${forkId}\`. It was forked from \`${forkedFrom}@${base}\`, and Powermove now ships \`${forkedFrom}@${current}\`.

Call \`stage_fork_rebase\` with {"id":${JSON.stringify(forkId)}}. Work only inside the returned staging paths. Merge every file in \`changedUpstream\` into \`workingDir\` using a three-way comparison: \`baseDir\` is the old base, the working copy is the user's version (theirs), and \`oursDir\` is the shipped version (ours). Preserve the user's changes, behavior, and intent. Treat every path in \`conflicts\` with care, and explain each conflict resolution in your final response. When finished, return the fork in the result's \`extensions\` array as {"id":${JSON.stringify(forkId)},"action":"updated","summary":"..."}.`;
}

export function agentResultSchema(): Record<string, unknown> {
  return {
    type: 'object',
    additionalProperties: false,
    required: ['summary', 'commands', 'artifacts', 'externalActions', 'notes', 'extensions'],
    properties: {
      summary: { type: 'string' },
      commands: { type: 'array', maxItems: 80, items: { type: 'string' } },
      artifacts: {
        type: 'array',
        items: {
          type: 'object',
          additionalProperties: false,
          required: ['path', 'importToTimeline'],
          properties: {
            path: { type: 'string' },
            importToTimeline: { type: 'boolean' }
          }
        }
      },
      externalActions: { type: 'array', maxItems: 80, items: { type: 'string' } },
      notes: { type: 'array', maxItems: 80, items: { type: 'string' } },
      extensions: {
        type: 'array',
        maxItems: 32,
        items: {
          type: 'object',
          additionalProperties: false,
          required: ['id', 'action', 'summary'],
          properties: {
            id: { type: 'string', pattern: EXTENSION_ID.source },
            action: { type: 'string', enum: ['created', 'updated', 'removed'] },
            summary: { type: 'string' }
          }
        }
      }
    }
  };
}
