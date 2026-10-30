import { execFile } from 'node:child_process';
import { chmodSync, constants, copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import type { AgentClientId, AgentClientStatus } from '../shared/agent-clients';

/* Any agent app that speaks MCP can drive Powermove with its own
   subscription: it starts `Powermove --powermove-mcp` and gets the same tools
   the built-in agent uses. This adds that one entry to each app's own MCP
   settings, and takes it out again. It never replaces an entry someone set up
   by hand, and never touches the app's sign-in. */

export const MCP_FLAG = '--powermove-mcp';
const SERVER = 'powermove';
type JsonObject = Record<string, unknown>;
const object = (value: unknown): value is JsonObject => !!value && typeof value === 'object' && !Array.isArray(value);

type Format =
  /** `{ mcpServers: { name: { command, args } } }`: Claude, Cursor, Gemini CLI, Windsurf, LM Studio. */
  | 'mcpServers'
  /** VS Code: `{ servers: { name: { type: 'stdio', command, args } } }`. */
  | 'vscode'
  /** OpenCode: `{ mcp: { name: { type: 'local', command: [...], enabled } } }`. */
  | 'opencode'
  /** Codex: a `[mcp_servers.name]` table in config.toml. */
  | 'codex'
  /** Claude Code keeps user servers in ~/.claude.json, which running sessions
   * rewrite; its own `claude mcp` command edits it safely. */
  | 'claude-code';

interface ClientSpec {
  id: AgentClientId;
  name: string;
  /** Whose plan it runs on, in a few words. */
  detail: string;
  /** Config file, relative to the home folder. */
  config: string;
  format: Format;
  /** Paths (relative to home, or absolute) whose presence means it is installed. */
  installed: string[];
  /** What to do after adding, when the app reads its settings only at launch. */
  restart?: string;
}

const CLIENTS: readonly ClientSpec[] = [
  { id: 'claude-code', name: 'Claude Code', detail: 'Your Claude plan, in the terminal or your editor.', config: '.claude.json', format: 'claude-code', installed: ['.claude.json', '.claude'], restart: 'Start a new Claude Code session to use it.' },
  { id: 'codex', name: 'Codex', detail: 'Your ChatGPT plan, in the Codex app or CLI.', config: '.codex/config.toml', format: 'codex', installed: ['.codex', '/Applications/Codex.app'], restart: 'Start a new Codex session to use it.' },
  { id: 'claude-desktop', name: 'Claude', detail: 'Your Claude plan, in the Claude app.', config: 'Library/Application Support/Claude/claude_desktop_config.json', format: 'mcpServers', installed: ['/Applications/Claude.app', 'Applications/Claude.app', 'Library/Application Support/Claude'], restart: 'Quit and reopen Claude to use it.' },
  { id: 'gemini-cli', name: 'Gemini CLI', detail: 'Your Google account or Gemini plan.', config: '.gemini/settings.json', format: 'mcpServers', installed: ['.gemini'], restart: 'Start a new Gemini CLI session to use it.' },
  { id: 'cursor', name: 'Cursor', detail: 'Your Cursor plan.', config: '.cursor/mcp.json', format: 'mcpServers', installed: ['/Applications/Cursor.app', '.cursor'], restart: 'Restart Cursor to use it.' },
  { id: 'vscode', name: 'VS Code', detail: 'GitHub Copilot’s agent mode.', config: 'Library/Application Support/Code/User/mcp.json', format: 'vscode', installed: ['/Applications/Visual Studio Code.app', 'Library/Application Support/Code/User'], restart: 'Reload VS Code to use it.' },
  { id: 'windsurf', name: 'Windsurf', detail: 'Your Windsurf plan.', config: '.codeium/windsurf/mcp_config.json', format: 'mcpServers', installed: ['/Applications/Windsurf.app', '.codeium/windsurf'], restart: 'Restart Windsurf to use it.' },
  { id: 'lm-studio', name: 'LM Studio', detail: 'Models on this Mac, in LM Studio’s chat.', config: '.lmstudio/mcp.json', format: 'mcpServers', installed: ['/Applications/LM Studio.app', '.lmstudio'], restart: 'Turn Powermove on under Integrations in LM Studio’s chat.' },
  { id: 'opencode', name: 'OpenCode', detail: 'Any provider you use in OpenCode.', config: '.config/opencode/opencode.json', format: 'opencode', installed: ['.config/opencode'], restart: 'Start a new OpenCode session to use it.' }
];

export interface AgentClientEnvironment {
  homeDir: string;
  /** The Powermove executable other apps should start. */
  executablePath: string;
  /** Where Claude Desktop's one-time registration receipt lives. */
  userDataDir: string;
  /** Runs Claude Code's own CLI; resolves with its exit code and output. */
  runClaude?: (args: string[]) => Promise<{ code: number; output: string }>;
}

const resolve = (env: AgentClientEnvironment, file: string): string => path.isAbsolute(file) ? file : path.join(env.homeDir, file);
function exists(file: string): boolean {
  try { lstatSync(file); return true; } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false;
    throw error;
  }
}
function readText(file: string): string {
  const stat = lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 16 * 1024 * 1024) throw new Error('Unsupported configuration');
  return readFileSync(file, 'utf8');
}
/* Strict JSON only. A file with comments (VS Code and OpenCode allow them)
   would lose them on rewrite, so it reads as "set it up yourself". */
