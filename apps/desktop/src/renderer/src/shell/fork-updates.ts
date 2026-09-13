/* Forked built-ins fall behind when the app ships a newer version of the
   extension they were copied from. The registry marks such records with
   `update` (see ExtensionRecord). This module turns that into one quiet
   notice (top-right toast) plus a home-screen sidebar block, and hands the
   actual merge to the user's agent through `api.extensions.rebase`. Dismissal
   is remembered per fork and target version, so a dismissed notice does not
   return until the next shipped version. */
import type { ExtensionRecord, PowermoveAPI } from '../kernel/api';
import type { PMRegistry } from '../legacy/registry';

export type ForkUpdate = { id: string; name: string; base: string; current: string };
type Listener = (pending: readonly ForkUpdate[]) => void;

type RecordWithUpdate = ExtensionRecord & { update?: { forkedFrom: string; base: string; current: string } };

const TOAST_KEY = 'fork-updates';
const DISMISSED = 'dismissed';

let api: PowermoveAPI | null = null;
let registry: PMRegistry | null = null;
let pending: ForkUpdate[] = [];
let toastShown = false;
const listeners = new Set<Listener>();

const signature = (update: ForkUpdate): string => `${update.id}@${update.current}`;

function dismissed(): Set<string> {
  const saved = api?.storage.get<string[]>(DISMISSED);
  return new Set(Array.isArray(saved) ? saved : []);
}

function collect(): ForkUpdate[] {
  if (!api) return [];
  const hidden = dismissed();
  return (api.extensions.list() as RecordWithUpdate[])
    .filter((record) => record.update && record.enabled)
    .map((record) => ({
      id: record.id,
      name: record.manifest?.name ?? record.id,
      base: record.update!.base,
      current: record.update!.current
    }))
    .filter((update) => !hidden.has(signature(update)));
}

function notify(): void {
  for (const listener of listeners) listener(pending);
}

function message(count: number): string {
  return count === 1
    ? '1 extension needs updating'
    : `${count} extensions need updating`;
}

/* The Projects home screen carries the notice in its sidebar, so the corner
   toast only shows over the editor; a toast there would cover New Project. */
function homeOpen(): boolean {
  return !!(registry?.ProjectsScreen as { isOpen?: boolean } | undefined)?.isOpen;
}

function syncToast(): void {
  if (!api) return;
  const wanted = pending.length > 0 && !homeOpen();
  if (wanted) {
    api.ui.toast(message(pending.length), {
      sticky: true,
      dismissible: true,
      icon: 'sparkle',
      key: TOAST_KEY,
      corner: 'top-right',
      action: { label: 'Update', run: updateAll },
      onDismiss: dismissAll
    });
    toastShown = true;
  } else if (toastShown) {
    (registry?.dismissToast as ((key: string) => void) | undefined)?.(TOAST_KEY);
    toastShown = false;
  }
}

function refresh(): void {
  const next = collect();
  const changed = next.length !== pending.length || next.some((update, i) => signature(update) !== signature(pending[i]!));
  pending = next;
  if (changed) notify();
  syncToast();
}

/** Hand every pending fork to the agent, one prompt per fork. */
export function updateAll(): void {
  if (!api) return;
  for (const update of pending) api.extensions.rebase(update.id);
}

/** Hide the current notices until a newer version ships. */
export function dismissAll(): void {
  if (!api) return;
  const hidden = dismissed();
  for (const update of pending) hidden.add(signature(update));
  api.storage.set(DISMISSED, [...hidden]);
  refresh();
}

export function pendingForkUpdates(): readonly ForkUpdate[] {
  return pending;
}

export function subscribeForkUpdates(listener: Listener): () => void {
  listeners.add(listener);
  listener(pending);
  return () => { listeners.delete(listener); };
}

export function installForkUpdates(PM: PMRegistry): () => void {
  const kernel = PM.Kernel as { api?: (id: string) => PowermoveAPI } | undefined;
  if (typeof kernel?.api !== 'function') return () => {};
  api = kernel.api('fork-updates');
  registry = PM;
  const subscriptions = [
    api.events.on('extensions:changed', refresh),
    api.events.on('extension:loaded', refresh)
  ];
  const offScreen = (PM.bus as { on?: (event: string, fn: () => void) => (() => void) | undefined } | undefined)?.on?.('projects:screen', syncToast);
  refresh();
  return () => {
    for (const subscription of subscriptions) subscription.dispose();
    offScreen?.();
    registry = null;
    api = null;
    pending = [];
    listeners.clear();
  };
}
