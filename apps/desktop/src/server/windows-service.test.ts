import { describe, expect, it } from 'vitest';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { windowsSupervisor, windowsTask, windowsTaskStatus } from './windows-service';
import { install, uninstall, unitPath, unitText } from './install';
const spec = { entry: 'C:\\Users\\Test\\Powermove\\bin\\powermove.mjs', node: 'C:\\Program Files\\nodejs\\node.exe', userData: 'C:\\Users\\Test\\Powermove & Files', args: ['--port', '4747', '--exports', "C:\\Users\\Test\\It's here"], platform: 'win32' as const };
describe('Windows host login task', () => {
  it('runs with normal user permissions, preserves data paths, and has no execution time limit', () => {
    const task = windowsTask(spec);
    expect(task).toContain('<RunLevel>LeastPrivilege</RunLevel>');
    expect(task).toContain('<LogonType>InteractiveToken</LogonType>');
    expect(task).toContain('<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>');
    expect(task).toContain('Powermove &amp; Files');
    expect(task).not.toContain('ExecutionPolicy Bypass');
    expect(unitText(spec)).toBe(task);
    expect(unitPath(spec)).toContain('serve-task.xml');
  });
  it('passes host flags as data and restarts when the host updates itself', () => {
    const source = windowsSupervisor(spec);
    expect(source).toContain(JSON.stringify([spec.entry, 'serve', ...spec.args]));
    expect(source).toContain('setTimeout(start, 3000)');
    expect(source).toContain('windowsHide: true');
    expect(source).not.toContain('shell: true');
  });
});

describe.runIf(process.platform === 'win32')('native Windows login task', () => {
  it('starts the host at installation and stops it on uninstall while keeping the profile', async () => {
    // Never replace an existing installation when this test runs on a developer PC.
    expect(await windowsTaskStatus()).toBe('not installed');
    const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-task-'));
    const entry = path.join(root, "host & user's 雪.cjs");
    const marker = path.join(root, 'started.json');
    const profile = path.join(root, 'profile');
    const nativeSpec = { entry, node: process.execPath, userData: profile, args: ['--test', "spaces & user's 雪"], platform: 'win32' as const };
    await writeFile(entry, `require('node:fs').writeFileSync(${JSON.stringify(marker)}, JSON.stringify({pid:process.pid,args:process.argv.slice(2),profile:process.env.POWERMOVE_USER_DATA})); setInterval(()=>{},1000);`);
    let pid: number | undefined;
    try {
      await install(nativeSpec, () => {});
      await expect.poll(() => readFile(marker, 'utf8'), { timeout: 30_000 }).toBeTypeOf('string');
      const started = JSON.parse(await readFile(marker, 'utf8'));
      pid = started.pid;
      expect(started.args).toEqual(['serve', ...nativeSpec.args]);
      expect(started.profile).toBe(profile);
      expect(await windowsTaskStatus()).toBe('Running');
      await uninstall(nativeSpec, () => {});
      expect(await windowsTaskStatus()).toBe('not installed');
      await expect.poll(() => { try { process.kill(pid!, 0); return true; } catch { return false; } }, { timeout: 15_000 }).toBe(false);
      expect(await readFile(path.join(profile, 'serve.log'), 'utf8')).toBeTypeOf('string');
    } finally {
      await uninstall(nativeSpec, () => {});
      if (pid) { try { process.kill(pid); } catch { /* already stopped */ } }
      await rm(root, { recursive: true, force: true });
    }
  }, 60_000);
});
