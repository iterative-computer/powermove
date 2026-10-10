import type { ForkUpdateResult } from '../../../shared/ipc';
import { bridge } from '../kernel/bridge';
import type { PMRegistry } from '../legacy/registry';
import { checkInSandbox } from '../store/sandbox-check';

/** Serialize the home screen's Update action and ignore repeated clicks.
 * Agent work starts only from the explicit action on an unresolved update. */
export function createForkUpdater(PM: PMRegistry, startAgent: (id: string) => Promise<void>): (id: string) => Promise<void> {
  let queue = Promise.resolve();
  const busy = new Set<string>();
  const offerAgent = (id: string, name: string, message: string) => PM.toast(message, 6000, {
    key: `fork-update:${id}`, corner: 'top-right', sticky: true, dismissible: true,
    kind: 'alert', source: { id, name },
    action: { label: 'Update with agent', run: () => {
      PM.dismissToast?.(`fork-update:${id}`);
      void startAgent(id).catch(error => PM.toast(String(error?.message ?? 'Could not start the extension update.')));
    } }
  });
  const reload = async (id: string) => {
    await PM.Kernel?.loader?.reload?.(id);
    await PM.Kernel?.loader?.whenIdle?.();
  };
  return (id: string) => {
    if (busy.has(id)) return queue;
    busy.add(id);
    const run = queue.then(async () => {
      const native = bridge()?.codex;
      const name = PM.Kernel?.loader?.records?.().find((record: { id: string }) => record.id === id)?.manifest?.name ?? id;
      if (!native?.updateFork) {
        PM.toast('Restart Powermove to update this extension.');
        return;
      }
      const key = `fork-update:${id}`;
      let applied: Extract<ForkUpdateResult, { kind: 'updated' }> | undefined;
      let restoring: Promise<void> | undefined;
      const restore = (): Promise<void> => {
        if (!applied) return Promise.resolve();
        const previous = applied;
        return restoring ??= (async () => {
          await native.restoreChangeSet({ projectId: previous.projectId, changeSetId: previous.changeSetId });
          await reload(id);
        })().catch(error => { restoring = undefined; throw error; });
      };
      PM.toast(`Updating ${name}…`, 6000, { key, corner: 'top-right', sticky: true });
      try {
        const result = await native.updateFork({ id });
        PM.dismissToast?.(key);
        if (result.kind === 'needs-agent') {
          offerAgent(id, name, result.reason === 'conflict'
            ? `${name} needs help combining this update with your customizations. Your current version was kept.`
            : `${name} couldn’t pass the update checks. Your current version was kept.`);
          return;
        }
        applied = result;
        await reload(id);
        const record = PM.Kernel?.loader?.records?.().find((record: { id: string }) => record.id === id);
        if (!record || ['build-error', 'manifest-error', 'activation-error', 'runtime-error', 'needs-update'].includes(record.health.state)) {
          throw new Error('The updated extension could not load.');
        }
        if (PM.Kernel?.deps) {
          const report = await checkInSandbox(PM as any, id);
          if (!report.skipped && !report.ok) throw new Error('The updated extension did not pass its sandbox check.');
        }
        PM.toast(`${name} is updated. Your customizations were kept.`, 6000, {
          key, corner: 'top-right', action: { label: 'Undo', run: () => {
            void restore().then(() => {
              PM.dismissToast?.(key);
              PM.toast(`${name} update undone.`);
            }).catch(error => PM.toast(String(error?.message ?? 'Could not undo the extension update.')));
          } }
        });
      } catch (error) {
        PM.dismissToast?.(key);
        if (applied) {
          try { await restore(); }
          catch {
            PM.toast(`${name} couldn’t be checked or restored. Review it in Extensions.`, 6000, { kind: 'alert' });
            return;
          }
          offerAgent(id, name, `${name} couldn’t pass the update checks. Your previous version was restored.`);
        } else PM.toast(String((error as Error)?.message ?? 'Could not update the extension.'), 6000, { kind: 'alert' });
      }
    }).finally(() => { busy.delete(id); });
    queue = run.catch(() => undefined);
    return run;
  };
}
