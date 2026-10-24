import type { PMRegistry } from '../registry';

export type ImportProgress = { label: string; completed?: number; total?: number };
let nextImport = 0;

/** Imports share the same persistent status surface and completion as Save. */
export function createImportProgress(title: string, PM: PMRegistry) {
  const key = `import-${++nextImport}`;
  let finished = false;
  let closed = false;
  let scheduled: number | null = null;
  let latest: number | null = null;
  const show = (progress: number | null) => PM.toast(title, 2200, {
    key, icon: 'plus', error: false, sticky: true, dismissible: false, progress,
  });
  const cancelUpdate = () => {
    if (scheduled !== null) cancelAnimationFrame(scheduled);
    scheduled = null;
  };
  show(null);
  return {
    update({ completed, total }: ImportProgress) {
      if (finished || closed) return;
      latest = completed != null && total != null && total > 0 ? Math.min(.95, Math.max(0, completed / total)) : null;
      if (scheduled === null) scheduled = requestAnimationFrame(() => {
        scheduled = null;
        if (!finished && !closed) show(latest);
      });
    },
    finish(message: string, milliseconds = 3400) {
      if (finished || closed) return;
      finished = true;
      cancelUpdate();
      PM.toast(message, milliseconds, { key, icon: 'plus', error: false, progress: 1, completed: true });
    },
    close() {
      cancelUpdate();
      if (!closed && !finished) PM.dismissToast?.(key);
      closed = true;
    },
  };
}
