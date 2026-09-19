import type { PMRegistry } from '../registry';

export type ImportProgress = { label: string; completed?: number; total?: number };
let nextImport = 0;

/** Imports share the same persistent status surface and completion as Save. */
export function createImportProgress(title: string, PM: PMRegistry) {
  const key = `import-${++nextImport}`;
  let finished = false;
  const show = (progress: number | null) => PM.toast(title, 2200, {
    key, icon: 'plus', error: false, sticky: true, dismissible: false, progress,
  });
  show(null);
  return {
    update({ completed, total }: ImportProgress) {
      if (!finished) show(completed != null && total != null && total > 0 ? Math.min(.95, Math.max(0, completed / total)) : null);
    },
    finish(message: string, milliseconds = 3400) {
      finished = true;
      PM.toast(message, milliseconds, { key, icon: 'plus', error: false, progress: 1, completed: true });
    },
    close() { if (!finished) PM.dismissToast?.(key); },
  };
}
