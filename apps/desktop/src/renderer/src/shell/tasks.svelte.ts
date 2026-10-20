/* Long-running jobs (saving, importing) report here instead of stacking
   toasts. The titlebar pill and its popover read this list. */

export type TaskEntry = {
  key: string;
  title: string;
  /** 0–1, or null while the amount of work is not yet known. */
  progress: number | null;
  /** What is being worked on right now, such as a file name or a stage. */
  detail?: string;
  /** Items finished out of the total, for jobs that work through a list. */
  count?: { done: number; total: number };
  /** Stage position, for jobs that run in fixed steps. */
  step?: { index: number; total: number };
  action?: { label: string; run: () => void; dismiss?: boolean };
  completed: boolean;
  startedAt: number;
  finishedAt?: number;
};

export type TaskPatch = Partial<Omit<TaskEntry, 'key' | 'startedAt'>>;

export const taskState = $state<{ list: TaskEntry[] }>({ list: [] });

const DONE_VISIBLE_MS = 4000;
const removals = new Map<string, ReturnType<typeof setTimeout>>();

export function upsertTask(key: string, patch: TaskPatch & { title: string }): void {
  const clear = removals.get(key);
  if (clear) { clearTimeout(clear); removals.delete(key); }
  const existing = taskState.list.find(task => task.key === key);
  if (existing && !existing.completed) {
    Object.assign(existing, patch, patch.completed ? { finishedAt: Date.now() } : {});
  } else {
    const next: TaskEntry = { progress: null, completed: false, ...patch, key, startedAt: Date.now() };
    if (existing) taskState.list = taskState.list.filter(task => task !== existing);
    taskState.list.push(next);
  }
  if (patch.completed) {
    removals.set(key, setTimeout(() => removeTask(key), DONE_VISIBLE_MS));
  }
}

export function removeTask(key: string): void {
  const clear = removals.get(key);
  if (clear) { clearTimeout(clear); removals.delete(key); }
  const index = taskState.list.findIndex(task => task.key === key);
  if (index >= 0) taskState.list.splice(index, 1);
}

export function clearTasks(): void {
  for (const key of [...removals.keys()]) removeTask(key);
  taskState.list.splice(0, taskState.list.length);
}

/** Seconds left, or null until there is enough progress to estimate fairly. */
export function secondsLeft(task: TaskEntry, now: number): number | null {
  if (task.completed || task.progress == null || task.progress < 0.05) return null;
  const elapsed = now - task.startedAt;
  if (elapsed < 1500) return null;
  return Math.max(0, Math.round(elapsed * (1 - task.progress) / task.progress / 1000));
}

export function etaLabel(seconds: number | null): string {
  if (seconds == null) return '';
  if (seconds < 90) return `~${Math.max(1, seconds)} s`;
  return `~${Math.round(seconds / 60)} m`;
}

/** Overall progress of the jobs still running, or null if any is unmeasured. */
export function overallProgress(list: readonly TaskEntry[]): number | null {
  const running = list.filter(task => !task.completed);
  if (!running.length || running.some(task => task.progress == null)) return null;
  return running.reduce((sum, task) => sum + (task.progress as number), 0) / running.length;
}
