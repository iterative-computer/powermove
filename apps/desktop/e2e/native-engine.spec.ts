import { expect, test } from '@playwright/test';

import { launchApp } from './helpers/app';
import { decode, encode } from '../src/main/engine/msgpack';
import { hostHello } from '../src/main/engine/protocol';

const engineProcesses = (app: import('playwright').ElectronApplication): Promise<string[]> =>
  app.evaluate(({ app }) => app.getAppMetrics()
    .filter((m) => m.type === 'Utility')
    .map((m) => [m.serviceName, m.name].filter(Boolean).join(' '))
    .filter((name) => / Engine/.test(name))
    .sort());

type Host = { engine: { status(): Record<string, { state: string; generation: number }>; reconnectClient(id: number): void } | null };

test('Use New Engine off: no engine process and no ports', async () => {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
  try {
    const host = await session.app.evaluate(() => {
      const installed = (globalThis as { __powermoveNativeEngine?: Host }).__powermoveNativeEngine;
      return { installed: Boolean(installed), engine: Boolean(installed?.engine) };
    });
    expect(host).toEqual({ installed: true, engine: false });
    expect(await engineProcesses(session.app)).toEqual([]);
  } finally {
    await session.close();
  }
});

test('Use New Engine on: the editor window gets a live port to each engine', async () => {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await expect.poll(() => session.app.evaluate(() => {
      const status = (globalThis as { __powermoveNativeEngine?: Host }).__powermoveNativeEngine?.engine?.status();
      return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
    }), { timeout: 15_000 }).toBe('ready/ready');

    // Collect the ports the preload hands to the page, then ask main for fresh ones.
    await session.page.evaluate(() => {
      const w = window as unknown as { __enginePorts: Array<{ process: string; generation: number; port: MessagePort }> };
      w.__enginePorts = [];
      window.addEventListener('message', (event) => {
        if (event.source !== window || event.data?.type !== 'powermove:engine-port' || !event.ports[0]) return;
        w.__enginePorts.push({ process: event.data.process, generation: event.data.generation, port: event.ports[0] });
      });
    });
    await session.app.evaluate(({ BrowserWindow }) => {
      const host = (globalThis as { __powermoveNativeEngine?: Host }).__powermoveNativeEngine!;
      for (const window of BrowserWindow.getAllWindows()) host.engine!.reconnectClient(window.webContents.id);
    });
    await session.page.waitForFunction(() => (window as unknown as { __enginePorts: unknown[] }).__enginePorts.length >= 2);

    // Negotiate over each port from the page as a renderer.
    const hello = Array.from(encode({ wire: 1, session: null, revision: null, message: { kind: 'hello', ...hostHello(), role: 'renderer' } }));
    const replies = await session.page.evaluate(async (bytes) => {
      const w = window as unknown as { __enginePorts: Array<{ process: string; generation: number; port: MessagePort }> };
      return Promise.all(w.__enginePorts.slice(-2).map(({ process, generation, port }) => new Promise<{ process: string; generation: number; reply: number[] }>((resolve) => {
        port.onmessage = (event) => resolve({ process, generation, reply: Array.from(new Uint8Array(event.data as Uint8Array)) });
        port.postMessage(new Uint8Array(bytes));
      })));
    }, hello);
    expect(replies.map((r) => r.process).sort()).toEqual(['interactive', 'job']);
    for (const { process, generation, reply } of replies) {
      const message = (decode(new Uint8Array(reply)) as { message: { kind: string; role: string } }).message;
      expect(message.kind).toBe('welcome');
      expect(message.role).toBe(process === 'interactive' ? 'interactiveEngine' : 'jobEngine');
      expect(generation).toBe(1);
    }
    const engines = await engineProcesses(session.app);
    expect(engines, JSON.stringify(engines)).toHaveLength(2);
    expect(engines.join()).toMatch(/Interactive Engine.*Job Engine/);
  } finally {
    await session.close();
  }
});
