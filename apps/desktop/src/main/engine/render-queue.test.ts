import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

import type { SheetExportOptions } from '../../shared/engine-ipc';
import { RenderQueue, type QueueEngine, type QueuePlatform } from './render-queue';

const project = (name = 'Launch') => ({ id: 'P1', name, compId: 'P1-root', w: 1920, h: 1080, fps: 24, dur: 2, layers: [], assets: {} });

const options = (format = 'prores-hq'): SheetExportOptions => ({
  format, alpha: false, fps: 24, scale: 1, start: 0, end: 2, audio: true, motionBlur: false, quality: 'high',
  delivery: { overwrite: 'keep-both' }
});

class FakeEngine implements QueueEngine {
  available = true;
  submitted: Array<{ project: any; options: SheetExportOptions; destination: string; overwrite: string; jobId: string; definitions?: unknown[] }> = [];
  cancelled: string[] = [];
  fail: string | null = null;
  async exportQueued(project: unknown, o: SheetExportOptions, destination: string, overwrite: 'uniqueName' | 'replace' | 'fail', definitions?: any[]) {
    if (this.fail) throw new Error(this.fail);
    const jobId = `engine-${this.submitted.length + 1}`;
    this.submitted.push({ project, options: o, destination, overwrite, jobId, definitions });
    return { jobId, frames: 48 };
  }
  async cancelQueued(jobId: string) {
    this.cancelled.push(jobId);
  }
}

function platform() {
  const sleeps: number[] = [];
  let awake = 0;
  const p: QueuePlatform & { notes: string[]; awake(): number; revealed: string[] } = {
    notes: [],
    revealed: [],
    awake: () => awake,
    preventSleep: () => {
      awake++;
      sleeps.push(awake);
      return () => { awake--; };
    },
    notify: (title, body) => { p.notes.push(`${title}: ${body}`); },
    reveal: (file) => { p.revealed.push(file); }
  };
  return p;
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 12; i++) await tick(); };

let dirs: string[] = [];
let queues: RenderQueue[] = [];
afterEach(async () => {
  // Pending saves finish before the folders go.
  for (const q of queues) await q.dispose();
  for (const d of dirs) await rm(d, { recursive: true, force: true });
  dirs = [];
  queues = [];
});

async function setup() {
  const dir = await mkdtemp(path.join(tmpdir(), 'pm-queue-'));
  dirs.push(dir);
  const engine = new FakeEngine();
  const p = platform();
  const changes = vi.fn();
  const queue = new RenderQueue({ directory: dir, engine, platform: p, onChange: changes, now: () => 1000 });
  queues.push(queue);
  return { dir, engine, p, queue, changes };
}