function readObject(file: string): JsonObject {
  if (!exists(file)) return {};
  const text = readText(file);
  if (!text.trim()) return {};
  const value: unknown = JSON.parse(text);
  if (!object(value)) throw new Error('Invalid configuration');
  return value;
}
function writeText(file: string, text: string): void {
  mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  // One private recovery copy of the person's own file, as it was before Powermove.
  const backup = `${file}.before-powermove`;
  if (exists(file) && !exists(backup)) {
    copyFileSync(file, backup, constants.COPYFILE_EXCL);
    chmodSync(backup, 0o600);
  }
  const mode = exists(file) ? lstatSync(file).mode & 0o777 : 0o600;
  const temporary = `${file}.${randomUUID()}.tmp`;
  try {
    writeFileSync(temporary, text, { mode, flag: 'wx' });
    renameSync(temporary, file);
  } finally {
    if (existsSync(temporary)) unlinkSync(temporary);
  }
}
const writeObject = (file: string, value: JsonObject): void => writeText(file, `${JSON.stringify(value, null, 2)}\n`);

const isPowermoveCommand = (command: unknown): boolean =>
  typeof command === 'string' && /\/Powermove(?: [^/]*)?\.app\/Contents\/MacOS\/Powermove$/.test(command);
const onlyFlag = (args: unknown): boolean => Array.isArray(args) && args.length === 1 && args[0] === MCP_FLAG;

/** The JSON object an app keeps its servers in, and the entry shape it expects. */
function serversKey(format: Format): string {
  return format === 'vscode' ? 'servers' : format === 'opencode' ? 'mcp' : 'mcpServers';
}
function entryFor(format: Format, command: string): JsonObject {
  if (format === 'vscode') return { type: 'stdio', command, args: [MCP_FLAG] };
  if (format === 'opencode') return { type: 'local', command: [command, MCP_FLAG], enabled: true };
  return { command, args: [MCP_FLAG] };
}
/** Whether an existing entry is one Powermove wrote (so it may update or remove it). */
function ownEntry(format: Format, entry: unknown): boolean {
  if (!object(entry)) return false;
  if (format === 'opencode') {
    return Array.isArray(entry.command) && entry.command.length === 2 && isPowermoveCommand(entry.command[0]) && entry.command[1] === MCP_FLAG;
  }
  const allowed = format === 'vscode' || format === 'claude-code' ? ['type', 'command', 'args', 'env'] : ['command', 'args'];
  return isPowermoveCommand(entry.command) && onlyFlag(entry.args)
    && Object.keys(entry).every(key => allowed.includes(key))
    && (entry.env === undefined || object(entry.env) && Object.keys(entry.env).length === 0);
}

/* Codex: config.toml is edited as text, appending or removing exactly the
   table Powermove writes. Any other mention of the server is left alone. */
