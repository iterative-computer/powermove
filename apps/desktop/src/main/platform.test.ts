import { describe, expect, it } from 'vitest';
import path from 'node:path';
import { agentPlatform, encoderBinary, executableName, windowChrome } from './platform';
import { absolutePathEntries } from './login-shell-path';
import { windowsSandboxArgs } from './windows-sandbox';

describe('Windows executable and window layout', () => {
  it('uses the native x64 agents and encoder', () => {
    expect(agentPlatform('win32', 'x64')).toEqual({ suffix: 'win32-x64', triple: 'x86_64-pc-windows-msvc', codex: 'codex.exe', claude: 'claude.exe' });
    expect(executableName('codex-code-mode-host', 'win32')).toBe('codex-code-mode-host.exe');
    expect(encoderBinary('/app', '/resources', 'win32')).toBe(path.join('/resources', 'encoder', 'ffmpeg.exe'));
    expect(encoderBinary('/app', undefined, 'win32')).toBe(path.join('/app', 'node_modules', 'ffmpeg-static', 'ffmpeg.exe'));
    expect(() => agentPlatform('win32', 'ia32')).toThrow('unavailable');
  });
  it('provides visible native Windows controls and preserves Mac traffic lights', () => {
    expect(windowChrome('win32')).toMatchObject({ titleBarStyle: 'hidden', titleBarOverlay: { height: 44 } });
    expect(windowChrome('win32')).not.toHaveProperty('vibrancy');
    expect(windowChrome('darwin')).toMatchObject({ titleBarStyle: 'hiddenInset', trafficLightPosition: { x: 14, y: 15 } });
  });
  it('keeps drive letters, UNC paths, spaces and quoted Windows PATH entries intact', () => {
    expect(absolutePathEntries('C:\\Tools;"C:\\Program Files\\nodejs";c:\\tools;relative;\\\\server\\tools', 'win32'))
      .toEqual(['C:\\Tools', 'C:\\Program Files\\nodejs', '\\\\server\\tools']);
  });
  it('encloses Windows Project commands in a sandbox with literal arguments', () => {
    const command = ['C:\\Program Files\\Claude\\claude.exe', 'a"b', '& echo dangerous'];
    const args = windowsSandboxArgs('C:\\My Project', command, ['C:\\Private Runtime']);
    expect(args.slice(args.indexOf('--') + 1)).toEqual(command);
    expect(args).toContain('powermove-project');
    expect(args.find(value => value.startsWith('permissions.powermove-project='))).toContain('"C:\\\\Private Runtime" = "write"');
    expect(args.join(' ')).not.toContain('dangerously');
  });
});
