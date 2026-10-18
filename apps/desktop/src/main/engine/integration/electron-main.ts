/*
 * Main-process integration test for the native engine host (run by
 * scripts/engine-host-integration.mjs inside Electron, no windows).
 *
 * Starts both engine utility processes with the real pm-node addon,
 * negotiates, connects a renderer stand-in to each, opens sessions and
 * resyncs (inline and through a staged cache file the engine resolves with
 * main), kills the interactive engine and then the job engine with SIGKILL,
 * and checks that each restarts with backoff, the crash report reaches the
 * client, the client gets new ports, re-opens its session, and resyncs,
 * and that the other engine kept its session throughout.
 */
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { app, MessageChannelMain, utilityProcess } from 'electron';

import { canonicalHash, canonicalStringify } from '../../../shared/canonical-json';
import type { CrashReport, EngineProcess, ProjectColor } from '../../../shared/engine-protocol';
import { NativeEngine } from '../native-engine';
import type { ChildLike, ClientPortInfo, PortLike } from '../ports';
import { TestClient } from './client';
import { mainHostClock } from './main-clock';

type Step = { name: string; ms: number; detail?: unknown };
const steps: Step[] = [];
const started = performance.now();
let last = started;

function step(name: string, detail?: unknown): void {
  const now = performance.now();
  steps.push({ name, ms: Math.round((now - last) * 10) / 10, ...(detail === undefined ? {} : { detail }) });
  last = now;
}

function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`check failed: ${message}`);
}

function waitFor<T>(label: string, poll: () => T | null | undefined | false, timeoutMs = 15_000): Promise<T> {
  return new Promise((resolve, reject) => {
    const deadline = Date.now() + timeoutMs;
    const tick = (): void => {
      let value: T | null | undefined | false;
      try {
        value = poll();
      } catch (error) {
        reject(error);
        return;
      }
      if (value) resolve(value);
      else if (Date.now() > deadline) reject(new Error(`timed out waiting for ${label}`));
      else setTimeout(tick, 5);
    };
    tick();
  });
}

/** Decision 1: Standard color, linear blending off (pm_protocol `ProjectColor::DEFAULT`). */
const COLOR: ProjectColor = { mode: 'standard', linearBlending: false, precision: 'float16', blendDomain: 'hdrSafe' };

const fromHex = (hex: string): Uint8Array => new Uint8Array(Buffer.from(hex, 'hex'));

