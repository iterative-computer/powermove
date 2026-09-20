import { describe, expect, it, vi } from 'vitest';

import { UpdateChecker, detectInstallKind, isNewer, updateCommand } from './updates';

describe('updates', () => {
  it('compares versions the way npm publishes them', () => {
    expect(isNewer('1.0.2', '1.0.1')).toBe(true);
    expect(isNewer('1.0.10', '1.0.9')).toBe(true);
    expect(isNewer('1.0.1', '1.0.1')).toBe(false);
    expect(isNewer('1.0.1', '1.0.2')).toBe(false);
    expect(isNewer('1.1.0-beta.1', '1.1.0')).toBe(false);
    expect(isNewer('1.1.0', '1.1.0-beta.1')).toBe(true);
    expect(isNewer('1.1.0-beta.2', '1.1.0-beta.1')).toBe(true);
  });

  it('tells a global install from npx and a source checkout', () => {
    expect(detectInstallKind('/usr/local/lib/node_modules/powermove-cli/bin/powermove.mjs')).toBe('global');
    expect(detectInstallKind('/home/me/.npm/_npx/abc123/node_modules/powermove-cli/bin/powermove.mjs')).toBe('npx');
    expect(detectInstallKind('/home/me/Developer/powermove/packages/cli/bin/powermove.mjs')).toBe('source');
    expect(updateCommand('npx')).toBe('npx powermove-cli@latest serve');
    expect(updateCommand('global')).toBe('npm i -g powermove-cli@latest');
  });

  it('reports ready when the registry is ahead, idle otherwise', async () => {
    const states: string[] = [];
    const checker = new UpdateChecker({ current: '1.0.1', installKind: 'global', fetchLatest: async () => '1.0.2' });
    checker.onChanged((state) => states.push(`${state.status}:${state.version}`));
    await checker.check();
    expect(states).toEqual(['checking:null', 'ready:1.0.2']);
    const idle = new UpdateChecker({ current: '1.0.2', installKind: 'global', fetchLatest: async () => '1.0.2' });
    await idle.check();
    expect(idle.status()).toEqual({ status: 'idle', current: '1.0.2', version: null });
  });

  it('self-updates a global install and exits; hands npx users the command', async () => {
    const selfUpdate = vi.fn(async () => {});
    const exit = vi.fn();
    const checker = new UpdateChecker({ current: '1.0.1', installKind: 'global', fetchLatest: async () => '1.0.2', selfUpdate, exit });
    await checker.check();
    expect(await checker.install()).toEqual({ restarting: true });
    expect(selfUpdate).toHaveBeenCalledOnce();
    await new Promise((resolve) => setTimeout(resolve, 350));
    expect(exit).toHaveBeenCalledOnce();
    const npx = new UpdateChecker({ current: '1.0.1', installKind: 'npx', fetchLatest: async () => '1.0.2' });
    expect(await npx.install()).toEqual({ restarting: false, command: 'npx powermove-cli@latest serve' });
  });

  it('falls back to the command when the self-update fails', async () => {
    const checker = new UpdateChecker({ current: '1.0.1', installKind: 'global', fetchLatest: async () => '1.0.2', selfUpdate: async () => { throw new Error('EACCES'); }, exit: vi.fn() });
    await checker.check();
    expect(await checker.install()).toEqual({ restarting: false, command: 'npm i -g powermove-cli@latest' });
    expect(checker.status().status).toBe('ready');
  });
});