const CODEX_HEADER = /^[ \t]*\[mcp_servers\.(?:powermove|"powermove"|'powermove')\][ \t]*(?:#.*)?$/m;
const tomlString = (value: string): string => JSON.stringify(value);
function codexBlock(command: string): string {
  return `[mcp_servers.${SERVER}]\ncommand = ${tomlString(command)}\nargs = [${tomlString(MCP_FLAG)}]\n# The hidden editor takes a while to start the first time.\nstartup_timeout_sec = 60\n`;
}
/** The table's lines, from its header to the next table. */
function codexTable(text: string): { start: number; end: number; body: string } | null {
  const match = CODEX_HEADER.exec(text);
  if (!match) return null;
  const start = match.index;
  const rest = text.slice(start + match[0].length);
  const next = /^[ \t]*\[/m.exec(rest);
  const end = next ? start + match[0].length + next.index : text.length;
  return { start, end, body: text.slice(start, end) };
}
function codexMentioned(text: string): boolean {
  return CODEX_HEADER.test(text) || /mcp_servers\.["']?powermove["']?\./.test(text)
    || /^\s*\[mcp_servers\][\s\S]*?^\s*["']?powermove["']?\s*=/m.test(text);
}
function codexOwn(body: string): boolean {
  const command = /^\s*command\s*=\s*"([^"\\]*(?:\\.[^"\\]*)*)"\s*$/m.exec(body);
  const args = /^\s*args\s*=\s*\[\s*"--powermove-mcp"\s*,?\s*\]\s*$/m.exec(body);
  const keys = body.split('\n').slice(1).map(line => line.trim()).filter(line => line && !line.startsWith('#')).map(line => line.split('=')[0]!.trim());
  return !!command && isPowermoveCommand(JSON.parse(`"${command[1]}"`)) && !!args
    && keys.every(key => ['command', 'args', 'startup_timeout_sec'].includes(key));
}

function stateOf(spec: ClientSpec, env: AgentClientEnvironment): Pick<AgentClientStatus, 'state' | 'custom'> {
  const file = resolve(env, spec.config);
  if (spec.format === 'codex') {
    const text = exists(file) ? readText(file) : '';
    if (!codexMentioned(text)) return { state: 'available', custom: false };
    const table = codexTable(text);
    return { state: 'connected', custom: !table || !codexOwn(table.body) };
  }
  const config = readObject(file);
  const servers = config[serversKey(spec.format === 'claude-code' ? 'mcpServers' : spec.format)];
  if (servers !== undefined && !object(servers)) return { state: 'unreadable', custom: false };
  const entry = object(servers) ? servers[SERVER] : undefined;
  if (entry === undefined) return { state: 'available', custom: false };
  return { state: 'connected', custom: !ownEntry(spec.format, entry) };
}

export function listAgentClients(env: AgentClientEnvironment): AgentClientStatus[] {
  return CLIENTS.map(spec => {
    const base = { id: spec.id, name: spec.name, detail: spec.detail, configPath: resolve(env, spec.config) };
    let installed = false;
    try { installed = spec.installed.some(item => exists(resolve(env, item))); } catch { /* unreadable home */ }
    if (!installed) return { ...base, state: 'not-installed' as const, custom: false };
    try { return { ...base, ...stateOf(spec, env) }; }
    catch { return { ...base, state: 'unreadable' as const, custom: false }; }
  });
}

function spec(id: AgentClientId): ClientSpec {
  const found = CLIENTS.find(item => item.id === id);
  if (!found) throw new Error('Unknown agent app.');
  return found;
}

const unreadable = (name: string, file: string) =>
  new Error(`Powermove can’t safely edit ${name}’s settings (${file}), for example because they contain comments. Use Copy setup and add it there yourself.`);

async function runClaudeMcp(env: AgentClientEnvironment, args: string[]): Promise<void> {
  if (!env.runClaude) throw new Error('Claude Code isn’t available. Install it, then try again.');
  const { code, output } = await env.runClaude(['mcp', ...args]);
  if (code !== 0) throw new Error(output.trim().split('\n').at(-1)?.slice(0, 240) || 'Claude Code could not change its settings.');
}

/** Adds Powermove to an agent app. Returns what to do next. */
export async function connectAgentClient(id: AgentClientId, env: AgentClientEnvironment): Promise<string> {
  const client = spec(id);
  if (!path.isAbsolute(env.executablePath)) throw new Error('Powermove’s location is unknown.');
  const file = resolve(env, client.config);
  if (client.format === 'claude-code') {
    const current = stateOf(client, env);
    if (current.state === 'connected' && current.custom) return 'Already set up.';
    // An outdated entry (Powermove moved) is replaced; `add` refuses a duplicate.
    if (current.state === 'connected') await runClaudeMcp(env, ['remove', '--scope', 'user', SERVER]);
    await runClaudeMcp(env, ['add', '--scope', 'user', SERVER, '--', env.executablePath, MCP_FLAG]);
    return client.restart ?? 'Added.';
  }
  if (client.format === 'codex') {
    const text = exists(file) ? readText(file) : '';
    const table = codexTable(text);
    if (codexMentioned(text) && !(table && codexOwn(table.body))) return 'Already set up.';
    const without = table ? text.slice(0, table.start) + text.slice(table.end) : text;
    const base = without.replace(/\s*$/, '');
    writeText(file, `${base}${base ? '\n\n' : ''}${codexBlock(env.executablePath)}`);
    return client.restart ?? 'Added.';
  }
  let config: JsonObject;
  try { config = readObject(file); } catch { throw unreadable(client.name, file); }
  const key = serversKey(client.format);
  if (config[key] !== undefined && !object(config[key])) throw unreadable(client.name, file);
  const servers = object(config[key]) ? config[key] as JsonObject : {};
  if (servers[SERVER] !== undefined && !ownEntry(client.format, servers[SERVER])) return 'Already set up.';
  servers[SERVER] = entryFor(client.format, env.executablePath);
  config[key] = servers;
  writeObject(file, config);
  return client.restart ?? 'Added.';
}

/** Takes Powermove out of an agent app, unless someone set the entry up by hand. */
export async function disconnectAgentClient(id: AgentClientId, env: AgentClientEnvironment): Promise<void> {
  const client = spec(id);
  const file = resolve(env, client.config);
  const current = stateOf(client, env);
  if (current.state !== 'connected') return;
  if (current.custom) throw new Error(`This connection was set up by hand. Remove it in ${client.name}’s settings.`);
  if (client.format === 'claude-code') {
    await runClaudeMcp(env, ['remove', '--scope', 'user', SERVER]);
    return;
  }
  if (client.format === 'codex') {
    const text = readText(file);
    const table = codexTable(text)!;
    writeText(file, `${(text.slice(0, table.start) + text.slice(table.end)).replace(/\n{3,}/g, '\n\n').replace(/\s*$/, '')}\n`);
    return;
  }
  const config = readObject(file);
  const servers = config[serversKey(client.format)] as JsonObject;
  delete servers[SERVER];
  writeObject(file, config);
  if (id === 'claude-desktop') {
    // Removing it here is a deliberate disconnect: launch-time setup must not add it back.
    const receipt = path.join(env.userDataDir, 'mcp', 'claude-desktop-registration.json');
    mkdirSync(path.dirname(receipt), { recursive: true, mode: 0o700 });
    writeFileSync(receipt, `${JSON.stringify({ registered: true }, null, 2)}\n`, { mode: 0o600 });
  }
}

/** Runs a CLI with no shell, capped in time and output. */
export function runCli(binary: string, args: string[], env: NodeJS.ProcessEnv): Promise<{ code: number; output: string }> {
  return new Promise(done => {
    execFile(binary, args, { env, timeout: 60_000, maxBuffer: 1024 * 1024 }, (error, stdout, stderr) => {
      const code = error ? typeof (error as { code?: unknown }).code === 'number' ? (error as { code: number }).code : 1 : 0;
      done({ code, output: `${stdout}\n${stderr}` });
    });
  });
}
