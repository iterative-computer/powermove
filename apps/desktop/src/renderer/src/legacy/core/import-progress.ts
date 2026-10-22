import type { PMRegistry } from '../registry';

export type ImportProgress = { label?: string; file?: string; completed?: number; total?: number };
let nextImport = 0;

/** Imports share the same persistent status surface and completion as Save. */
export function createImportProgress(title: string, PM: PMRegistry, cancel?: () => void) {
  const key = `import-${++nextImport}`;
  let finished = false;
  let cancelling = false;
  const show = (progress: number | null, label = title, count?: { done: number; total: number }) => PM.toast(label, 2200, {
    key, icon: 'plus', error: false, sticky: true, dismissible: false, progress, task: true, count,
    ...(cancel && !cancelling ? { action: { label: 'Cancel', dismiss: false, run: () => {
      if (finished || cancelling) return;
      cancelling = true;
      cancel();
      show(null, 'Cancelling import…');
    } } } : {}),
  });
  show(null);
  return {
    update({ label, file, completed, total }: ImportProgress) {
      if (!finished && !cancelling) show(completed != null && total != null && total > 0 ? Math.min(1, Math.max(0, completed / total)) : null, label ?? (file ? `${title} · ${file}` : title), completed != null && total != null ? { done: completed, total } : undefined);
    },
    finish(message: string, milliseconds = 3400) {
      finished = true;
      PM.toast(message, milliseconds, { key, icon: 'plus', error: false, progress: 1, completed: true, task: true });
    },
    close() { if (!finished) PM.dismissToast?.(key); },
  };
}
