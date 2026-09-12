import type { CodexAccess, ReasoningEffort } from '../../shared/ipc';
import { AGENT_TESTING_INSTRUCTIONS } from '../../shared/agent-testing';
import { modelEffort } from '../../shared/agent-models';
import { POWERMOVE_MCP_TOOL_NAMES, type NativeMcpServerConfig } from '../agent-tools/spec';

const PROJECT_TOOLS = 'Read,Glob,Grep,Write,Edit,Bash,WebSearch,WebFetch';
const EDITOR_TOOLS = 'Read,Glob,Grep';

const STRICT_SANDBOX = JSON.stringify({
  sandbox: {
    enabled: true,
    autoAllowBashIfSandboxed: true,
    allowUnsandboxedCommands: false,
    failIfUnavailable: true
  }
});

interface ClaudeArgvOptions {
  schema: Record<string, unknown>;
  prompt: string;
  imagePaths: readonly string[];
  model: string | null;
  reasoningEffort: ReasoningEffort | null;
  sessionId: string | null;
  access: CodexAccess;
  extensionsDir?: string;
  instructions?: string;
  nativeTools?: NativeMcpServerConfig;
}

function promptWithImages(prompt: string, imagePaths: readonly string[]): string {
  if (imagePaths.length === 0) return prompt;
  return `${prompt}\n\nREFERENCE IMAGES\nInspect these files with the Read tool:\n${imagePaths.map((file) => `- ${file}`).join('\n')}`;
}

/** CLI arguments intentionally use only documented Claude Code flags. The
 * bundled executable remains Anthropic's unmodified native distribution. */
export function buildClaudeArgv(options: ClaudeArgvOptions): string[] {
  const mcpConfig = options.nativeTools
    ? JSON.stringify({ mcpServers: { powermove: { type: 'stdio', ...options.nativeTools } } })
    : '{"mcpServers":{}}';
  const projectTools = options.nativeTools
    ? `${PROJECT_TOOLS},${POWERMOVE_MCP_TOOL_NAMES.join(',')}`
    : PROJECT_TOOLS;
  const argv = [
    '--print',
    '--output-format', 'stream-json',
    '--verbose',
    '--safe-mode',
    '--setting-sources', '',
    '--strict-mcp-config',
    '--mcp-config', mcpConfig,
    '--no-chrome',
    '--disable-slash-commands',
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
      '--permission-mode', options.access === 'editor' ? 'dontAsk' : 'acceptEdits',
      '--settings', STRICT_SANDBOX,
      '--tools', options.access === 'editor' ? EDITOR_TOOLS : projectTools,
      '--allowedTools', options.access === 'editor' ? EDITOR_TOOLS : projectTools
    );
  }

  if (options.extensionsDir) argv.push('--add-dir', options.extensionsDir);
  const systemPrompt = options.instructions
    ? `${options.instructions}\n\nReturn the final answer only through the requested JSON schema.`
    : `${AGENT_TESTING_INSTRUCTIONS}\n\nUse the supplied reference files as read-only context. Return only a value matching the requested JSON schema.`;
  argv.push('--system-prompt', systemPrompt);
  argv.push(promptWithImages(options.prompt, options.imagePaths));
  return argv;
}

export const CLAUDE_PROJECT_SANDBOX_SETTINGS = STRICT_SANDBOX;