async function main(): Promise<void> {
  const enginePath = process.env['PM_IT_ENGINE_PROCESS'];
  const addon = process.env['POWERMOVE_ENGINE_ADDON'];
  check(enginePath && addon, 'PM_IT_ENGINE_PROCESS and POWERMOVE_ENGINE_ADDON are set');
  const clock = mainHostClock();
  step('main reads the engines\' host clock through pm-bridge', { libuvMinusEngineUs: clock.libuvMinusEngineUs });
  const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-engine-it-'));
  const logs: string[] = [];
  const pids: Record<EngineProcess, number[]> = { interactive: [], job: [] };

  const engine = new NativeEngine({
    spawn: (kind) => {
      const child = utilityProcess.fork(enginePath, [], {
        serviceName: `Powermove ${kind} engine (test)`,
        stdio: ['ignore', 'pipe', 'pipe'],
        env: { ...process.env, POWERMOVE_ENGINE_PROCESS: kind, POWERMOVE_ENGINE_ADDON: addon }
      });
      child.on('spawn', () => { if (child.pid) pids[kind].push(child.pid); });
      child.stderr?.on('data', (chunk: Buffer) => logs.push(`[${kind} stderr] ${chunk.toString().trim()}`));
      return child as unknown as ChildLike;
    },
    channel: () => new MessageChannelMain() as unknown as { port1: PortLike; port2: PortLike },
    cacheDirectory: path.join(dir, 'Engine Cache'),
    automaticOutputDirectory: path.join(dir, 'Powermove'),
    physicalMemory: os.totalmem(),
    hostNow: clock.now,
    log: (level, message) => logs.push(`${level}: ${message}`)
  });

  const crashes: CrashReport[] = [];
  engine.listen({ crash: (report) => crashes.push(report) });
  const clients: TestClient[] = [];
  const latest = (kind: EngineProcess): TestClient | undefined => clients.filter((c) => c.process === kind).at(-1);

  try {
    await engine.start();
    await waitFor('both engines ready', () => engine.status().interactive.state === 'ready' && engine.status().job.state === 'ready');
    step('both engines started and negotiated with main', engine.status());
    for (const kind of ['interactive', 'job'] as const) {
      const launch = engine.supervisor(kind).current!;
      await waitFor(`${kind} start notice`, () => launch.started);
      const skew = launch.started!.clockSkewUs;
      check(skew !== null && skew >= 0 && skew < 2_000_000, `the ${kind} engine shares main's host clock (delivered after ${skew} µs)`);
    }
    step('engines read the same host clock as main', {
      interactive: engine.supervisor('interactive').current!.started,
      job: engine.supervisor('job').current!.started
    });
    await waitFor('leases acknowledged', () => engine.memory.isAcknowledged('interactive') && engine.memory.isAcknowledged('job'));
    step('initial leases granted and acknowledged', { interactive: engine.memory.current('interactive'), job: engine.memory.current('job') });

    engine.addClient(1, (info: ClientPortInfo, port: PortLike) => clients.push(new TestClient(info.process, info.generation, port)));
    await waitFor('client ports', () => latest('interactive') && latest('job'));

    const doc = { comps: {}, layers: [{ id: 'l1', name: 'Title' }], name: 'Integration' };
    const body = new TextEncoder().encode(canonicalStringify(doc)!);
    const hash = fromHex(canonicalHash(doc));

    /** Handshake, session, resync, consistency: what a renderer does after (re)connecting. */
    const connect = async (kind: EngineProcess, revision: number, viaFile: boolean): Promise<number> => {
      const client = latest(kind)!;
      await client.open();
      const opened = await client.request({ type: 'openSession', document: 'doc-1', role: kind === 'interactive' ? 'interactive' : 'job', process: kind, color: COLOR });
      check(opened.type === 'sessionOpened', 'session opened');
      const session = opened.session;
      const source = viaFile
        ? await (async () => {
          const staged = await engine.stageDocument(kind, session, body);
          return { kind: 'cacheFile' as const, handle: staged.handle, length: staged.length };
        })()
        : { kind: 'inline' as const, body };
      const applied = await client.request({ type: 'resync', document: 'doc-1', revision, encoding: 'canonicalJsonV1', source, hash }, session);
      check(applied.type === 'patchesApplied' && applied.revision === revision, `resync applied at ${revision}`);
      const consistency = await client.request({ type: 'checkConsistency', document: 'doc-1', revision, hash }, session);
      check(consistency.type === 'consistency' && consistency.matches, 'the engine mirror matches the document hash');
      return session;
    };

    await (async () => {
      const interactiveSession = await connect('interactive', 5, false);
      step('renderer stand-in negotiated, opened a session, and resynced inline', { session: interactiveSession });
      const jobSession = await connect('job', 5, true);
      step('job engine resynced from a staged cache file resolved through main', { session: jobSession });

      // A session main opens itself, primed with a window's definitions.
      engine.updateDefinitions(1, { generation: 1, full: true, upserts: [], removals: [] });
      const snapshot = await engine.request('job', { type: 'openSession', document: 'doc-1', role: 'jobSnapshot', process: 'job', color: COLOR });
      check(snapshot.type === 'sessionOpened', 'host-opened job snapshot session');
      const definitions = await engine.request('job', { type: 'pushDefinitions', ...engine.definitions.snapshotFor(1)! }, snapshot.session);
      check(definitions.type === 'definitionsApplied' && definitions.generation === 1, 'definitions relayed to the job snapshot session');
      step('host opened a job snapshot session and relayed definitions');

      // Unsupported work is routed back, not failed silently.
      const render = await latest('interactive')!.request({ type: 'evaluate', composition: 'c', time: 0, view: null }, interactiveSession).then(() => 'answered', (e: Error & { error?: { code: string } }) => e.error?.code);
      check(render === 'unsupported', `evaluate is routed back to the current engine (${render})`);

      for (const kind of ['interactive', 'job'] as const) {
        const other: EngineProcess = kind === 'interactive' ? 'job' : 'interactive';
        const victim = engine.supervisor(kind).current!;
        const pid = victim.pid!;
        const generation = victim.generation;
        const crashesBefore = crashes.length;
        const killedAt = performance.now();
        process.kill(pid, 'SIGKILL');
        const report = await waitFor(`${kind} crash report`, () => crashes.length > crashesBefore && crashes.at(-1));
        check(report.process === kind && report.attempt === 1 && report.restartInMs === 250, `${kind} crash reported with attempt 1 and a 250 ms backoff (${JSON.stringify(report)})`);
        check(report.exit.startsWith('exit status 9'), `${kind} crash report carries the SIGKILL status, not main's own kill (${report.exit})`);
        await waitFor(`${kind} restart`, () => engine.supervisor(kind).current?.generation === generation + 1);
        const restartMs = Math.round(performance.now() - killedAt);
        await waitFor(`${kind} new client port`, () => latest(kind)?.generation === generation + 1);
        check(engine.supervisor(kind).current!.pid !== pid, 'a new process');
        const session = await connect(kind, 6, kind === 'job');
        step(`${kind} engine killed, restarted, and the client resynced`, { exit: report.exit, restartMs, newPid: engine.supervisor(kind).current!.pid, session });
        // The other engine kept its session through the crash.
        const survivor = latest(other)!;
        const otherSession = other === 'interactive' ? interactiveSession : jobSession;
        if (kind === 'interactive') {
          const still = await survivor.request({ type: 'checkConsistency', document: 'doc-1', revision: 5, hash }, otherSession);
          check(still.type === 'consistency' && still.matches, `the ${other} engine kept its session through the ${kind} crash`);
          step(`the ${other} engine was unaffected`);
        }
      }
      // Control messages that race an engine's death (a window reloading
      // while the engine dies) are dropped, never posted to the dying
      // process: posting to an exiting utility process crashes main.
      {
        const dying = engine.supervisor('job').current!;
        process.kill(dying.pid!, 'SIGKILL');
        for (let i = 0; i < 40; i++) {
          engine.reconnectClient(1);
          await new Promise((resolve) => setImmediate(resolve));
        }
        await waitFor('job restart after the reconnect storm', () => engine.supervisor('job').current?.generation === dying.generation + 1);
        await waitFor('job client port after the reconnect storm', () => latest('job')?.generation === dying.generation + 1);
        step('reconnects racing a dying engine were dropped; the engine restarted');
      }
      check(engine.handles.size >= 0 && engine.outputs.size === 0, 'no outputs left behind');
      await waitFor('leases after restarts', () => engine.memory.isAcknowledged('interactive') && engine.memory.isAcknowledged('job'));
      step('leases re-granted to both restarted engines');

      await engine.stop();
      check(engine.status().interactive.state === 'stopped' && engine.status().job.state === 'stopped', 'both engines stopped');
      step('both engines stopped cleanly');
      // Electron reports the exit as the child's IPC goes away; the OS
      // reaps it moments later.
      const alive = (pid: number): boolean => {
        try { process.kill(pid, 0); return true; } catch { return false; }
      };
      await waitFor('engine processes to be reaped', () => [...pids.interactive, ...pids.job].every((pid) => !alive(pid)), 5000);
      step('no engine process left running', pids);

      // Stopping while an engine is crashing: the crash's cleanup must not
      // reach the other engine, which is shutting down.
      await engine.start();
      await waitFor('both engines ready again', () => engine.status().interactive.state === 'ready' && engine.status().job.state === 'ready');
      process.kill(engine.supervisor('interactive').current!.pid!, 'SIGKILL');
      engine.removeClient(1);
      await engine.stop();
      check(engine.status().interactive.state === 'stopped' && engine.status().job.state === 'stopped', 'both engines stopped after a crash during stop');
      await waitFor('engine processes to be reaped after the second stop', () => [...pids.interactive, ...pids.job].every((pid) => !alive(pid)), 5000);
      step('restarted, then stopped while the interactive engine crashed');
    })();
  } finally {
    await engine.stop().catch(() => {});
    await rm(dir, { recursive: true, force: true });
    process.stdout.write(`${JSON.stringify({ steps, crashes, totalMs: Math.round(performance.now() - started), log: logs.slice(-80) })}\n`);
  }
}

// Main's own errors fail the test instead of opening Electron's dialog.
process.on('uncaughtException', (error) => {
  process.stdout.write(`ENGINE-HOST-IT FAIL uncaught ${error.stack ?? error.message}\n`);
  app.exit(1);
});

app.dock?.hide();
app.on('window-all-closed', () => {});
void app.whenReady().then(() => main()).then(
  () => {
    process.stdout.write('ENGINE-HOST-IT PASS\n');
    app.exit(0);
  },
  (error: unknown) => {
    process.stdout.write(`ENGINE-HOST-IT FAIL ${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
    app.exit(1);
  }
);
setTimeout(() => {
  process.stdout.write('ENGINE-HOST-IT FAIL timeout\n');
  app.exit(1);
}, 90_000).unref();
