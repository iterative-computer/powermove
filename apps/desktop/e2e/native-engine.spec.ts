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

type Host = { engine: { ensureReady(process: 'job'): Promise<void>; status(): Record<string, { state: string; generation: number }>; reconnectClient(id: number): void } | null };

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

test('Use New Engine on: idle jobs stay absent and their first request gets live ports', async () => {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await expect.poll(() => session.app.evaluate(() => {
      const status = (globalThis as { __powermoveNativeEngine?: Host }).__powermoveNativeEngine?.engine?.status();
      return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
    }), { timeout: 15_000 }).toBe('ready/stopped');
    expect(await engineProcesses(session.app)).toHaveLength(1);

    // Collect the ports the preload hands to the page, then ask main for fresh ones.
    await session.page.evaluate(() => {
      const w = window as unknown as { __enginePorts: Array<{ process: string; generation: number; port: MessagePort }> };
      w.__enginePorts = [];
      window.addEventListener('message', (event) => {
        if (event.source !== window || event.data?.type !== 'powermove:engine-port' || !event.ports[0]) return;
        w.__enginePorts.push({ process: event.data.process, generation: event.data.generation, port: event.ports[0] });
      });
    });
    await session.app.evaluate(async ({ BrowserWindow }) => {
      const host = (globalThis as { __powermoveNativeEngine?: Host }).__powermoveNativeEngine!;
      await host.engine!.ensureReady('job');
      for (const window of BrowserWindow.getAllWindows()) host.engine!.reconnectClient(window.webContents.id);
    });
    await session.page.waitForFunction(() => (window as unknown as { __enginePorts: unknown[] }).__enginePorts.length >= 2);

    // The kernel's engine connection (Use New Engine) negotiates the
    // interactive engine's port itself (a second hello on it is refused):
    // it is connected, on the first launch.
    await expect.poll(() => session.page.evaluate(() => {
      const status = (window as any).PM.EngineConnection?.status();
      return status ? `${status.connected}/${status.generation}` : 'missing';
    }), { timeout: 15_000 }).toBe('true/1');
    // The job engine's port is the page's to negotiate as a renderer.
    const hello = Array.from(encode({ wire: 1, session: null, revision: null, message: { kind: 'hello', ...hostHello(), role: 'renderer' } }));
    const reply = await session.page.evaluate(async (bytes) => {
      const w = window as unknown as { __enginePorts: Array<{ process: string; generation: number; port: MessagePort }> };
      const job = w.__enginePorts.filter((p) => p.process === 'job').at(-1)!;
      return new Promise<{ generation: number; reply: number[] }>((resolve) => {
        job.port.onmessage = (event) => resolve({ generation: job.generation, reply: Array.from(new Uint8Array(event.data as Uint8Array)) });
        job.port.postMessage(new Uint8Array(bytes));
      });
    }, hello);
    const message = (decode(new Uint8Array(reply.reply)) as { message: { kind: string; role: string } }).message;
    expect(message.kind).toBe('welcome');
    expect(message.role).toBe('jobEngine');
    expect(reply.generation).toBe(1);
    expect(await session.page.evaluate(() => (window as unknown as { __enginePorts: Array<{ process: string }> }).__enginePorts.map((p) => p.process))).toEqual(expect.arrayContaining(['interactive', 'job']));
    const engines = await engineProcesses(session.app);
    expect(engines, JSON.stringify(engines)).toHaveLength(2);
    expect(engines.join()).toMatch(/Interactive Engine.*Job Engine/);
  } finally {
    await session.close();
  }
});

test('Use New Engine on: an unused job engine exits and the next request starts it again', async () => {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1', POWERMOVE_ENGINE_JOB_IDLE_MS: '1500' } });
  const state = (): Promise<string> => session.app.evaluate(() => {
    const status = (globalThis as { __powermoveNativeEngine?: Host }).__powermoveNativeEngine?.engine?.status();
    return status ? `${status['interactive']!.state}/${status['job']!.state}/${status['job']!.generation}` : 'missing';
  });
  try {
    await expect.poll(state, { timeout: 15_000 }).toBe('ready/stopped/0');
    const firstMs = await session.app.evaluate(async () => {
      const start = performance.now();
      await (globalThis as { __powermoveNativeEngine?: Host }).__powermoveNativeEngine!.engine!.ensureReady('job');
      return performance.now() - start;
    });
    expect(await state()).toBe('ready/ready/1');
    expect(await engineProcesses(session.app)).toHaveLength(2);
    // Nothing uses it: it exits on its own, and so does its process.
    await expect.poll(state, { timeout: 15_000 }).toBe('ready/stopped/0');
    await expect.poll(() => engineProcesses(session.app), { timeout: 10_000 }).toHaveLength(1);
    // The interactive engine and the viewer carry on.
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection?.status().connected), { timeout: 10_000 }).toBe(true);
    // The next request starts a new one.
    await session.app.evaluate(async () => {
      await (globalThis as { __powermoveNativeEngine?: Host }).__powermoveNativeEngine!.engine!.ensureReady('job');
    });
    expect(await state()).toMatch(/^ready\/ready\/\d+$/);
    expect(await engineProcesses(session.app)).toHaveLength(2);
    test.info().annotations.push({ type: 'cold job readiness ms', description: firstMs.toFixed(1) });
  } finally {
    await session.close();
  }
});
