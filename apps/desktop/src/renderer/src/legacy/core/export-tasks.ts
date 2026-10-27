/*
 * Exports that render without the export sheet, with Use New Engine on: the
 * render queue's jobs and agents' export_video, both in the job engine. While
 * one renders it is a task in the titlebar's task pill (percent, time left,
 * Cancel); when it ends, a notice says so (Show in Finder) or says why it
 * failed. The Render Queue panel stays the place to manage them.
 */
import type { RenderQueueAction, RenderQueueJob, RenderQueueState } from '../../../../shared/engine-ipc';
import { stateLine } from './render-queue-panel';

export type ExportTaskQueue = {
  queueList(): Promise<RenderQueueState>;
  queueAction(action: RenderQueueAction): Promise<RenderQueueState>;
  onQueueChanged(listener: (state: RenderQueueState) => void): () => void;
};

const RENDERING = new Set<RenderQueueJob['state']>(['queued', 'running', 'waitingForMedia']);

/** Mirrors the queue's rendering jobs as tasks; returns an unsubscribe. */
export function trackExportTasks(PM: any, queue: ExportTaskQueue): () => void {
  /** Jobs shown as tasks, by id. Only these announce how they ended. */
  const shown = new Set<string>();
  const key = (id: string): string => `export-job-${id}`;
  const act = (action: RenderQueueAction): void => {
    void queue.queueAction(action).catch((error: unknown) => PM.toast(`Render queue: ${error instanceof Error ? error.message : String(error)}`, 4000, { error: true }));
  };

  const ended = (job: RenderQueueJob): void => {
    shown.delete(job.id);
    PM.dismissToast?.(key(job.id));
    if (job.state === 'done') {
      PM.toast(`Exported ${job.name}`, 8000, {
        key: `export-result-${job.id}`, error: false,
        ...(job.path ? { action: { label: 'Show in Finder', run: () => act({ kind: 'reveal', id: job.id }) } } : {})
      });
    } else if (job.state === 'failed') {
      PM.toast(`Export failed · ${job.name}${job.error ? `: ${job.error}` : ''}`, 5000, { key: `export-result-${job.id}`, error: true });
    }
  };

  const draw = (state: RenderQueueState): void => {
    const listed = new Set<string>();
    for (const job of state.jobs) {
      listed.add(job.id);
      if (RENDERING.has(job.state)) {
        shown.add(job.id);
        PM.toast(`Exporting ${job.name}`, 0, {
          key: key(job.id), task: true, error: false, sticky: true, dismissible: false,
          progress: job.state === 'running' ? Math.max(0, Math.min(1, job.progress)) : null,
          detail: job.state === 'running' ? job.summary : stateLine(job),
          action: { label: 'Cancel', dismiss: false, run: () => act({ kind: 'cancel', id: job.id }) }
        });
      } else if (shown.has(job.id)) ended(job);
    }
    // Removed from the list while it rendered (another window cleared it).
    for (const id of [...shown]) if (!listed.has(id)) { shown.delete(id); PM.dismissToast?.(key(id)); }
  };

  // The first listing is skipped once a change has arrived: it may be older.
  let changed = false;
  const off = queue.onQueueChanged((state) => { changed = true; draw(state); });
  void queue.queueList().then((state) => { if (!changed) draw(state); }, () => undefined);
  return () => {
    off();
    for (const id of shown) PM.dismissToast?.(key(id));
    shown.clear();
  };
}
