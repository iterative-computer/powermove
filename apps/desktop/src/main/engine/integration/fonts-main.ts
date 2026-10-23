/*
 * Integration test for the interactive engine's font service (plan section
 * 12, Phase 2 "Font registry and subsetting move into the engine"). Run by
 * scripts/engine-fonts-integration.mjs inside Electron, no windows.
 *
 * Everything is the production path: the interactive engine in a utility
 * process with the real pm-node addon, main's `EngineFontClient` on its own
 * font port (`NativeEngine.addClient` with the `fonts` peer), and the
 * `engine-fonts:*` channels a renderer invokes, registered by
 * `installEngineFontService`. It checks that the CoreText registry starts on
 * the utility process's JavaScript thread (AppKit needs the process's main
 * thread), answers and subsets reach the renderer's channels, project fonts
 * change the generation, the job engine gets its own font port with the
 * project fonts and seeds mirrored (its exports and agent frames lay text
 * out with them), and that after an engine is killed the restarted engine
 * gets a new font port, the project fonts and seeds are replayed, and the
 * generation never goes back.
 */
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { app, MessageChannelMain, utilityProcess } from 'electron';

import { ENGINE_FONTS_IPC } from '../../../shared/engine-fonts-ipc';
import { EngineFontClient, FONT_CLIENT_ID, installEngineFontService, JOB_FONT_CLIENT_ID } from '../../engine-fonts';
import { NativeEngine } from '../native-engine';
import type { ChildLike, PortLike } from '../ports';
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
      const value = poll();
      if (value) resolve(value);
      else if (Date.now() > deadline) reject(new Error(`timed out waiting for ${label}`));
      else setTimeout(tick, 5);
    };
    tick();
  });
}

/** `f32` bits, the way registry queries key floats. */
const bits = (v: number): number => new Uint32Array(new Float32Array([v]).buffer)[0]!;

type Handler = (event: unknown, ...args: unknown[]) => unknown;
type Result<T> = { ok: true; value: T } | { ok: false; error: string };

