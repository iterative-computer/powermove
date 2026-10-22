/*
 * The Render Queue panel (plan section 17, "Presets and the render queue"),
 * with Use New Engine on: jobs from the export sheet and agents, each with its
 * destination, rendering unattended in the job engine from snapshots; a failed
 * job shows why and the next one runs; user jobs survive relaunch.
 *
 * It is a Settings sheet: a card of rows, each a job's name over its summary
 * and destination folder, its state on the right (a thin progress bar while it
 * renders), and a button menu for the job (controls/popover-menu.ts). Lists
 * stay quiet text: no glyphs, counts, or thumbnails.
 */
import { openPopoverMenu, type PopoverMenuItem } from '../../controls/popover-menu';
import type { RenderQueueJob, RenderQueueState } from '../../../../shared/engine-ipc';
import { bridge as hostBridge } from '../../kernel/bridge';
import { createSettingsSection } from '../ui/settings-section';

type QueueBridge = {
  queueList(): Promise<RenderQueueState>;
  queueAction(action: import('../../../../shared/engine-ipc').RenderQueueAction): Promise<RenderQueueState>;
  onQueueChanged(listener: (state: RenderQueueState) => void): () => void;
};

function queueBridge(): QueueBridge | null {
  const engine = (hostBridge() as { nativeEngine?: Partial<QueueBridge> } | undefined)?.nativeEngine;
  return engine?.queueList && engine.queueAction && engine.onQueueChanged ? engine as QueueBridge : null;
}

/** The job's state as one line ("Rendering · 42%", "Waiting for media · 2
 *  clips offline", "Failed · The disk is full."). */
export function stateLine(job: RenderQueueJob): string {
  switch (job.state) {
    case 'pending': return 'Waiting to start';
    case 'queued': return 'Starting';
    case 'paused': return `Paused${job.error ? ` · ${job.error.replace(/\.$/, '')}` : ''}`;
    case 'running': return `Rendering · ${Math.floor(job.progress * 100)}%`;
    case 'waitingForMedia': return `Waiting for media${job.error ? ` · ${job.error.replace(/\.$/, '')}` : ''}`;
    case 'done': return 'Done';
    case 'cancelled': return 'Cancelled';
    case 'failed': return `Failed${job.error ? ` · ${job.error}` : ''}`;
  }
}

/** The folder a job writes into, with the home folder as `~` (`~/Movies`). */
export function folderOf(destination: string): string {
  const folder = destination.replace(/\/[^/]*\/?$/, '') || '/';
  return folder.replace(/^\/Users\/[^/]+(?=\/|$)/, '~');
}

/** The job menu's items for a job in the queue. */
export function jobMenu(job: RenderQueueJob, index: number, users: number, act: (action: import('../../../../shared/engine-ipc').RenderQueueAction) => void): PopoverMenuItem[] {
  const items: PopoverMenuItem[] = [];
  const live = job.state === 'running' || job.state === 'queued' || job.state === 'waitingForMedia';
  if (live || job.state === 'pending') items.push({ label: 'Cancel', run: () => act({ kind: 'cancel', id: job.id }) });
  if (job.source === 'user' && (job.state === 'failed' || job.state === 'cancelled')) items.push({ label: 'Render Again', run: () => act({ kind: 'retry', id: job.id }) });
  if (job.path) items.push({ label: 'Show in Finder', run: () => act({ kind: 'reveal', id: job.id }) });
  if (job.source === 'user' && job.state === 'pending') {
    if (index > 0) items.push({ label: 'Move Up', run: () => act({ kind: 'move', id: job.id, to: index - 1 }) });
    if (index < users - 1) items.push({ label: 'Move Down', run: () => act({ kind: 'move', id: job.id, to: index + 1 }) });
  }
  if (!live) {
    if (items.length) items.push('-');
    items.push({ label: 'Remove', run: () => act({ kind: 'remove', id: job.id }) });
  }
  return items;
}

