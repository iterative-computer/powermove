import type { CodexAccess, ReasoningEffort } from '../../shared/ipc';
import { AGENT_TESTING_INSTRUCTIONS } from '../../shared/agent-testing';

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
}

function promptWithImages(prompt: string, imagePaths: readonly string[]): string {
  if (imagePaths.length === 0) return prompt;
  return `${prompt}\n\nREFERENCE IMAGES\nInspect these files with the Read tool:\n${imagePaths.map((file) => `- ${file}`).join('\n')}`;
}

/** CLI arguments intentionally use only documented Claude Code flags. The
 * bundled executable remains Anthropic's unmodified native distribution. */
export function buildClaudeArgv(options: ClaudeArgvOptions): string[] {
  const argv = [
    '--print',
    '--output-format', 'stream-json',
    '--verbose',
    '--safe-mode',
    '--setting-sources', '',
    '--strict-mcp-config',
    '--mcp-config', '{"mcpServers":{}}',
    '--no-chrome',
    '--disable-slash-commands',
    '--json-schema', JSON.stringify(options.schema)
  ];

  if (options.model) argv.push('--model', options.model);
  if (options.reasoningEffort) argv.push('--effort', options.reasoningEffort);
  if (options.access === 'editor') argv.push('--no-session-persistence');
  if (options.sessionId) argv.push('--resume', options.sessionId);

  if (options.access === 'computer') {
    argv.push('--dangerously-skip-permissions', '--tools', 'default');
  } else {
    argv.push(
      '--permission-mode', options.access === 'editor' ? 'dontAsk' : 'acceptEdits',
      '--settings', STRICT_SANDBOX,
      '--tools', options.access === 'editor' ? EDITOR_TOOLS : PROJECT_TOOLS,
      '--allowedTools', options.access === 'editor' ? EDITOR_TOOLS : PROJECT_TOOLS
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
