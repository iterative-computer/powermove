// Host services in main: capability handles, output destinations, leases,
// hardware sessions, crash attribution, and the definition relay.
import { mkdtemp, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import type { DefinitionSnapshot, LeaseUsage } from '../../shared/engine-protocol';
import { ACK_DEADLINE_MS, globalBudget, HardwareArbiter, LeaseError, MemoryArbiter } from './arbiter';
import { CrashLedger, describeExit, restartBackoffMs, RestartPolicy } from './crash';
import { DefinitionGap, DefinitionRelay, DefinitionSet } from './definitions';
import { HandleRejected, HandleTable } from './handles';
import { candidateName, OutputDestinations, OutputError } from './outputs';

const GiB = 1024 ** 3;

describe('capability handles', () => {
  it('are 128 random bits as 32 lowercase hex digits, bound to a session and a port', () => {
    const table = new HandleTable();
    const handle = table.mint({ kind: { kind: 'mediaRead', asset: 'a1' }, session: 1, port: 7, path: '/Users/x/clip.mov' });
    expect(handle).toMatch(/^[0-9a-f]{32}$/);
    expect(table.resolve(handle, 1, 7)).toEqual({ handle, kind: { kind: 'mediaRead', asset: 'a1' }, access: 'read', path: '/Users/x/clip.mov' });
    const reason = (fn: () => unknown) => {
      try { fn(); } catch (error) { return (error as HandleRejected).reason; }
      return null;
    };
    expect(reason(() => table.check(handle, 2, 7))).toBe('wrongSession');
    expect(reason(() => table.check(handle, 1, 8))).toBe('wrongPort');
    expect(reason(() => table.check('0'.repeat(32), 1, 7))).toBe('unknown');
    const other = table.mint({ kind: { kind: 'cache', cache: 'scratch' }, session: 2, port: 7, path: '/tmp/cache' });
    expect(table.resolve(other, 2, 7).access).toBe('readWrite');
    expect(table.revokeSession(1, 7)).toBe(1);
    expect(table.revokePort(7)).toBe(1);
    expect(table.size).toBe(0);
  });

  it('refuse relative paths and a repeating random source', () => {
    const table = new HandleTable(() => new Uint8Array(16).fill(3));
    expect(() => table.mint({ kind: { kind: 'projectStore' }, session: 0, port: 1, path: 'relative/file' })).toThrow();
    table.mint({ kind: { kind: 'projectStore' }, session: 0, port: 1, path: '/a' });
    expect(() => table.mint({ kind: { kind: 'projectStore' }, session: 0, port: 1, path: '/b' })).toThrow(/repeated/);
  });
});

describe('output destinations', () => {
  let dir: string;
  let outputs: OutputDestinations;
  let handles: HandleTable;
  beforeEach(async () => {
    dir = await mkdtemp(path.join(os.tmpdir(), 'pm-outputs-'));
    handles = new HandleTable();
    outputs = new OutputDestinations({ handles, automaticDirectory: path.join(dir, 'Powermove') });
  });
  afterEach(async () => {
    await rm(dir, { recursive: true, force: true });
  });

  const agent = (name = 'Launch cut', extension = 'mov') => ({
    job: 'job-1',
    choice: { kind: 'agentAutomatic' as const, suggestedName: name, extension },
    overwrite: 'uniqueName' as const
  });
  const verified = (bytes: number) => ({ codec: 'prores', frames: 24, audioSamples: 0, bytes });

  it('creates an owner-only temporary file and commits it under a unique name', async () => {
    const first = await outputs.destination(agent(), 3, 9);
    const temp = handles.resolve(first.temporary, 3, 9);
    expect(temp.access).toBe('write');
    expect(path.dirname(temp.path)).toBe(path.join(dir, 'Powermove'));
    expect((await stat(temp.path)).mode & 0o777).toBe(0o600);
    await writeFile(temp.path, 'frames');
    const done = await outputs.commit({ job: 'job-1', temporary: first.temporary, commit: true, verification: verified(6) }, 3, 9);
    expect(done.path).toBe(path.join(dir, 'Powermove', 'Launch cut.mov'));
    expect(await readFile(done.path!, 'utf8')).toBe('frames');
    // The same name again gets "(2)", never replacing the first.
    const second = await outputs.destination(agent(), 3, 9);
    await writeFile(handles.resolve(second.temporary, 3, 9).path, 'again');
    const again = await outputs.commit({ job: 'job-1', temporary: second.temporary, commit: true, verification: verified(5) }, 3, 9);
    expect(again.path).toBe(path.join(dir, 'Powermove', 'Launch cut (2).mov'));
    expect(await readFile(done.path!, 'utf8')).toBe('frames');
    expect((await readdir(path.join(dir, 'Powermove'))).sort()).toEqual(['Launch cut (2).mov', 'Launch cut.mov']);
    expect(handles.size).toBe(0);
  });

  it('never lets an agent choose a path', async () => {
    for (const bad of [agent('../../etc/passwd'), agent('a/b'), agent('..'), agent('ok', 'mp4/x'), agent('ok', 'm.p4'), { ...agent(), overwrite: 'replace' as const }]) {
      await expect(outputs.destination(bad, 1, 1)).rejects.toBeInstanceOf(OutputError);
    }
    expect(candidateName('x'.repeat(300), 'mov', 3)).toHaveLength(255);
    expect(candidateName('x'.repeat(300), 'mov', 3).endsWith(' (4).mov')).toBe(true);
  });

  it('removes empty, unverified, and abandoned outputs', async () => {
    const empty = await outputs.destination(agent(), 1, 2);
    const emptyPath = handles.resolve(empty.temporary, 1, 2).path;
    await expect(outputs.commit({ job: 'job-1', temporary: empty.temporary, commit: true, verification: verified(0) }, 1, 2)).rejects.toThrow(/empty/);
    await expect(stat(emptyPath)).rejects.toThrow();

    const unverified = await outputs.destination(agent(), 1, 2);
    const unverifiedPath = handles.resolve(unverified.temporary, 1, 2).path;
    await writeFile(unverifiedPath, 'data');
    await expect(outputs.commit({ job: 'job-1', temporary: unverified.temporary, commit: true, verification: null }, 1, 2)).rejects.toThrow(/verified/);
    await expect(stat(unverifiedPath)).rejects.toThrow();

    const cancelled = await outputs.destination(agent(), 1, 2);
    const cancelledPath = handles.resolve(cancelled.temporary, 1, 2).path;
    expect(await outputs.commit({ job: 'job-1', temporary: cancelled.temporary, commit: false, verification: null }, 1, 2)).toEqual({ job: 'job-1', path: null });
    await expect(stat(cancelledPath)).rejects.toThrow();

    const crashed = await outputs.destination(agent(), 1, 2);
    const crashedPath = handles.resolve(crashed.temporary, 1, 2).path;
    await outputs.destination(agent(), 1, 3);
    expect(await outputs.abandonPort(2)).toBe(1);
    await expect(stat(crashedPath)).rejects.toThrow();
    expect(outputs.size).toBe(1);
    expect(await readdir(path.join(dir, 'Powermove'))).toHaveLength(1);
  });

  it('refuses commits from another session, port, or job', async () => {
    const out = await outputs.destination(agent(), 1, 2);
    await expect(outputs.commit({ job: 'job-1', temporary: out.temporary, commit: true, verification: verified(1) }, 5, 2)).rejects.toThrow(/session/);
    await expect(outputs.commit({ job: 'job-1', temporary: out.temporary, commit: true, verification: verified(1) }, 1, 3)).rejects.toThrow(/port/);
    await expect(outputs.commit({ job: 'other', temporary: out.temporary, commit: true, verification: verified(1) }, 1, 2)).rejects.toThrow(/job/);
  });

  it('places user-chosen outputs by the overwrite policy', async () => {
    const target = path.join(dir, 'Exports', 'Final.mp4');
    await writeFile(path.join(dir, 'existing.mp4'), 'old');
    const write = async (policy: 'replace' | 'fail' | 'uniqueName', file: string, content: string) => {
      const destination = outputs.chooseDestination(file, 'job-u', 4, 4);
      const out = await outputs.destination({ job: 'job-u', choice: { kind: 'userChosen', destination }, overwrite: policy }, 4, 4);
      await writeFile(handles.resolve(out.temporary, 4, 4).path, content);
      return outputs.commit({ job: 'job-u', temporary: out.temporary, commit: true, verification: verified(content.length) }, 4, 4);
    };
    expect((await write('fail', target, 'one')).path).toBe(target);
    await expect(write('fail', target, 'two')).rejects.toThrow(/exists/);
    expect(await readFile(target, 'utf8')).toBe('one');
    expect((await write('replace', target, 'three')).path).toBe(target);
    expect(await readFile(target, 'utf8')).toBe('three');
    expect((await write('uniqueName', target, 'four')).path).toBe(path.join(dir, 'Exports', 'Final (2).mp4'));
    expect((await readdir(path.join(dir, 'Exports'))).sort()).toEqual(['Final (2).mp4', 'Final.mp4']);
  });

  it('sweeps only its own leftover temporary files', async () => {
    const auto = path.join(dir, 'Powermove');
    await outputs.destination(agent(), 1, 1);
    await writeFile(path.join(auto, '.powermove-zz.part'), '');
    await writeFile(path.join(auto, 'keep.mov'), '');
    expect(await outputs.sweepAutomaticDirectory()).toBe(1);
    expect((await readdir(auto)).sort()).toEqual(['.powermove-zz.part', 'keep.mov']);
  });
});

describe('memory arbiter', () => {
  const usage = (process: 'interactive' | 'job', epoch: number, total: number, ack: boolean): LeaseUsage => ({
    process, epoch, total, ack,
    categories: { frameCache: total, renderCache: 0, blenderSurfaces: 0, geometry3d: 0, surfacePool: 0, decoders: 0, pipelines: 0, other: 0 }
  });

  it('sizes one global budget from physical memory', () => {
    expect(globalBudget(8 * GiB)).toBe(4 * GiB);
    expect(globalBudget(16 * GiB)).toBe(8 * GiB);
    expect(globalBudget(128 * GiB)).toBe(64 * GiB);
    expect(globalBudget(4 * GiB)).toBe(1 * GiB);
  });

  it('shrinks the job engine for playback and grows the interactive engine only after the job engine gives memory back', () => {
    let clock = 0;
    const arbiter = new MemoryArbiter(10_000, () => clock);
    const [interactive] = [arbiter.initial('interactive')];
    const job = arbiter.initial('job');
    expect(interactive!.lease.bytes).toBe(6000);
    expect(job.lease.bytes).toBe(4000);
    arbiter.report(usage('interactive', interactive!.lease.epoch, 5000, true));
    arbiter.report(usage('job', job.lease.epoch, 3500, true));

    const grants = arbiter.playbackStarted();
    const jobShrink = grants.find((g) => g.process === 'job')!;
    const interactiveGrow = grants.find((g) => g.process === 'interactive')!;
    expect(jobShrink.lease.bytes).toBe(1500);
    expect(jobShrink.lease.reason).toBe('playbackStarted');
    // The job engine may still hold 4,000 bytes: the interactive engine waits.
    expect(interactiveGrow.lease.bytes).toBe(6000);
    expect(arbiter.report(usage('job', jobShrink.lease.epoch, 3500, false))).toEqual([]);
    clock = ACK_DEADLINE_MS + 1;
    expect(arbiter.overdue()).toContain('job');
    const unblocked = arbiter.report(usage('job', jobShrink.lease.epoch, 1400, true));
    expect(unblocked).toHaveLength(1);
    expect(unblocked[0]!.process).toBe('interactive');
    expect(unblocked[0]!.lease.bytes).toBe(8500);
    expect(() => arbiter.report(usage('job', jobShrink.lease.epoch, 1600, true))).toThrow(LeaseError);

    // Stop: the interactive engine shrinks first; the job engine regrows after it acknowledges.
    const stop = arbiter.playbackStopped();
    const interactiveShrink = stop.find((g) => g.process === 'interactive')!;
    expect(interactiveShrink.lease.bytes).toBe(6000);
    expect(stop.find((g) => g.process === 'job')!.lease.bytes).toBe(1500);
    const regrow = arbiter.report(usage('interactive', interactiveShrink.lease.epoch, 5900, true));
    expect(regrow[0]).toMatchObject({ process: 'job', lease: { bytes: 4000, reason: 'playbackStopped' } });
  });

  it('ignores stale reports, grants requests from free memory, and frees a dead engine', () => {
    const arbiter = new MemoryArbiter(10_000);
    const i = arbiter.initial('interactive');
    const j = arbiter.initial('job');
    arbiter.report(usage('job', j.lease.epoch, 100, true));
    arbiter.report(usage('interactive', i.lease.epoch, 100, true));
    const more = arbiter.request({ process: 'job', wanted: 9000, reason: 'requested' });
    expect(more.lease.bytes).toBe(4000);
    expect(arbiter.report(usage('job', j.lease.epoch, 100, false))).toEqual([]);
    arbiter.playbackStarted();
    const afterDeath = arbiter.exited('interactive');
    expect(arbiter.isPlaying).toBe(false);
    expect(afterDeath.find((g) => g.process === 'job')?.lease.reason).toBe('peerExited');
  });
});

describe('hardware sessions', () => {
  it('are capped per codec across both engines and freed with an engine', () => {
    const arbiter = new HardwareArbiter();
    expect(arbiter.request('job', { kind: 'encode', delta: 1 })).toEqual({ kind: 'encode', held: 1 });
    expect(arbiter.request('interactive', { kind: 'encode', delta: 5 })).toEqual({ kind: 'encode', held: 1 });
    expect(arbiter.request('job', { kind: 'encode', delta: 1 })).toEqual({ kind: 'encode', held: 1 });
    expect(arbiter.request('job', { kind: 'encode', delta: -3 })).toEqual({ kind: 'encode', held: 0 });
    expect(() => arbiter.request('job', { kind: 'encode', delta: 0 })).toThrow();
    arbiter.releaseAll('interactive');
    expect(arbiter.total('encode')).toBe(0);
  });
});

describe('crash handling', () => {
  it('backs off 250 ms doubling to 30 s', () => {
    expect([0, 1, 2, 3, 8, 1000].map(restartBackoffMs)).toEqual([250, 250, 500, 1000, 30_000, 30_000]);
    expect(describeExit(0)).toBe('exit status 0');
    expect(describeExit(9)).toBe('exit status 9 (SIGKILL if signalled)');
  });

  it('resets the attempt count after a healthy interval', () => {
    let now = 0;
    const policy = new RestartPolicy(() => now);
    policy.started();
    expect(policy.crashed()).toEqual({ attempt: 1, delayMs: 250 });
    policy.started();
    now += 1000;
    expect(policy.crashed()).toEqual({ attempt: 2, delayMs: 500 });
    policy.started();
    now += 60_000;
    expect(policy.crashed().attempt).toBe(1);
  });

  it('marks an item failed on its second attributed crash, once', () => {
    const ledger = new CrashLedger();
    const asset = { kind: 'asset' as const, asset: 'clip' };
    expect(ledger.record([asset, { kind: 'connection' }])).toEqual([]);
    expect(ledger.record([asset, asset])).toEqual([asset]);
    expect(ledger.record([asset])).toEqual([]);
    expect(ledger.isFailed(asset)).toBe(true);
    expect(ledger.failed()).toEqual([asset]);
    expect(ledger.isFailed({ kind: 'session', session: 1 })).toBe(false);
  });
});

describe('definition relay', () => {
  const effect = (id: string): DefinitionSnapshot => ({
    id, kind: 'effect', version: 0, origin: { kind: 'builtIn' } as never, renderer: null, glsl: null, params: [],
    flags: {} as never
  });

  it('applies updates in generation order like the engine', () => {
    const set = new DefinitionSet();
    expect(() => set.apply({ generation: 1, full: false, upserts: [], removals: [] })).toThrow(DefinitionGap);
    set.apply({ generation: 1, full: true, upserts: [effect('a'), effect('b')], removals: [] });
    set.apply({ generation: 2, full: false, upserts: [], removals: [{ id: 'a', kind: 'effect' }] });
    expect(() => set.apply({ generation: 4, full: false, upserts: [], removals: [] })).toThrow(/does not follow/);
    expect(set.size).toBe(1);
    expect(set.missingIds()).toEqual(['a']);
    expect(set.fullUpdate(1)).toMatchObject({ generation: 1, full: true, upserts: [{ id: 'b' }] });
    expect(() => set.apply({ generation: 3, full: false, upserts: [effect('c'), effect('c')], removals: [] })).toThrow(/twice/);
  });

  it('keeps each window\'s latest set and restarts after a reload', () => {
    const relay = new DefinitionRelay();
    expect(() => relay.update(1, { generation: 1, full: false, upserts: [], removals: [] })).toThrow();
    relay.update(1, { generation: 1, full: true, upserts: [effect('a')], removals: [] });
    relay.update(1, { generation: 2, full: false, upserts: [effect('b')], removals: [] });
    expect(relay.snapshotFor(1)?.upserts.map((d) => d.id)).toEqual(['a', 'b']);
    relay.update(1, { generation: 1, full: true, upserts: [effect('z')], removals: [] });
    expect(relay.snapshotFor(1)?.upserts.map((d) => d.id)).toEqual(['z']);
    expect(relay.snapshotFor(2)).toBe(null);
    expect(() => relay.update(1, { nope: true })).toThrow();
  });
});
