import { execFile, spawn } from 'node:child_process';
import { once } from 'node:events';
import { copyFile, lstat, mkdir, mkdtemp, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({ app: { getAppPath: () => process.cwd() } }));
import { bundledCodexCandidates, discoverCodexBinary } from './codex/env';
import { bundledClaudeCandidates, discoverClaudeBinary } from './claude/env';
import { spawnClaudeProcess } from './claude/process';
import { windowsCloudFileState, windowsFontFamilies } from './windows-system';
import { materializeFile } from './cloud-media';
import { MediaProxyService, stillImageConverter, imageSequenceConverter } from './media-proxy';
import { encoderBinary } from './platform';
import { fileVersion, publishFile, syncDirectory } from './durable-file';
import { killProcessFamily } from './process-family';
import { CompatibleWorkspace, runWorkspaceCommand } from './compatible-workspace';
import { prepareUserResources } from './agent-tools/user-resources';

const run = promisify(execFile);
const roots: string[] = [];
afterEach(async () => { vi.unstubAllEnvs(); await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });
async function folder() { const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-windows-')); roots.push(root); return root; }

describe('portable HEIC import', () => {
  it('decodes an Apple HEIC fixture in a worker without macOS image tools', async () => {
    const root = await folder();
    const output = path.join(root, 'converted.png');
    await stillImageConverter(encoderBinary(process.cwd()), 'win32')(path.resolve('e2e/fixtures/still-red.heic'), 'heic', output);
    expect((await readFile(output)).subarray(0, 8)).toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
  }, 30_000);
});

describe.runIf(process.platform === 'win32')('native Windows x64 integration', () => {
  it('shares skills through a stable junction and refreshes copied instructions', async () => {
    const source = await folder(); const runtime = await folder();
    await mkdir(path.join(source, 'skills'));
    await writeFile(path.join(source, 'AGENTS.md'), 'first');
    await prepareUserResources(runtime, source, 'chatgpt');
    expect(await readFile(path.join(runtime, 'config.toml'), 'utf8')).toContain('sandbox = "unelevated"');
    expect((await lstat(path.join(runtime, 'skills'))).isSymbolicLink()).toBe(true);
    await prepareUserResources(runtime, source, 'chatgpt');
    await expect(stat(path.join(runtime, '.powermove-resource-backups'))).rejects.toMatchObject({ code: 'ENOENT' });
    await writeFile(path.join(source, 'AGENTS.md'), 'second');
    await prepareUserResources(runtime, source, 'chatgpt');
    expect(await readFile(path.join(runtime, 'AGENTS.md'), 'utf8')).toBe('second');
    await rm(path.join(source, 'skills'), { recursive: true });
    await prepareUserResources(runtime, source, 'chatgpt');
    await expect(lstat(path.join(runtime, 'skills'))).rejects.toMatchObject({ code: 'ENOENT' });
  });
  it('finds and launches both bundled agents and the video encoder', async () => {
    const codex = await discoverCodexBinary(null, { bundledCandidates: bundledCodexCandidates(process.cwd(), undefined) });
    const claude = await discoverClaudeBinary(null, { bundledCandidates: bundledClaudeCandidates(process.cwd(), undefined) });
    for (const binary of [codex, claude, encoderBinary(process.cwd())]) {
      expect(binary).toMatch(/\.exe$/i);
      const { stdout } = await run(binary, [binary.endsWith('ffmpeg.exe') ? '-version' : '--version'], { windowsHide: true, timeout: 15_000 });
      expect(stdout.trim().length).toBeGreaterThan(0);
    }
    expect((await stat(path.join(path.dirname(codex), 'codex-code-mode-host.exe'))).size).toBeGreaterThan(0);
  }, 60_000);
  it('keeps shell metacharacters literal when launching a process', async () => {
    const args = ['spaces and 雪', '&echo NO', '$(echo NO)', 'a"b', "a'b"];
    const child = spawnClaudeProcess(process.execPath, ['-e', 'console.log(JSON.stringify(process.argv.slice(1)))', ...args], { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = ''; child.stdout!.on('data', data => { stdout += data; });
    const [code] = await once(child, 'close');
    expect(code).toBe(0); expect(JSON.parse(stdout)).toEqual(args);
  });
  it('lists fonts and reads cloud files without interpreting their names', async () => {
    expect((await windowsFontFamilies()).length).toBeGreaterThan(0);
    const root = await folder(); const source = path.join(root, "A & B's 雪.mov");
    await writeFile(source, 'media');
    expect(await windowsCloudFileState(source)).toBe('local');
    await materializeFile(source);
    expect(await windowsCloudFileState(path.join(root, 'missing.mov'))).toBe('missing');
  }, 45_000);
  it('saves and replaces files without trying to open a Windows directory handle', async () => {
    const root = await folder(); const destination = path.join(root, 'project.pmv');
    const first = path.join(root, 'first.tmp'); await writeFile(first, 'first');
    await publishFile(first, destination, undefined, 'unused');
    const previous = await fileVersion(destination);
    const next = path.join(root, 'next.tmp'); await writeFile(next, 'next');
    await publishFile(next, destination, previous, 'unused');
    await syncDirectory(root);
    expect(await readFile(destination, 'utf8')).toBe('next');
  });
  it('imports an image sequence without requiring symbolic-link privileges', async () => {
    const root = await folder(); const files = [path.join(root, 'shot 01.png'), path.join(root, 'shot 02.png')];
    for (const file of files) await copyFile(path.resolve('e2e/fixtures/still-red.png'), file);
    const service = new MediaProxyService(root, async () => {}, imageSequenceConverter(encoderBinary(process.cwd())));
    try { expect((await service.createSequence(files, 24)).size).toBeGreaterThan(100); }
    finally { await service.dispose(); }
  }, 30_000);
  it('stops a running process and the child it started', async () => {
    const root = await folder();
    const child = spawn(process.execPath, ['-e', `const {spawn}=require('node:child_process'); const child=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'}); console.log(child.pid); setInterval(()=>{},1000);`], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'], cwd: root });
    const [data] = await once(child.stdout!, 'data'); const descendant = Number(String(data).trim());
    const exited = once(child, 'close');
    try {
      await killProcessFamily(child.pid!, { cwd: root, since: Date.now() - 5_000 });
      await exited;
      expect(() => process.kill(descendant, 0)).toThrow();
    } finally { child.kill(); try { process.kill(descendant); } catch { /* gone */ } }
  }, 45_000);
  it('allows Project commands to write inside the workspace and denies writes outside it', async () => {
    const root = await folder(); const runtime = await folder(); const outside = await folder();
    vi.stubEnv('CODEX_HOME', runtime);
    const quote = (value: string) => `'${value.replaceAll("'", "''")}'`;
    const insideFile = path.join(root, 'allowed.txt'); const outsideFile = path.join(outside, 'denied.txt');
    const result = await runWorkspaceCommand(root, 'project', `$ErrorActionPreference='Stop'; Set-Content -LiteralPath ${quote(insideFile)} -Value 'allowed'; Set-Content -LiteralPath ${quote(outsideFile)} -Value 'denied'`, 60_000, new AbortController().signal);
    await expect(readFile(insideFile, 'utf8'), result.output).resolves.toContain('allowed');
    expect(result.exitCode).not.toBe(0);
    await expect(stat(outsideFile)).rejects.toMatchObject({ code: 'ENOENT' });
  }, 90_000);
  it('writes custom-provider files through the sandbox with literal UTF-8 names and contents', async () => {
    const root = await folder(); const runtime = await folder(); const outside = await folder();
    vi.stubEnv('CODEX_HOME', runtime);
    const workspace = new CompatibleWorkspace({ root } as any, 'project');
    const file = "nested/A & B's 雪.txt";
    const text = 'héllo 雪 $(exit 1)\n' + 'x'.repeat(100_000);
    const signal = new AbortController().signal;
    try {
      await workspace.call('write_file', { path: file, text }, signal);
      expect(await readFile(path.join(root, file), 'utf8')).toBe(text);
      await symlink(outside, path.join(root, 'escape'), 'junction');
      await expect(workspace.call('write_file', { path: 'escape/denied.txt', text: 'no' }, signal)).rejects.toThrow('symbolic link');
      await expect(stat(path.join(outside, 'denied.txt'))).rejects.toMatchObject({ code: 'ENOENT' });
    } finally { await workspace.stopCommands(); }
  }, 90_000);
});
