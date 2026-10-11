import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { registerClaudeDesktopMcp } from './claude-desktop-mcp';

describe('automatic Claude desktop connection', () => {
  let root: string, directory: string, file: string;
  let options: Parameters<typeof registerClaudeDesktopMcp>[0];
  beforeEach(() => {
    root = mkdtempSync(path.join(os.tmpdir(), 'powermove-claude-registration-'));
    options = { homeDir: path.join(root, 'home'), userDataDir: path.join(root, 'profile'), executablePath: '/Applications/Powermove.app/Contents/MacOS/Powermove', claudeInstalled: false };
    directory = path.join(options.homeDir, 'Library/Application Support/Claude');
    file = path.join(directory, 'claude_desktop_config.json');
  });
  afterEach(() => rmSync(root, { recursive: true, force: true }));
  const installed = () => mkdirSync(directory, { recursive: true });
  const config = () => JSON.parse(readFileSync(file, 'utf8'));

  it('waits until Claude is present', () => {
    expect(registerClaudeDesktopMcp(options)).toBe('unavailable');
    expect(existsSync(directory)).toBe(false);
    installed();
    expect(registerClaudeDesktopMcp(options)).toBe('registered');
    expect(config().mcpServers.powermove).toEqual({ command: options.executablePath, args: ['--powermove-mcp'] });
  });

  it('connects an installed Claude app before its first launch', () => {
    options.claudeInstalled = true;
    expect(registerClaudeDesktopMcp(options)).toBe('registered');
    expect(config().mcpServers.powermove.command).toBe(options.executablePath);
  });

  it('preserves other servers and app preferences, with a private recovery copy', () => {
    installed();
    const original = JSON.stringify({ preferences: { theme: 'dark' }, mcpServers: { other: { command: '/example/other', env: { EXAMPLE_SETTING: 'fixture' } } } });
    writeFileSync(file, original, { mode: 0o644 });
    expect(registerClaudeDesktopMcp(options)).toBe('registered');
    expect(config().preferences).toEqual({ theme: 'dark' });
    expect(config().mcpServers.other).toEqual(JSON.parse(original).mcpServers.other);
    expect(readFileSync(`${file}.before-powermove`, 'utf8')).toBe(original);
    expect(lstatSync(file).mode & 0o777).toBe(0o600);
    expect(lstatSync(`${file}.before-powermove`).mode & 0o777).toBe(0o600);
    const before = lstatSync(file).mtimeMs;
    expect(registerClaudeDesktopMcp(options)).toBe('ready');
    expect(lstatSync(file).mtimeMs).toBe(before);
  });

  it('updates its own executable after moving the app', () => {
    installed();
    registerClaudeDesktopMcp(options);
    options.executablePath = '/Users/example/Applications/Powermove.app/Contents/MacOS/Powermove';
    expect(registerClaudeDesktopMcp(options)).toBe('registered');
    expect(config().mcpServers.powermove.command).toBe(options.executablePath);
  });

  it('remembers a deliberate disconnect across launches and app moves', () => {
    installed();
    registerClaudeDesktopMcp(options);
    writeFileSync(file, '{"mcpServers":{}}');
    expect(registerClaudeDesktopMcp(options)).toBe('removed');
    options.executablePath = '/Users/example/Applications/Powermove.app/Contents/MacOS/Powermove';
    expect(registerClaudeDesktopMcp(options)).toBe('removed');
    expect(config().mcpServers).toEqual({});
    rmSync(file);
    expect(registerClaudeDesktopMcp(options)).toBe('removed');
    expect(existsSync(file)).toBe(false);
  });

  it.each([
    { command: '/custom/powermove', args: ['mcp'] },
    { command: '/Applications/Powermove.app/Contents/MacOS/Powermove', args: ['--powermove-mcp'], env: { EXAMPLE_SETTING: 'fixture' } },
    null
  ])('leaves a custom connection untouched: %j', server => {
    installed();
    const original = JSON.stringify({ mcpServers: { powermove: server } });
    writeFileSync(file, original);
    expect(registerClaudeDesktopMcp(options)).toBe('customized');
    expect(readFileSync(file, 'utf8')).toBe(original);
  });

  it.each(['{broken', '[]', '{"mcpServers":null}', '{"mcpServers":[]}'])('leaves invalid configuration untouched: %s', original => {
    installed();
    writeFileSync(file, original);
    expect(registerClaudeDesktopMcp(options)).toBe('failed');
    expect(readFileSync(file, 'utf8')).toBe(original);
  });

  it.each([true, false])('does not follow a configuration symlink (target exists: %s)', targetExists => {
    installed();
    const target = path.join(root, 'custom-config.json');
    if (targetExists) writeFileSync(target, '{}');
    symlinkSync(target, file);
    expect(registerClaudeDesktopMcp(options)).toBe('failed');
    expect(lstatSync(file).isSymbolicLink()).toBe(true);
    if (targetExists) expect(readFileSync(target, 'utf8')).toBe('{}');
    else expect(existsSync(target)).toBe(false);
  });

  it('does not rewrite a manually configured standard connection', () => {
    installed();
    const original = JSON.stringify({ mcpServers: { powermove: { command: options.executablePath, args: ['--powermove-mcp'] } } });
    writeFileSync(file, original);
    expect(registerClaudeDesktopMcp(options)).toBe('ready');
    expect(readFileSync(file, 'utf8')).toBe(original);
    expect(existsSync(`${file}.before-powermove`)).toBe(false);
  });
});
