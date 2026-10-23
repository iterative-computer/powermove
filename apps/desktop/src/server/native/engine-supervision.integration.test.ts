/**
 * The macOS hidden host's engine children over its Node IPC transport
 * (ports.ts, engine-child.ts) under the desktop's supervisor (plan section 22,
 * "Crash handling"; survey row 88): a killed job or interactive child is
 * reported with its exit and attempt, restarted with the bounded backoff, and
 * work for every project continues on the new launch, with each project's
 * definitions pushed to it again. Real `pm_node` children, so it runs where
 * the addon is built: POWERMOVE_ENGINE_ADDON (every gate script sets it).
 */
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import { NativeEngine } from '../../main/engine/native-engine';
import { AgentJobs, type AgentJobsEngine } from '../../main/engine/agent-jobs';
import { restartBackoffMs } from '../../main/engine/crash';
import type { CrashReport, EngineProcess, Request } from '../../shared/engine-protocol';
import { NodeEngineTransport } from './ports';

const addon = process.env['POWERMOVE_ENGINE_ADDON'];
const dist = process.env['POWERMOVE_NATIVE_HOST_DIST'];

const channel = (v: number) => ({ v, kf: [], expr: null });
/** A 160 × 90 solid project (no media, no text) for `id`. */
function project(id: string, color: string) {
  const p = Object.fromEntries([['anchor.x', 0], ['anchor.y', 0], ['position.x', 0], ['position.y', 0], ['scale.x', 100], ['scale.y', 100], ['rotation', 0], ['opacity', 100], ['skew', 0]].map(([k, v]) => [k, channel(v as number)]));
  return {
    id, name: id, w: 160, h: 90, fps: 24, dur: 1, work: [0, 1], compId: `${id}-root`,
    backgroundFill: { type: 'solid', angle: 0, stops: [{ id: 's', color: '#000000', position: 0 }] },
    layers: [{ id: 'bg', type: 'solid', name: 'bg', from: 0, dur: 1, on: true, solo: false, blend: 'normal', mblur: false, parent: null, group: null, matteSource: null,
      threeD: false, p, fx: [], masks: [], d: { color, w: 160, h: 90 }, transitionIn: null, transitionOut: null }],
    assets: {}, comps: {}
  };
}

