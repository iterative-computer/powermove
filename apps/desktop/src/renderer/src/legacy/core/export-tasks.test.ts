import { describe, expect, it, vi } from 'vitest';
import type { RenderQueueJob, RenderQueueState } from '../../../../shared/engine-ipc';
import { trackExportTasks } from './export-tasks';

function job(state: RenderQueueJob['state'], extra: Partial<RenderQueueJob> = {}): RenderQueueJob {
  return { id: 'q1', source: 'user', name: 'Film.mp4', summary: 'H.264 · 1920×1080', destination: '/x/Film.mp4', state, progress: 0, createdAt: 1, ...extra };
}

function fixture(initial: RenderQueueState = { running: false, jobs: [] }) {
  let listener!: (state: RenderQueueState) => void;
  let listed!: (state: RenderQueueState) => void;
  const queue = {
    queueList: vi.fn(() => new Promise<RenderQueueState>((resolve) => { listed = resolve; })),
    queueAction: vi.fn(async () => initial),
    onQueueChanged: vi.fn((fn: (state: RenderQueueState) => void) => { listener = fn; return () => {}; })
  };
  const toasts: Array<[string, any]> = [];
  const dismissed: string[] = [];
  const PM: any = { toast: (text: string, _ms: number, options: any) => toasts.push([text, options]), dismissToast: (key: string) => dismissed.push(key) };
  const stop = trackExportTasks(PM, queue);
  return { queue, toasts, dismissed, stop, change: (state: RenderQueueState) => listener(state), list: (state: RenderQueueState) => listed(state) };
}

describe('export tasks', () => {
  it('shows a rendering queue job as a task with its progress and Cancel', () => {
    const f = fixture();
    f.change({ running: true, jobs: [job('queued')] });
    expect(f.toasts.at(-1)).toEqual(['Exporting Film.mp4', expect.objectContaining({ key: 'export-job-q1', task: true, progress: null, detail: 'Starting' })]);
    f.change({ running: true, jobs: [job('running', { progress: 0.42 })] });
    const [title, options] = f.toasts.at(-1)!;
    expect(title).toBe('Exporting Film.mp4');
    expect(options).toMatchObject({ key: 'export-job-q1', task: true, progress: 0.42, detail: 'H.264 · 1920×1080' });
    options.action.run();
    expect(f.queue.queueAction).toHaveBeenCalledWith({ kind: 'cancel', id: 'q1' });
  });

  it('announces a finished job once, with Show in Finder', () => {
    const f = fixture();
    f.change({ running: true, jobs: [job('running', { progress: 0.9 })] });
    f.change({ running: false, jobs: [job('done', { progress: 1, path: '/x/Film.mp4' })] });
    expect(f.dismissed).toEqual(['export-job-q1']);
    const [title, options] = f.toasts.at(-1)!;
    expect(title).toBe('Exported Film.mp4');
    expect(options).toMatchObject({ error: false, action: { label: 'Show in Finder' } });
    options.action.run();
    expect(f.queue.queueAction).toHaveBeenCalledWith({ kind: 'reveal', id: 'q1' });
    const count = f.toasts.length;
    f.change({ running: false, jobs: [job('done', { progress: 1, path: '/x/Film.mp4' })] });
    expect(f.toasts).toHaveLength(count);
  });

  it('says why a job failed, and stays quiet about a cancelled one', () => {
    const f = fixture();
    f.change({ running: true, jobs: [job('running'), job('running', { id: 'a1', source: 'agent', name: 'Agent.mp4' })] });
    f.change({ running: false, jobs: [job('failed', { error: 'The disk is full.' }), job('cancelled', { id: 'a1', source: 'agent', name: 'Agent.mp4' })] });
    expect(f.toasts.at(-1)).toEqual(['Export failed · Film.mp4: The disk is full.', expect.objectContaining({ error: true })]);
    expect(f.toasts.filter(([text]) => text.startsWith('Exported') || text.includes('Agent.mp4') && !text.startsWith('Exporting'))).toEqual([]);
    expect(f.dismissed.sort()).toEqual(['export-job-a1', 'export-job-q1']);
  });

  it('never announces jobs that ended before this window watched them', async () => {
    const f = fixture();
    f.list({ running: false, jobs: [job('done', { path: '/x/Film.mp4' }), job('failed', { id: 'q2' })] });
    await Promise.resolve();
    expect(f.toasts).toEqual([]);
  });

  it('ignores a first listing older than a change already received', async () => {
    const f = fixture();
    f.change({ running: false, jobs: [job('done', { path: '/x/Film.mp4' })] });
    f.list({ running: true, jobs: [job('running')] });
    await Promise.resolve();
    expect(f.toasts).toEqual([]);
  });
});
