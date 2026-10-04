import { execFile } from 'node:child_process';
import type { AgentApproval, CodexAccess, ReasoningEffort } from '../../shared/ipc';
import { discoverCodexBinary } from './env';
import { AGENT_TESTING_INSTRUCTIONS } from '../../shared/agent-testing';
import { OUTSIDE_SANDBOX_TOOL_NAME, type NativeMcpServerConfig } from '../agent-tools/spec';

import { userMcpArgv, type UserMcpServers } from '../agent-tools/user-mcp';
import { AGENT_SHELL_NETWORK_INSTRUCTIONS } from './instructions';

export const ADAPTER_VERSION = '6';

export const REQUIRED_CODEX_FLAGS = [
  '--ephemeral',
  '--skip-git-repo-check',
  '--sandbox',
  '--output-schema',
  '--output-last-message',
  '--json',
  '--model',
  '--config',
  '--image',
  '--search',
  '--add-dir',
  '--dangerously-bypass-approvals-and-sandbox'
] as const;

/** The permission profile sandboxed Project shells run under. */
export const PROJECT_PERMISSION_PROFILE = 'powermove';

/**
 * `--approve-for-me` without its legacy `sandbox_mode="workspace-write"`,
 * which would override a permission profile: a command the sandbox blocks
 * may ask to run outside it, and Codex's automatic reviewer decides. The
 * profile extends Codex's workspace sandbox and with shell network lets
 * commands reach any host. They follow `exec` because root-level approval
 * and profile overrides do not reach it; the legacy
 * `sandbox_workspace_write.network_access` switch did not either.
 *
 * `exec` cannot ask a person, so a supervised run never escalates: Codex's
 * own approvals are off and the run asks through Powermove's
 * run_outside_sandbox tool instead.
 */
export function projectSandboxArgv(options: { shellNetwork: boolean; approval?: AgentApproval }): string[] {
  const profile = `permissions.${PROJECT_PERMISSION_PROFILE}`;
  const config = [
    ...options.approval === 'supervised'
      ? ['approval_policy="never"']
      : ['approvals_reviewer="auto_review"', 'approval_policy="on-request"'],
    `default_permissions=${JSON.stringify(PROJECT_PERMISSION_PROFILE)}`,
    `${profile}.extends=":workspace"`
  ];
  // Direct sockets to any host: no network proxy, no domain list.
  if (options.shellNetwork) config.push(`${profile}.network.enabled=true`);
  return config.flatMap(value => ['--config', value]);
}

interface CommonArgvOptions {
  schemaPath: string;
  outputPath: string;
  prompt: string;
  imagePaths: readonly string[];
  model: string | null;
  reasoningEffort: ReasoningEffort | null;
  externalMcpServers?: UserMcpServers;
}

export interface EditorArgvOptions extends CommonArgvOptions {}

export interface AutonomousArgvOptions extends CommonArgvOptions {
  access: Exclude<CodexAccess, 'editor'>;
  extensionsDir: string;
  sessionId: string | null;
  instructions: string;
  nativeTools?: NativeMcpServerConfig;
  /** Outbound network to any host for sandboxed shell commands. Only the
   * Project access choice grants it; Edit project runs keep project
   * authority without it. */
  shellNetwork?: boolean;
  /** Who approves a sandboxed command that needs to leave the sandbox. */
  approval?: AgentApproval;
}

const SUPERVISED_INSTRUCTIONS = `APPROVALS\nThe sandbox confines writes to this workspace and you cannot escalate a command yourself. When one genuinely needs to run outside it, such as installing a font or writing to the user's home folder, call ${OUTSIDE_SANDBOX_TOOL_NAME} with that one command and why. Powermove asks the user to approve it; if they decline, do not retry it.`;

function appendModelOptions(
  argv: string[],
  model: string | null,
  reasoningEffort: ReasoningEffort | null
): void {
  if (model !== null) argv.push('--model', model);
  if (reasoningEffort !== null) {
    argv.push('--config', `model_reasoning_effort="${reasoningEffort}"`);
  }
}

function appendPromptAndImages(argv: string[], prompt: string, imagePaths: readonly string[]): void {
  argv.push(prompt);
  for (const imagePath of imagePaths) argv.push('--image', imagePath);
}

