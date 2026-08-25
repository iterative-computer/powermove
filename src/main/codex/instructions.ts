import type { CodexAccess } from '../../shared/ipc';
import { AGENT_COMMAND_TYPES } from '../../shared/edit-vocabulary';

export interface AgentInstructionsOptions {
  projectName: string;
  artifactPath: string;
  access: Exclude<CodexAccess, 'editor'>;
}

export function agentInstructions({ projectName, artifactPath, access }: AgentInstructionsOptions): string {
  return `You are the general production agent working beside Powermove. Complete the user's request end to end, using web search, shell tools, installed creative applications, and reusable integrations when useful. The current editable Powermove project snapshot is inputs/powermove-project.json. Treat it as read-only reference; return Powermove edits through typed commands instead of rewriting that file.

Place every deliverable file under the artifacts directory for this run: ${artifactPath}. Do not leave deliverables elsewhere. You may create project-local scripts, notes, and adapters in this workspace when they help finish the task.

Supported Powermove command types are: ${AGENT_COMMAND_TYPES.join(', ')}. Return each command as one JSON-encoded string. Files that should become editable media layers must be listed in artifacts with importToTimeline=true.

Record uploads, messages, publications, remote changes, application launches, or other outside-world side effects in externalActions. Never claim an external action succeeded unless a tool result proves it. The active authority is ${access}. Project authority limits writes to this project workspace; computer authority was explicitly granted for this run and may operate outside it when required by the user's request.

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
}

export function agentResultSchema(): Record<string, unknown> {
  return {
    type: 'object',
    additionalProperties: false,
    required: ['summary', 'commands', 'artifacts', 'externalActions', 'notes'],
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
      notes: { type: 'array', maxItems: 80, items: { type: 'string' } }
    }
  };
}
