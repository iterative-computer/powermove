import type { SandboxSettings } from '@anthropic-ai/claude-agent-sdk';

import type { CodexAccess, ReasoningEffort } from '../../shared/ipc';
import { AGENT_TESTING_INSTRUCTIONS } from '../../shared/agent-testing';
import { modelEffort } from '../../shared/agent-models';
import {
  POWERMOVE_LIVE_INSPECTION_MCP_TOOL_NAMES,
  POWERMOVE_MCP_TOOL_NAMES,
  type NativeMcpServerConfig
} from '../agent-tools/spec';

import type { UserMcpServers } from '../agent-tools/user-mcp';
import { AGENT_SHELL_NETWORK_INSTRUCTIONS } from '../codex/instructions';

const PROJECT_TOOLS = 'Read,Glob,Grep,Write,Edit,Bash,WebSearch,WebFetch,Skill,Agent,Task';
const EDITOR_TOOLS = 'Read,Glob,Grep,Skill,Agent,Task';

const STRICT_SANDBOX_SETTINGS = {
  enabled: true,
  autoAllowBashIfSandboxed: true,
  allowUnsandboxedCommands: false,
  failIfUnavailable: true
} satisfies SandboxSettings;

// Sandboxed Bash would still be auto-approved under autoAllowBashIfSandboxed,
// so Edit runs turn that off and deny every shell and file-writing tool.
const EDITOR_SANDBOX_SETTINGS = { ...STRICT_SANDBOX_SETTINGS, autoAllowBashIfSandboxed: false } satisfies SandboxSettings;
const EDITOR_SANDBOX = JSON.stringify({ sandbox: EDITOR_SANDBOX_SETTINGS });
const EDITOR_DISALLOWED_TOOLS = 'Bash,Monitor,PowerShell,Write,Edit,NotebookEdit';

// Project Bash reaches any host: the sandbox proxy matches a bare `*` against
// every host, and without an allowlist a print run denies each host it would
// ask about. Local binding lets it run dev servers. Writes stay confined.
const PROJECT_SANDBOX_SETTINGS = {
  ...STRICT_SANDBOX_SETTINGS,
  network: { allowedDomains: ['*'], allowLocalBinding: true }
} satisfies SandboxSettings;
const PROJECT_SANDBOX = JSON.stringify({ sandbox: PROJECT_SANDBOX_SETTINGS });

const PROJECT_NETWORK_INSTRUCTIONS = `SHELL NETWORK\n${AGENT_SHELL_NETWORK_INSTRUCTIONS}`;

interface ClaudeArgvOptions {
  platform?: NodeJS.Platform;
  schema: Record<string, unknown>;
  prompt: string;
  imagePaths: readonly string[];
  model: string | null;
  reasoningEffort: ReasoningEffort | null;
  sessionId: string | null;
  access: CodexAccess;
  extensionsDir?: string;
  instructions?: string;
  additionalInstructions?: string;
  nativeTools?: NativeMcpServerConfig;
  externalMcpServers?: UserMcpServers;
}

function promptWithImages(prompt: string, imagePaths: readonly string[]): string {
  if (imagePaths.length === 0) return prompt;
  return `${prompt}\n\nREFERENCE IMAGES\nInspect these files with the Read tool:\n${imagePaths.map((file) => `- ${file}`).join('\n')}`;
}

/** The prompt, as the one stream-json user message written to the CLI's stdin. */
export function claudeUserMessage(prompt: string, imagePaths: readonly string[]): string {
  return `${JSON.stringify({
    type: 'user',
    session_id: '',
    parent_tool_use_id: null,
    message: { role: 'user', content: [{ type: 'text', text: promptWithImages(prompt, imagePaths) }] }
  })}\n`;
}

/** CLI arguments intentionally use only documented Claude Code flags. The
 * bundled executable remains Anthropic's unmodified native distribution.
 * The prompt goes to stdin (`claudeUserMessage`): stream-json input keeps a
 * channel open for permission prompts, which is how AskUserQuestion reaches
 * the person. Every other prompt is denied by the runner, so no tool gains
 * approval this way. */
export function buildClaudeArgv(options: ClaudeArgvOptions): string[] {
  const windows = (options.platform ?? process.platform) === 'win32';
  const external = Object.fromEntries(Object.entries(options.externalMcpServers ?? {}).filter(([name]) => name !== 'powermove'));
  const mcpConfig = JSON.stringify({ mcpServers: {
    ...external,
    ...(options.nativeTools ? { powermove: { type: 'stdio', ...options.nativeTools } } : {})
  } });
  const externalTools = Object.keys(external).map(name => `mcp__${name}__*`);
  const withExternal = (tools: string): string => [tools, ...externalTools].join(',');
  const projectTools = options.nativeTools
    ? `${PROJECT_TOOLS},${POWERMOVE_MCP_TOOL_NAMES.join(',')}`
    : PROJECT_TOOLS;
  const editorTools = options.nativeTools
    ? `${EDITOR_TOOLS},${POWERMOVE_LIVE_INSPECTION_MCP_TOOL_NAMES.join(',')}`
    : EDITOR_TOOLS;
  const argv = [
    '--print',
    '--input-format', 'stream-json',
    '--output-format', 'stream-json',
    '--permission-prompt-tool', 'stdio',
    '--include-partial-messages',
    '--verbose',
    '--mcp-config', mcpConfig,
    '--json-schema', JSON.stringify(options.schema)
  ];

  if (options.model) argv.push('--model', options.model);
  const effort = modelEffort('claude', options.model, options.reasoningEffort);
  if (effort) argv.push('--effort', effort);
  if (options.access === 'editor') argv.push('--no-session-persistence');
  if (options.sessionId) argv.push('--resume', options.sessionId);

  if (options.access === 'computer') {
    argv.push('--dangerously-skip-permissions', '--tools', 'default');
  } else {
    argv.push(
      // dontAsk would deny AskUserQuestion before it is asked. `default` sends
      // it (and anything unlisted) to the permission prompt, which the runner
      // answers: questions go to the person, everything else is denied.
      '--permission-mode', options.access === 'editor' ? 'default' : 'acceptEdits',
      // Windows Project runs are enclosed by Codex's sandbox in the runner.
      '--settings', windows ? JSON.stringify({ sandbox: { enabled: false } }) : options.access === 'editor' ? EDITOR_SANDBOX : PROJECT_SANDBOX,
      '--tools', 'default',
      '--allowedTools', withExternal(options.access === 'editor' ? editorTools : windows ? `${projectTools},PowerShell` : projectTools)
    );
    if (options.access === 'editor') argv.push('--disallowedTools', EDITOR_DISALLOWED_TOOLS);
  }

  if (options.extensionsDir) argv.push('--add-dir', options.extensionsDir);
  const network = options.access === 'project' ? `\n\n${PROJECT_NETWORK_INSTRUCTIONS}` : '';
  const systemPrompt = options.instructions
    ? `${options.instructions}${network}\n\nReturn the final answer only through the requested JSON schema.`
    : `${AGENT_TESTING_INSTRUCTIONS}\n\nUse the supplied reference files as read-only context. Return only a value matching the requested JSON schema.`;
  argv.push('--system-prompt', systemPrompt + (options.additionalInstructions ? `\n\n${options.additionalInstructions}` : ''));
  return argv;
}

export const CLAUDE_PROJECT_SANDBOX_SETTINGS = PROJECT_SANDBOX;
export const CLAUDE_EDITOR_SANDBOX_SETTINGS = EDITOR_SANDBOX;
