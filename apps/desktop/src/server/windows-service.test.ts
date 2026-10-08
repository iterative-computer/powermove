import { describe, expect, it } from 'vitest';
import { windowsSupervisor, windowsTask } from './windows-service';
import { unitPath, unitText } from './install';
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