describe('render queue', () => {
  it('runs jobs unattended from their snapshots, one at a time, and keeps going after a failure', async () => {
    const { engine, p, queue, dir } = await setup();
    const live = project();
    const a = await queue.add({ name: 'A.mov', project: live, options: options(), destination: '/out/A.mov', preset: 'Master · ProRes 422 HQ' });
    live.name = 'edited after queuing';
    const b = await queue.add({ name: 'B.mp4', project: project('B'), options: options('h264'), destination: '/out/B.mp4', sidecars: [{ extension: 'srt', text: '1\n00:00:00,000 --> 00:00:01,000\nHi\n' }] });
    expect(queue.state().jobs.map((j) => [j.id, j.state, j.summary])).toEqual([
      [a, 'pending', 'Master · ProRes 422 HQ · QuickTime · ProRes 422 HQ · 1920×1080 · 24 fps'],
      [b, 'pending', 'MP4 · H.264 · 1920×1080 · 24 fps']
    ]);
    await queue.start();
    await settle();
    expect(engine.submitted).toHaveLength(1);
    expect(engine.submitted[0]!.project.name).toBe('Launch');
    expect(engine.submitted[0]!.overwrite).toBe('uniqueName');
    expect(p.awake()).toBe(1);
    queue.status('engine-1', { jobId: 'engine-1', state: 'running', progress: 0.5 });
    expect(queue.state().jobs[0]).toMatchObject({ state: 'running', progress: 0.5 });
    queue.status('engine-1', { jobId: 'engine-1', state: 'failed', progress: 0.5, error: 'The disk is full.' });
    await settle();
    expect(queue.state().jobs[0]).toMatchObject({ state: 'failed', error: 'The disk is full.' });
    expect(engine.submitted).toHaveLength(2);
    expect(engine.submitted[1]!.destination).toBe('/out/B.mp4');
    const out = await mkdtemp(path.join(tmpdir(), 'pm-queue-out-'));
    dirs.push(out);
    queue.status('engine-2', { jobId: 'engine-2', state: 'done', progress: 1, path: path.join(out, 'B.mp4') });
    await vi.waitFor(() => expect(queue.state().running).toBe(false));
    expect(queue.state()).toMatchObject({ running: false, jobs: [{ state: 'failed' }, { state: 'done', path: path.join(out, 'B.mp4') }] });
    expect(await readFile(path.join(out, 'B.srt'), 'utf8')).toContain('Hi');
    expect(p.awake()).toBe(0);
    expect(p.notes).toEqual(['Render queue finished with errors: 1 export done · 1 failed']);
    // Snapshots live beside the queue file, owner-only.
    const files = (await readdir(dir)).filter((f) => !f.startsWith('.')).sort();
    expect(files).toEqual([`${a}.json`, `${b}.json`, 'queue.json'].sort());
  });

  it('persists user jobs across relaunch: a running job returns as pending and a running queue resumes', async () => {
    const { dir, engine, queue } = await setup();
    await queue.add({ name: 'A.mov', project: project(), options: options(), destination: '/out/A.mov' });
    await queue.add({ name: 'B.mov', project: project(), options: options(), destination: '/out/B.mov' });
    await queue.start();
    await settle();
    queue.status('engine-1', { jobId: 'engine-1', state: 'running', progress: 0.3 });
    await queue.dispose();
    expect(engine.cancelled).toEqual(['engine-1']);
    const stored = JSON.parse(await readFile(path.join(dir, 'queue.json'), 'utf8'));
    expect(stored.running).toBe(true);
    expect(stored.jobs.map((j: any) => j.state)).toEqual(['pending', 'pending']);
    // Relaunch: the engine starts later; the queue resumes when it is ready.
    const engine2 = new FakeEngine();
    engine2.available = false;
    const again = new RenderQueue({ directory: dir, engine: engine2, platform: platform() });
    queues.push(again);
    await again.ready();
    expect(again.state()).toMatchObject({ running: true, jobs: [{ state: 'pending' }, { state: 'pending' }] });
    engine2.available = true;
    again.engineReady();
    await settle();
    expect(engine2.submitted.map((s) => s.destination)).toEqual(['/out/A.mov']);
  });

  it('keeps the definitions a job was queued with across relaunch and removes them with the job', async () => {
    const { dir, queue } = await setup();
    const captured = [{ generation: 1, full: true, upserts: [{ id: 'blur', kind: 'effect', origin: 'fork' }], removals: [] }];
    const a = await queue.add({ name: 'A.mov', project: project(), options: options(), destination: '/out/A.mov', definitions: captured as any });
    await queue.add({ name: 'B.mov', project: project(), options: options(), destination: '/out/B.mov' });
    const file = path.join(dir, `${a}.definitions.json`);
    expect(JSON.parse(await readFile(file, 'utf8'))).toEqual(captured);
    await queue.dispose();
    // Relaunch: the job renders with the set it was queued with, not the
    // windows' sets now; a job queued without one gets none.
    const engine2 = new FakeEngine();
    const again = new RenderQueue({ directory: dir, engine: engine2, platform: platform() });
    queues.push(again);
    await again.start();
    await settle();
    expect(engine2.submitted[0]!.definitions).toEqual(captured);
    again.status('engine-1', { jobId: 'engine-1', state: 'done', progress: 1, path: '/out/A.mov' });
    await settle();
    expect(engine2.submitted[1]!.definitions).toEqual([]);
    await again.act({ kind: 'clearFinished' });
    await vi.waitFor(async () => expect((await readdir(dir)).filter((f) => f.endsWith('.definitions.json'))).toEqual([]));
  });

  it('logs a queued job whose definitions cannot be read and reports its effects instead of drawing another set', async () => {
    const { dir, queue } = await setup();
    const logs: string[] = [];
    const a = await queue.add({ name: 'A.mov', project: project(), options: options(), destination: '/out/A.mov', definitions: [] as any });
    await queue.add({ name: 'B.mov', project: project(), options: options(), destination: '/out/B.mov' });
    await writeFile(path.join(dir, `${a}.definitions.json`), '{ not json');
    await queue.dispose();
    const engine2 = new FakeEngine();
    const p2 = { ...platform(), log: (level: string, message: string) => { logs.push(`${level}: ${message}`); } };
    const again = new RenderQueue({ directory: dir, engine: engine2, platform: p2 });
    queues.push(again);
    await again.start();
    await settle();
    expect(engine2.submitted[0]!.definitions).toEqual([]);
    expect(logs.filter((l) => l.startsWith('warn: the definitions of A.mov could not be read'))).toHaveLength(1);
    again.status('engine-1', { jobId: 'engine-1', state: 'done', progress: 1, path: '/out/A.mov' });
    await settle();
    expect(engine2.submitted[1]!.definitions).toEqual([]);
    expect(logs.filter((l) => l.startsWith('info: B.mov was queued without definitions'))).toHaveLength(1);
  });

  it('cancels, retries, reorders, removes, and reveals', async () => {
    const { engine, p, queue } = await setup();
    const a = await queue.add({ name: 'A.mov', project: project(), options: options(), destination: '/out/A.mov' });
    const b = await queue.add({ name: 'B.mov', project: project(), options: options(), destination: '/out/B.mov' });
    const c = await queue.add({ name: 'C.mov', project: project(), options: options(), destination: '/out/C.mov' });
    await queue.act({ kind: 'move', id: c, to: 0 });
    expect(queue.state().jobs.map((j) => j.id)).toEqual([c, a, b]);
    await queue.act({ kind: 'cancel', id: b });
    await queue.act({ kind: 'start' });
    await settle();
    expect(engine.submitted.map((s) => s.destination)).toEqual(['/out/C.mov']);
    await queue.act({ kind: 'cancel', id: c });
    expect(engine.cancelled).toEqual(['engine-1']);
    queue.status('engine-1', { jobId: 'engine-1', state: 'cancelled', progress: 0.2 });
    await settle();
    expect(engine.submitted.map((s) => s.destination)).toEqual(['/out/C.mov', '/out/A.mov']);
    queue.status('engine-2', { jobId: 'engine-2', state: 'done', progress: 1, path: '/out/A.mov' });
    await vi.waitFor(() => expect(queue.state().jobs.find((j) => j.id === a)!.state).toBe('done'));
    expect(queue.state().jobs.map((j) => j.state)).toEqual(['cancelled', 'done', 'cancelled']);
    await queue.act({ kind: 'reveal', id: a });
    expect(p.revealed).toEqual(['/out/A.mov']);
    await queue.act({ kind: 'retry', id: b });
    expect(queue.state().jobs.find((j) => j.id === b)!.state).toBe('pending');
    await queue.act({ kind: 'clearFinished' });
    expect(queue.state().jobs.map((j) => j.id)).toEqual([b]);
    await queue.act({ kind: 'remove', id: b });
    expect(queue.state().jobs).toEqual([]);
  });

  it('shows waiting for media and routed jobs as such', async () => {
    const { queue } = await setup();
    await queue.add({ name: 'A.mov', project: project(), options: options(), destination: '/out/A.mov' });
    await queue.start();
    await settle();
    queue.status('engine-1', { jobId: 'engine-1', state: 'waitingForMedia', progress: 0, waiting: '2 clips are offline: a.mov, b.mov.' });
    expect(queue.state().jobs[0]).toMatchObject({ state: 'waitingForMedia', error: '2 clips are offline: a.mov, b.mov.' });
    queue.status('engine-1', { jobId: 'engine-1', state: 'failed', progress: 0, routed: true, error: 'text layers' });
    await settle();
    expect(queue.state().jobs[0]!.error).toContain('cannot render this project yet (text layers)');
  });

  it('lists agent exports while they run and keeps the Mac awake for them', async () => {
    const { p, queue } = await setup();
    queue.agentJob({ jobId: 'agent-export-1', owner: 3, name: 'Cut', summary: 'MP4 · H.264 · 1280×720', status: { state: 'running', progress: 0.1 } });
    expect(queue.state().jobs[0]).toMatchObject({ source: 'agent', state: 'running', name: 'Cut' });
    expect(p.awake()).toBe(1);
    const cancel = vi.fn(async () => {});
    await queue.act({ kind: 'cancel', id: 'agent-export-1' }, cancel);
    expect(cancel).toHaveBeenCalledWith(3, 'agent-export-1');
    queue.agentJob({ jobId: 'agent-export-1', owner: 3, name: 'Cut', summary: 'MP4 · H.264 · 1280×720', status: { state: 'done', progress: 1, path: '/Users/x/Powermove/Cut.mp4' } });
    expect(queue.state().jobs[0]).toMatchObject({ state: 'done', path: '/Users/x/Powermove/Cut.mp4' });
    expect(p.awake()).toBe(0);
  });

  it('refuses invalid jobs', async () => {
    const { queue } = await setup();
    await expect(queue.add({ name: 'x', project: project(), options: { ...options(), fps: 0 }, destination: '/out/x.mov' })).rejects.toThrow('Invalid');
    await expect(queue.add({ name: 'x', project: project(), options: options(), destination: 'relative.mov' })).rejects.toThrow('destination');
    await expect(queue.add({ name: 'x', project: { name: 'no comp' }, options: options(), destination: '/out/x.mov' })).rejects.toThrow('project');
  });
});