function nativeMcpArgv(config?: NativeMcpServerConfig): string[] {
  if (!config) return [];
  const argv = [
    '--config', `mcp_servers.powermove.command=${JSON.stringify(config.command)}`,
    '--config', `mcp_servers.powermove.args=${JSON.stringify(config.args)}`,
    '--config', 'mcp_servers.powermove.required=true',
    // Run-scoped tools carry no annotations; approve them without a review.
    '--config', 'mcp_servers.powermove.default_tools_approval_mode="approve"',
    '--config', 'mcp_servers.powermove.startup_timeout_sec=30',
    '--config', `mcp_servers.powermove.tool_timeout_sec=${config.toolTimeoutSec ?? 120}`
  ];
  for (const [name, value] of Object.entries(config.env).sort(([a], [b]) => a.localeCompare(b))) {
    argv.push('--config', `mcp_servers.powermove.env.${name}=${JSON.stringify(value)}`);
  }
  return argv;
}

export function buildEditorArgv(options: EditorArgvOptions): string[] {
  const argv = [
    'exec',
    '--ephemeral',
    '--skip-git-repo-check',
    ...userMcpArgv(options.externalMcpServers),
    '--sandbox',
    'read-only',
    '--output-schema',
    options.schemaPath,
    '--output-last-message',
    options.outputPath,
    '--json'
  ];
  appendModelOptions(argv, options.model, options.reasoningEffort);
  appendPromptAndImages(argv, `${AGENT_TESTING_INSTRUCTIONS}\n\n${options.prompt}`, options.imagePaths);
  return argv;
}

const SESSION_ID = /^[A-Za-z0-9][A-Za-z0-9_-]{0,199}$/;

export function buildAutonomousArgv(options: AutonomousArgvOptions): string[] {
  const argv = ['--search'];
  const sandboxed = options.access !== 'computer';
  if (!sandboxed) argv.push('--dangerously-bypass-approvals-and-sandbox');
  argv.push('--add-dir', options.extensionsDir);

  const sessionId = options.sessionId?.trim() || null;
  if (sessionId !== null && !SESSION_ID.test(sessionId)) throw new Error('Invalid Codex session id.');
  argv.push('exec');
  if (sessionId !== null) argv.push('resume');
  argv.push(
    '--skip-git-repo-check',
    ...sandboxed ? projectSandboxArgv({ shellNetwork: options.shellNetwork === true, approval: options.approval }) : [],
    ...userMcpArgv(options.externalMcpServers),
    ...nativeMcpArgv(options.nativeTools),
    '--output-schema',
    options.schemaPath,
    '--output-last-message',
    options.outputPath,
    '--json'
  );
  appendModelOptions(argv, options.model, options.reasoningEffort);
  for (const imagePath of options.imagePaths) argv.push('--image', imagePath);
  // Positionals follow `--`, so neither the session id nor the prompt can read as a flag.
  argv.push('--');
  if (sessionId !== null) argv.push(sessionId);
  const network = sandboxed && options.shellNetwork ? `\n\nSHELL NETWORK\n${AGENT_SHELL_NETWORK_INSTRUCTIONS}` : '';
  const approvals = sandboxed && options.approval === 'supervised' && options.nativeTools ? `\n\n${SUPERVISED_INSTRUCTIONS}` : '';
  argv.push(`${options.instructions}${network}${approvals}\n\nUSER REQUEST\n${options.prompt}`);
  return argv;
}

export interface AdapterCapabilities {
  adapterVersion: string;
  supported: string[];
  missing: string[];
}

function execFileText(file: string, args: readonly string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(file, [...args], { encoding: 'utf8', timeout: 5_000, maxBuffer: 2 * 1024 * 1024 }, (error, stdout) => {
      if (error) reject(error);
      else resolve(stdout);
    });
  });
}

export async function capabilities(binary?: string | null): Promise<AdapterCapabilities> {
  const executable = binary ?? await discoverCodexBinary(null);
  const help = await execFileText(executable, ['exec', '--help']);
  const supported = REQUIRED_CODEX_FLAGS.filter((flag) => help.includes(flag));
  const missing = REQUIRED_CODEX_FLAGS.filter((flag) => !help.includes(flag));
  return { adapterVersion: ADAPTER_VERSION, supported: [...supported], missing: [...missing] };
}