async function main(): Promise<void> {
  const enginePath = process.env['PM_IT_ENGINE_PROCESS'];
  const addon = process.env['POWERMOVE_ENGINE_ADDON'];
  check(enginePath && addon, 'PM_IT_ENGINE_PROCESS and POWERMOVE_ENGINE_ADDON are set');
  const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-engine-fonts-it-'));
  const logs: string[] = [];
  const pids: number[] = [];
  const jobPids: number[] = [];

  const engine = new NativeEngine({
    eagerJobs: true,
    spawn: (kind) => {
      const child = utilityProcess.fork(enginePath, [], {
        serviceName: `Powermove ${kind} engine (fonts test)`,
        stdio: ['ignore', 'pipe', 'pipe'],
        env: { ...process.env, POWERMOVE_ENGINE_PROCESS: kind, POWERMOVE_ENGINE_ADDON: addon }
      });
      if (kind === 'interactive') child.on('spawn', () => { if (child.pid) pids.push(child.pid); });
      if (kind === 'job') child.on('spawn', () => { if (child.pid) jobPids.push(child.pid); });
      child.stderr?.on('data', (chunk: Buffer) => logs.push(`[${kind} stderr] ${chunk.toString().trim()}`));
      return child as unknown as ChildLike;
    },
    channel: () => new MessageChannelMain() as unknown as { port1: PortLike; port2: PortLike },
    cacheDirectory: path.join(dir, 'Engine Cache'),
    automaticOutputDirectory: path.join(dir, 'Powermove'),
    physicalMemory: os.totalmem(),
    hostNow: mainHostClock().now,
    log: (level, message) => logs.push(`${level}: ${message}`)
  });

  // The renderer's side: the channels as `ipcMain` would dispatch them.
  const handlers = new Map<string, Handler>();
  const ipc = {
    handle: (channel: string, fn: Handler) => void handlers.set(channel, fn),
    removeHandler: (channel: string) => void handlers.delete(channel)
  };
  const trusted = { id: 7 };
  const invoke = async <T>(channel: string, ...args: unknown[]): Promise<T> => {
    const result = (await handlers.get(channel)!({ sender: trusted }, ...args)) as Result<T>;
    if (!result.ok) throw new Error(`${channel}: ${result.error}`);
    return result.value;
  };

  const client = new EngineFontClient({ log: (level, message) => logs.push(`${level}: ${message}`) });
  const uninstall = installEngineFontService(ipc as never, { isTrustedSender: (e) => (e as unknown as { sender: unknown }).sender === trusted }, client);
  const delivered: { process: string; generation: number }[] = [];
  engine.addClient(FONT_CLIENT_ID, (info, port) => {
    delivered.push({ process: info.process, generation: info.generation });
    client.connect(port);
  }, { peer: 'fonts', processes: ['interactive'] });
  // The job engine's font port mirrors the interactive one's changes.
  const jobClient = new EngineFontClient({ log: (level, message) => logs.push(`job ${level}: ${message}`) });
  client.mirror(jobClient);
  const jobDelivered: { process: string; generation: number }[] = [];
  engine.addClient(JOB_FONT_CLIENT_ID, (info, port) => {
    jobDelivered.push({ process: info.process, generation: info.generation });
    jobClient.connect(port);
  }, { peer: 'fonts', processes: ['job'] });
  const seeds = '{"seeds":[{"family":"system-ui","weight":400,"italic":false,"size":24,"specified":12},{"family":"system-ui","weight":400,"italic":false,"size":26,"specified":13}]}';
  void client.setSeeds(seeds);

  try {
    await engine.start();
    await waitFor('both engines ready', () => engine.status().interactive.state === 'ready' && engine.status().job.state === 'ready');
    await waitFor('font ports', () => client.connected && jobClient.connected);
    check(delivered.length === 1 && delivered[0]!.process === 'interactive', `one font port to the interactive engine (${JSON.stringify(delivered)})`);
    check(jobDelivered.length === 1 && jobDelivered[0]!.process === 'job', `one font port to the job engine (${JSON.stringify(jobDelivered)})`);
    const status = JSON.parse(JSON.stringify(engine.status()));
    step('engines started; main holds a font port to each engine', { delivered: [...delivered], job: [...jobDelivered], status: status.job?.state });

    // A document's families, as pm-wasm asks for them on project open.
    const queries = JSON.stringify(['Helvetica', 'Menlo', 'system-ui'].map((name) => ({ kind: 'family', name, weight: bits(400), italic: false, size: bits(24) })));
    let t = performance.now();
    const answerJson = await invoke<string>(ENGINE_FONTS_IPC.answer, queries);
    const answerMs = performance.now() - t;
    const answers = JSON.parse(answerJson) as { generation: number; faces: { blob: string; index: number; postscriptName: string }[]; answers: unknown[] };
    check(answers.answers.length === 3 && answers.faces.length >= 3, `three family answers (${answerJson.slice(0, 200)})`);
    check(answers.generation >= 1, `the generation is above every earlier one (${answers.generation})`);
    t = performance.now();
    let subsetBytes = 0;
    for (const face of answers.faces) {
      const bytes = await invoke<Uint8Array>(ENGINE_FONTS_IPC.subset, face.blob, face.index);
      check(bytes instanceof Uint8Array && bytes.byteLength > 1000, `a subset for ${face.postscriptName}`);
      subsetBytes += bytes.byteLength;
    }
    const subsetMs = performance.now() - t;
    step('answers and subsets reach the renderer channels from the engine', {
      faces: answers.faces.map((f) => f.postscriptName),
      answerMs: Math.round(answerMs * 100) / 100,
      subsets: answers.faces.length,
      subsetKB: Math.round(subsetBytes / 1024),
      subsetMs: Math.round(subsetMs * 100) / 100
    });

    const manifest = JSON.parse(await invoke<string>(ENGINE_FONTS_IPC.manifest, 'system-ui')) as { baseFamily?: string };
    const faces = JSON.parse(await invoke<string>(ENGINE_FONTS_IPC.familyFaces, 'Helvetica')) as unknown[];
    check(manifest && faces.length > 0, 'level 2 inputs: the fallback manifest and family faces');
    step('level 2 inputs: fallback manifest and family faces');

    // A project font, then an answer that names it.
    const font = await readFile('/System/Library/Fonts/Supplemental/Georgia.ttf');
    const before = await invoke<number>(ENGINE_FONTS_IPC.generation);
    const added = await invoke<number>(ENGINE_FONTS_IPC.addProjectFont, 'Brand Serif', new Uint8Array(font), { weight: '400', style: 'normal' });
    check(added === before + 1, `a project font moves the generation (${before} → ${added})`);
    const brand = JSON.parse(await invoke<string>(ENGINE_FONTS_IPC.answer, JSON.stringify([{ kind: 'family', name: 'Brand Serif', weight: bits(400), italic: false, size: bits(24) }]))) as { generation: number; faces: { custom: boolean }[] };
    check(brand.generation === added && brand.faces.some((f) => f.custom), 'answers name the project font');
    // The job engine got the same project font through the mirror.
    const jobBrand = JSON.parse(await jobClient.answer(JSON.stringify([{ kind: 'family', name: 'Brand Serif', weight: bits(400), italic: false, size: bits(24) }]), null)) as { faces: { custom: boolean }[] };
    check(jobBrand.faces.some((f) => f.custom), 'the job engine answers with the mirrored project font');
    const refused = (await handlers.get(ENGINE_FONTS_IPC.addProjectFont)!({ sender: trusted }, 'Bad', new Uint8Array([1, 2, 3, 4]))) as Result<number>;
    check(!refused.ok, 'data that is not a font is refused');
    const untrusted = await handlers.get(ENGINE_FONTS_IPC.generation)!({ sender: {} });
    check(untrusted === null, 'untrusted senders get nothing');
    step('project fonts: added, answered, bad data refused', { generation: added });

    // Crash: the restarted engine gets a new font port, the project font is
    // replayed, and the generation keeps going up.
    const pid = pids.at(-1)!;
    process.kill(pid, 'SIGKILL');
    await waitFor('a new font port', () => delivered.length === 2 && client.connected, 20_000);
    check(delivered[1]!.process === 'interactive' && delivered[1]!.generation === 2, `the restarted interactive engine's font port (${JSON.stringify(delivered)})`);
    const after = await invoke<number>(ENGINE_FONTS_IPC.generation);
    check(after > added, `the generation never goes back (${added} → ${after})`);
    const replayed = JSON.parse(await invoke<string>(ENGINE_FONTS_IPC.answer, JSON.stringify([{ kind: 'family', name: 'Brand Serif', weight: bits(400), italic: false, size: bits(24) }]))) as { generation: number; faces: { custom: boolean }[] };
    check(replayed.faces.some((f) => f.custom) && replayed.generation === after, 'the project font is replayed to the restarted engine');
    step('interactive engine killed: new font port, project fonts replayed, generation kept rising', { before: added, after });

    // The job engine restarts too: a new font port, the project font and the
    // seeds replayed.
    process.kill(jobPids.at(-1)!, 'SIGKILL');
    await waitFor('a new job font port', () => jobDelivered.length === 2 && jobClient.connected, 20_000);
    const jobAfter = JSON.parse(await jobClient.answer(JSON.stringify([{ kind: 'family', name: 'Brand Serif', weight: bits(400), italic: false, size: bits(24) }]), null)) as { faces: { custom: boolean }[] };
    check(jobAfter.faces.some((f) => f.custom), 'the project font is replayed to the restarted job engine');
    step('job engine killed: new font port, project font replayed', { job: [...jobDelivered] });

    uninstall();
    check(handlers.size === 0, 'uninstall removes every channel');
  } finally {
    client.disconnect();
    jobClient.disconnect();
    await engine.stop();
    await rm(dir, { recursive: true, force: true });
    process.stdout.write(`${JSON.stringify({ steps, totalMs: Math.round(performance.now() - started), log: logs.slice(-60) })}\n`);
  }
}

process.on('uncaughtException', (error) => {
  process.stdout.write(`ENGINE-FONTS-IT FAIL uncaught ${error.stack ?? error.message}\n`);
  app.exit(1);
});
app.dock?.hide();
void app.whenReady().then(() => main()).then(
  () => {
    process.stdout.write('ENGINE-FONTS-IT PASS\n');
    app.exit(0);
  },
  (error: unknown) => {
    process.stdout.write(`ENGINE-FONTS-IT FAIL ${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
    app.exit(1);
  }
);
setTimeout(() => {
  process.stdout.write('ENGINE-FONTS-IT FAIL timeout\n');
  app.exit(1);
}, 90_000).unref();
