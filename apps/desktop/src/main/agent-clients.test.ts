import { afterEach, describe, expect, it, vi } from 'vitest';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { connectAgentClient, disconnectAgentClient, listAgentClients, type AgentClientEnvironment } from './agent-clients';

const EXE = '/Applications/Powermove.app/Contents/MacOS/Powermove';
const homes: string[] = [];
afterEach(async () => { for (const dir of homes.splice(0)) await rm(dir, { recursive: true, force: true }); });

async function environment(runClaude?: AgentClientEnvironment['runClaude']): Promise<AgentClientEnvironment> {
  const homeDir = await mkdtemp(path.join(os.tmpdir(), 'pm-agent-clients-'));
  homes.push(homeDir);
  return { homeDir, userDataDir: path.join(homeDir, 'userData'), executablePath: EXE, runClaude };
}
const json = async (file: string) => JSON.parse(await readFile(file, 'utf8'));
const state = (env: AgentClientEnvironment, id: string) => listAgentClients(env).find(client => client.id === id)!;

describe('adding Powermove to other agent apps', () => {
  it('lists only what is installed, and adds Powermove next to the person’s own servers', async () => {
    const env = await environment();
    expect(state(env, 'gemini-cli').state).toBe('not-installed');
    const settings = path.join(env.homeDir, '.gemini', 'settings.json');
    await mkdir(path.dirname(settings), { recursive: true });
    await writeFile(settings, JSON.stringify({ theme: 'Dracula', mcpServers: { github: { command: 'gh-mcp' } } }));
    expect(state(env, 'gemini-cli')).toMatchObject({ state: 'available', custom: false });

    expect(await connectAgentClient('gemini-cli', env)).toContain('new Gemini CLI session');
    expect(await json(settings)).toEqual({ theme: 'Dracula', mcpServers: { github: { command: 'gh-mcp' }, powermove: { command: EXE, args: ['--powermove-mcp'] } } });
    expect(JSON.parse(await readFile(`${settings}.before-powermove`, 'utf8')).mcpServers.powermove).toBeUndefined();
    expect(state(env, 'gemini-cli')).toMatchObject({ state: 'connected', custom: false });

    await disconnectAgentClient('gemini-cli', env);
    expect(await json(settings)).toEqual({ theme: 'Dracula', mcpServers: { github: { command: 'gh-mcp' } } });
  });

  it('writes each app’s own shape: VS Code servers and OpenCode commands', async () => {
    const env = await environment();
    await mkdir(path.join(env.homeDir, 'Library/Application Support/Code/User'), { recursive: true });
    await mkdir(path.join(env.homeDir, '.config/opencode'), { recursive: true });
    await connectAgentClient('vscode', env);
    await connectAgentClient('opencode', env);
    expect(await json(path.join(env.homeDir, 'Library/Application Support/Code/User/mcp.json')))
      .toEqual({ servers: { powermove: { type: 'stdio', command: EXE, args: ['--powermove-mcp'] } } });
    expect(await json(path.join(env.homeDir, '.config/opencode/opencode.json')))
      .toEqual({ mcp: { powermove: { type: 'local', command: [EXE, '--powermove-mcp'], enabled: true } } });
    expect(state(env, 'vscode').state).toBe('connected');
    expect(state(env, 'opencode').state).toBe('connected');
  });

  it('never replaces or removes an entry someone set up by hand', async () => {
    const env = await environment();
    const file = path.join(env.homeDir, '.cursor', 'mcp.json');
    await mkdir(path.dirname(file), { recursive: true });
    const custom = { mcpServers: { powermove: { command: EXE, args: ['--powermove-mcp', '--user-data', '/tmp/x'] } } };
    await writeFile(file, JSON.stringify(custom));
    expect(state(env, 'cursor')).toMatchObject({ state: 'connected', custom: true });
    expect(await connectAgentClient('cursor', env)).toBe('Already set up.');
    await expect(disconnectAgentClient('cursor', env)).rejects.toThrow('set up by hand');
    expect(await json(file)).toEqual(custom);
  });

  it('leaves settings with comments alone and points to Copy setup', async () => {
    const env = await environment();
    const file = path.join(env.homeDir, '.lmstudio', 'mcp.json');
    await mkdir(path.dirname(file), { recursive: true });
    await writeFile(file, '{\n  // my servers\n  "mcpServers": {}\n}\n');
    expect(state(env, 'lm-studio').state).toBe('unreadable');
    await expect(connectAgentClient('lm-studio', env)).rejects.toThrow('Copy setup');
    expect(await readFile(file, 'utf8')).toContain('// my servers');
  });

  it('appends and removes exactly its own Codex table', async () => {
    const env = await environment();
    const file = path.join(env.homeDir, '.codex', 'config.toml');
    await mkdir(path.dirname(file), { recursive: true });
    const before = 'model = "gpt-5"\n\n[mcp_servers.github]\ncommand = "gh-mcp"\n\n[profiles.fast]\nmodel = "mini"\n';
    await writeFile(file, before);
    expect(state(env, 'codex').state).toBe('available');
    await connectAgentClient('codex', env);
    const added = await readFile(file, 'utf8');
    expect(added.startsWith(before.trimEnd())).toBe(true);
    expect(added).toContain(`[mcp_servers.powermove]\ncommand = "${EXE}"\nargs = ["--powermove-mcp"]`);
    expect(added).toContain('startup_timeout_sec = 60');
    expect(state(env, 'codex')).toMatchObject({ state: 'connected', custom: false });
    // Connecting again after Powermove moves updates the one table.
    await connectAgentClient('codex', { ...env, executablePath: '/Users/me/Applications/Powermove.app/Contents/MacOS/Powermove' });
    expect((await readFile(file, 'utf8')).match(/mcp_servers\.powermove/g)).toHaveLength(1);
    await disconnectAgentClient('codex', env);
    expect(await readFile(file, 'utf8')).toBe(before);
  });

  it('recognizes the Codex table people commonly write, and leaves custom ones alone', async () => {
    const env = await environment();
    const file = path.join(env.homeDir, '.codex', 'config.toml');
    await mkdir(path.dirname(file), { recursive: true });
    await writeFile(file, `[mcp_servers.powermove]\ncommand = "${EXE}"\nargs = [ "--powermove-mcp" ]\nstartup_timeout_sec = 60\n`);
    expect(state(env, 'codex')).toMatchObject({ state: 'connected', custom: false });
    await writeFile(file, `[mcp_servers.powermove]\ncommand = "${EXE}"\nargs = ["--powermove-mcp"]\nenv = { DEBUG = "1" }\n`);
    expect(state(env, 'codex')).toMatchObject({ state: 'connected', custom: true });
    expect(await connectAgentClient('codex', env)).toBe('Already set up.');
  });

  it('uses Claude Code’s own command, since running sessions rewrite its settings file', async () => {
    const calls: string[][] = [];
    const env = await environment(vi.fn(async (args: string[]) => { calls.push(args); return { code: 0, output: '' }; }));
    await writeFile(path.join(env.homeDir, '.claude.json'), JSON.stringify({ mcpServers: {} }));
    expect(state(env, 'claude-code').state).toBe('available');
    await connectAgentClient('claude-code', env);
    expect(calls).toEqual([['mcp', 'add', '--scope', 'user', 'powermove', '--', EXE, '--powermove-mcp']]);
    // What `claude mcp add` writes reads as Powermove's own entry.
    await writeFile(path.join(env.homeDir, '.claude.json'), JSON.stringify({ mcpServers: { powermove: { type: 'stdio', command: EXE, args: ['--powermove-mcp'], env: {} } } }));
    expect(state(env, 'claude-code')).toMatchObject({ state: 'connected', custom: false });
    await disconnectAgentClient('claude-code', env);
    expect(calls.at(-1)).toEqual(['mcp', 'remove', '--scope', 'user', 'powermove']);
  });

  it('keeps Claude disconnected after Remove, so launch-time setup does not add it back', async () => {
    const env = await environment();
    await mkdir(path.join(env.homeDir, 'Library/Application Support/Claude'), { recursive: true });
    await connectAgentClient('claude-desktop', env);
    await disconnectAgentClient('claude-desktop', env);
    const receipt = path.join(env.userDataDir, 'mcp', 'claude-desktop-registration.json');
    expect(existsSync(receipt)).toBe(true);
    expect(await json(receipt)).toEqual({ registered: true });
  });
});