export function openRenderQueuePanel(PM: any): any {
  const queue = queueBridge();
  if (!queue) return null;
  const h = PM.h;
  const body: HTMLElement = h('div.export-form.sg-column.render-queue');
  const jobsSection = createSettingsSection('Jobs');
  const empty: HTMLElement = h('p.settings-note', 'Add exports from the Export sheet with Add to Queue. Agent exports appear here while they render.');
  body.append(jobsSection.element, empty);
  let state: RenderQueueState = { running: false, jobs: [] };
  let modal: any = null;
  const act = (action: import('../../../../shared/engine-ipc').RenderQueueAction): void => {
    void queue.queueAction(action).then(draw, (error: unknown) => PM.toast(`Render queue: ${error instanceof Error ? error.message : String(error)}`, 4000));
  };

  function row(job: RenderQueueJob, index: number, users: number): HTMLElement {
    const element: HTMLElement = h('div.settings-row.render-queue-job', { 'data-job': job.id, 'data-state': job.state });
    const copy: HTMLElement = h('div.settings-copy',
      h('b', job.name),
      h('span', [job.summary, job.destination ? folderOf(job.destination) : job.source === 'agent' ? 'Agent export' : ''].filter(Boolean).join(' · ')));
    const status: HTMLElement = h('div.render-queue-state');
    const line: HTMLElement = h('span.render-queue-line', stateLine(job));
    status.append(line);
    if (job.state === 'running') {
      const bar: HTMLElement = h('div.bar', { role: 'progressbar', 'aria-label': `${job.name} progress`, 'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': String(Math.round(job.progress * 100)) },
        h('i', { style: { width: `${(job.progress * 100).toFixed(1)}%` } }));
      status.append(bar);
    }
    const more: HTMLButtonElement = h('button.btn.render-queue-more', { type: 'button', 'aria-label': `${job.name} actions`, 'aria-haspopup': 'menu' }, 'Actions');
    more.onclick = () => openPopoverMenu({ anchor: more, label: `${job.name} actions`, align: 'end', items: jobMenu(job, index, users, act) });
    element.append(copy, status, more);
    return element;
  }

  function draw(next: RenderQueueState): void {
    state = next;
    const users = state.jobs.filter((j) => j.source === 'user').length;
    let userIndex = 0;
    jobsSection.body.replaceChildren(...state.jobs.map((job) => row(job, job.source === 'user' ? userIndex++ : 0, users)));
    jobsSection.element.hidden = !state.jobs.length;
    empty.hidden = !!state.jobs.length;
    const pending = state.jobs.some((j) => j.source === 'user' && j.state === 'pending');
    const done = state.jobs.filter((j) => j.state === 'done').length;
    jobsSection.summary.textContent = state.running ? 'Rendering' : pending ? 'Paused' : done ? 'Finished' : '';
    if (modal?.el) {
      const buttons = [...modal.el.querySelectorAll('.mf button')] as HTMLButtonElement[];
      const primary = buttons.find((b) => b.classList.contains('pri'));
      if (primary) {
        primary.textContent = state.running ? 'Pause Queue' : 'Start Queue';
        primary.disabled = !state.running && !pending;
      }
      const clear = buttons.find((b) => b.textContent === 'Clear Finished');
      if (clear) clear.disabled = !state.jobs.some((j) => j.state === 'done' || j.state === 'cancelled');
    }
  }

  const off = queue.onQueueChanged(draw);
  modal = PM.modal({
    title: 'Render Queue',
    body,
    width: 620,
    onClose: () => off(),
    actions: [
      { label: 'Close' },
      { label: 'Clear Finished', run: () => { act({ kind: 'clearFinished' }); return false; } },
      { label: 'Start Queue', pri: true, run: () => { act({ kind: state.running ? 'pause' : 'start' }); return false; } }
    ]
  });
  modal.el.classList.add('export-modal', 'render-queue-modal');
  void queue.queueList().then(draw);
  draw(state);
  return modal;
}