describe.runIf(process.platform === 'darwin' && !!addon && !!dist)('hidden host engine supervision over Node IPC', () => {
  let dir = '';
  let engine: NativeEngine;
  let jobs: AgentJobs;
  const reports: CrashReport[] = [];
  const sent: Array<{ process: EngineProcess; type: Request['type']; session: number | null }> = [];
  const log: string[] = [];

  const ready = async (process: EngineProcess, generation: number) => {
    for (const until = Date.now() + 60_000; ;) {
      const status = engine.status()[process];
      if (status.state === 'ready' && status.generation >= generation && engine.isReady(process)) return status;
      if (Date.now() > until) throw new Error(`the ${process} engine did not become ready: ${JSON.stringify(engine.status())}\n${log.slice(-20).join('\n')}`);
      await new Promise(resolve => setTimeout(resolve, 50));
    }
  };
  const kill = (process: EngineProcess) => {
    const pid = engine.status()[process].pid;
    if (!pid) throw new Error(`no ${process} engine to kill`);
    globalThis.process.kill(pid, 'SIGKILL');
    return pid;
  };
  const reported = async (count: number) => {
    for (const until = Date.now() + 20_000; reports.length < count;) {
      if (Date.now() > until) throw new Error(`no crash report: ${log.slice(-20).join('\n')}`);
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    return reports[count - 1]!;
  };
  const frames = (owner: number, id: string, color: string) =>
    jobs.renderFrames(owner, `run-${id}`, { project: project(id, color), times: [0, 0.5], width: 160, revision: 1, imageFormat: 'png' });

  // Kill only after the production child reports a real in-flight asset over
  // Node IPC. This exercises attribution, not an injected host breadcrumb.
  const crashInFlight = async (count: number) => {
    const supervisor = engine.supervisor('job'), record = supervisor.recordInFlight.bind(supervisor);
    let killed = false;
    const spy = vi.spyOn(supervisor, 'recordInFlight').mockImplementation(crumb => {
      record(crumb);
      if (!killed && crumb.suspects.some(scope => scope.kind === 'asset' && scope.asset === 'attribution-image')) {
        killed = true; kill('job');
      }
    });
    try {
      const document: any = project('attribution', '#ffffff');
      document.w = 3840; document.h = 2160;
      document.assets = { 'attribution-image': { id: 'attribution-image', kind: 'image', name: 'image.png', w: 32, h: 16 } };
      document.layers[0].type = 'image'; document.layers[0].d = { asset: 'attribution-image', w: 3840, h: 2160 };
      const result = jobs.renderFrames(3, 'run-attribution', { project: document, times: [0, 0.1, 0.2, 0.3, 0.4], width: 3840, revision: 1, imageFormat: 'png' }).then(() => null, error => error);
      const report = await reported(count);
      expect(killed).toBe(true); expect(await result).toBeInstanceOf(Error);
      expect(report.suspects).toContainEqual({ kind: 'asset', asset: 'attribution-image' });
      return report;
    } finally { spy.mockRestore(); }
  };

  beforeAll(async () => {
    dir = await mkdtemp(path.join(os.tmpdir(), 'pm-hidden-supervision-'));
    execFileSync(path.join(process.env['PM_FFMPEG_DIR']!, 'bin/ffmpeg'), ['-v', 'error', '-f', 'lavfi', '-i', 'color=c=red:s=32x16', '-frames:v', '1', path.join(dir, 'image.png')]);
    // Build before acquiring the Electron lease; run the shipping child entry.
    const child = path.join(dist!, 'native-engine-child.cjs');
    const transport = new NodeEngineTransport();
    const spawn = transport.spawn(child, addon!, line => log.push(line), { POWERMOVE_ENGINE_SHADER_CACHE: path.join(dir, 'Shader Cache') });
    engine = new NativeEngine({ spawn, channel: transport.channel, cacheDirectory: path.join(dir, 'Engine Cache'), automaticOutputDirectory: path.join(dir, 'out'),
      physicalMemory: os.totalmem(), hostNow: () => Math.round(performance.now() * 1000), log: (level, message) => log.push(`[${level}] ${message}`),
      resolveAsset: async id => id === 'attribution-image' ? { path: path.join(dir, 'image.png'), contentHash: null } : null });
    engine.listen({ crash: report => reports.push(report) });
    // Every request AgentJobs makes, as the job engine receives it.
    const recorded: AgentJobsEngine = {
      request: (process, body, session = null, revision = null) => { sent.push({ process, type: body.type, session }); return engine.request(process, body, session, revision); },
      stageDocument: (process, session, bytes) => engine.stageDocument(process, session, bytes),
      listen: listener => engine.listen(listener),
      isReady: process => engine.isReady(process)
    };
    // Each project's winning definitions (empty sets here) at version 1.
    jobs = new AgentJobs({ engine: recorded, definitions: owner => owner >= 1 && owner <= 3 ? { version: 1, updates: (generation: number) => [{ generation, full: true, upserts: [], removals: [] }] } : null });
    await engine.start();
    await ready('interactive', 1);
    await ready('job', 1);
  }, 120_000);

  afterAll(async () => {
    jobs?.dispose();
    await engine?.stop();
    if (dir) await rm(dir, { recursive: true, force: true });
  }, 60_000);

  it('restarts a killed job child with attribution and backoff, and every project renders again with its definitions', async () => {
    // Two projects with their own sessions on the first launch.
    expect((await frames(1, 'A', '#ff0000')).images).toHaveLength(2);
    expect((await frames(2, 'B', '#00ff00')).images).toHaveLength(2);
    const before = engine.status();
    const pushesBefore = sent.filter(s => s.type === 'pushDefinitions').length;
    expect(pushesBefore).toBe(2);

    const killedAt = Date.now(), killed = before.job.pid;
    const report = await crashInFlight(1);
    // SIGKILL over Node IPC reaches the supervisor as status 9 (ports.ts), named as such.
    expect(report).toMatchObject({ process: 'job', attempt: 1, restartInMs: restartBackoffMs(1) });
    expect(report.exit).toBe('exit status 9 (SIGKILL if signalled)');
    expect(report.markedFailed).toEqual([]);
    const after = await ready('job', before.job.generation + 1);
    expect(Date.now() - killedAt).toBeGreaterThanOrEqual(report.restartInMs - 10);
    expect(after.pid).not.toBe(killed);
    // The interactive engine was not touched.
    expect(engine.status().interactive.generation).toBe(before.interactive.generation);
    const mark = sent.length;

    // Both projects render on the new launch: each opens a session again and
    // gets its definitions pushed (acknowledged) before its first frame there.
    expect((await frames(2, 'B', '#00ff00')).images).toHaveLength(2);
    expect((await frames(1, 'A', '#ff0000')).images).toHaveLength(2);
    const since = sent.slice(mark);
    const sessions = new Set(since.filter(s => s.type === 'agentFrames').map(s => s.session));
    expect(sessions.size).toBe(2);
    for (const session of sessions) {
      const push = since.findIndex(s => s.type === 'pushDefinitions' && s.session === session);
      expect(push).toBeGreaterThanOrEqual(0);
      expect(push).toBeLessThan(since.findIndex(s => s.type === 'agentFrames' && s.session === session));
    }
  }, 120_000);

  it('backs off a child that crashes again soon after its restart, within the bound', async () => {
    const generation = engine.status().job.generation;
    const report = await crashInFlight(2);
    expect(report).toMatchObject({ process: 'job', attempt: 2, restartInMs: restartBackoffMs(2) });
    expect(restartBackoffMs(2)).toBeGreaterThan(restartBackoffMs(1));
    expect(restartBackoffMs(100)).toBe(30_000);
    expect(report.markedFailed).toEqual([{ kind: 'asset', asset: 'attribution-image' }]);
    await ready('job', generation + 1);
    expect((await frames(2, 'B', '#00ff00')).images).toHaveLength(2);
  }, 120_000);

  it('restarts a killed interactive child while the job engine keeps rendering', async () => {
    const before = engine.status();
    kill('interactive');
    const report = await reported(3);
    expect(report).toMatchObject({ process: 'interactive', attempt: 1 });
    expect(report.exit).toBe('exit status 9 (SIGKILL if signalled)');
    // Frames come from the job engine during the interactive restart.
    expect((await frames(1, 'A', '#ff0000')).images).toHaveLength(2);
    await ready('interactive', before.interactive.generation + 1);
    expect(engine.status().job.generation).toBe(before.job.generation);
  }, 120_000);
});
