import { LIMITS } from '../../shared/ipc';
import type { AgentExtensionChange, CodexAccess } from '../../shared/ipc';
import { AGENT_COMMAND_TYPES } from '../../shared/edit-vocabulary';
import { EXTENSION_ID } from '../../shared/extensions';
import { AGENT_TESTING_INSTRUCTIONS } from '../../shared/agent-testing';

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
Read powermove-api/BACKGROUND_TESTING.md before visual or interaction tests; it identifies the source checkout when available. Do not assume an Agent Workspace contains the app's npm scripts.

PROJECT EDITING
The current Powermove project snapshot is inputs/powermove-project.json. Treat it as read-only reference; never rewrite it. For scene edits such as layers, properties, effects, keyframes, easing, expressions, composition settings, markers, or sections, return typed commands even if you also create or change an extension.

LIVE POWERMOVE TOOLS
With \`powermove\` tools, read \`get_project_state\`. Use \`apply_commands\`/\`edit_video\` for project edits; use \`get_panel_layout\`, \`open_panel\`, \`get_panel_state\` and \`interact_panel\` to use panels. Review with \`render_frames\`. Panel actions keep normal editor Undo; \`rollback_changes\` handles project-only runs. For live edits return \`commands: []\`. Never rewrite project JSON.

ANIMATION-FIRST VALUES
Treat every user-editable project value as keyframeable by default, especially every parameter introduced by an effect, layer type, generated control, or extension. Store values in Powermove's real editable property/keyframe model, expose the normal animation controls, and use set_property or replace_keyframes commands for edits. Do not bake adjustable values into opaque code, flattened media, or a second source of truth. Only inherently structural metadata such as stable IDs, names, and file references may remain non-keyframeable.

Supported Powermove command types are: ${AGENT_COMMAND_TYPES.join(', ')}. Return each command as one JSON-encoded string.

EXTENDING POWERMOVE
The writable extension staging directory for this run is ${extensionsDir}. When the user asks to change or add Powermove functionality, create or edit extensions only under that directory. It is an isolated copy: Powermove validates the reported changes, promotes them atomically, and retains the previous live version for recovery. Never edit the app bundle. Never edit the live user-extension folder or the source checkout. The folder name must equal the extension manifest id.

The extension API pack is in powermove-api/. Read powermove-api/EXTENSIONS.md first, then use the included TypeScript API, manifest, project, and command types as needed. Prefer the smallest extension shape in this order: contribute a new capability; override an existing contribution by id; fork a built-in by copying its folder from the app's builtin extensions directory (packaged: Resources/builtin-extensions/<id>; dev: src/extensions/<id>) into the extensions directory with manifest \`replaces\` and \`forkedFrom\` ("<id>@<version>") entries. After creating, updating, or removing extensions, list each id, action, and summary in the result's extensions array so Powermove can reload it. Always return the extensions array; use [] when no extension changed. A summary may be an empty string.

DELIVERABLES AND SIDE EFFECTS
Place every non-extension deliverable file under the artifacts directory for this run: ${artifactPath}. Do not leave deliverables elsewhere. You may create project-local scripts, notes, and adapters in this workspace when they help finish the task. Files that should become editable media layers must be listed in artifacts with importToTimeline=true.

Record uploads, messages, publications, remote changes, application launches, or other outside-world side effects in externalActions. Never claim an external action succeeded unless a tool result proves it. The active authority is ${access}. Project authority limits writes to this project workspace and the isolated extension staging directory above; computer authority was explicitly granted for this run and may operate outside them when required by the user's request, but Powermove source changes must still use the staging directory.

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
        maxItems: 80,
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
