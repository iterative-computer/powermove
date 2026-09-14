import { once } from 'node:events';
import { describe, expect, it } from 'vitest';
import { spawnClaudeProcess } from './process';

describe('Claude process launcher', () => {
  it('preserves arguments literally, exit status, and the PID through exec', async () => {
    const args = ['space in an argument', '$(echo must-not-run)', '`echo must-not-run`', 'a"b', "a'b"];
    const child = spawnClaudeProcess(process.execPath, ['-e', 'console.log(JSON.stringify({ pid: process.pid, args: process.argv.slice(1) })); process.exitCode = 7;', ...args], { stdio: ['ignore', 'pipe', 'pipe'] });
    let output = '';
    child.stdout!.on('data', chunk => { output += chunk; });
    const [code] = await once(child, 'close');
    expect(code).toBe(7);
    expect(JSON.parse(output)).toEqual({ pid: child.pid, args });
  });

  it('can cancel a pending executable without leaving a wrapper process', async () => {
    const child = spawnClaudeProcess(process.execPath, ['-e', 'console.log("ready"); setInterval(() => {}, 1000);'], { stdio: ['ignore', 'pipe', 'pipe'] });
    await once(child.stdout!, 'data');
    const closed = once(child, 'close');
    child.kill('SIGTERM');
    const [code, signal] = await closed;
    expect(code).toBeNull();
    expect(signal).toBe('SIGTERM');
  });
});
