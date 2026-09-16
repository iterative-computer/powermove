/* The app updates itself in the background (see main/updates.ts). Once a new
   version is staged this module surfaces it in two quiet places: a block in
   the Projects home sidebar, and a bottom-right toast over the editor that
   stays until the user acts on it or dismisses it. Dismissal is remembered
   per version, so a dismissed toast does not come back for the same update;
   the home block always shows while an update is ready. */
import type { AppUpdateState } from '../../../shared/ipc';
import type { PowermoveAPI } from '../kernel/api';
import type { PMRegistry } from '../legacy/registry';

type Listener = (state: AppUpdateState | null) => void;

const TOAST_KEY = 'app-update';
const DISMISSED = 'dismissed-version';

let api: PowermoveAPI | null = null;
let registry: PMRegistry | null = null;
let state: AppUpdateState | null = null;
let toastShown = false;
const listeners = new Set<Listener>();

function ready(): boolean {
  return state?.status === 'ready';
}

function message(): string {
  return state?.version ? `Powermove ${state.version} is ready to install` : 'A Powermove update is ready to install';
}

function homeOpen(): boolean {
  return !!(registry?.ProjectsScreen as { isOpen?: boolean } | undefined)?.isOpen;
}

function dismissedForThisVersion(): boolean {
  if (!api || !state) return false;
  return api.storage.get<string>(DISMISSED) === (state.version ?? 'unknown');
}

function syncToast(): void {
  if (!api) return;
  const wanted = ready() && !homeOpen() && !dismissedForThisVersion();
  if (wanted) {
    api.ui.toast(message(), {
      sticky: true,
      dismissible: true,
      icon: 'sparkle',
      key: TOAST_KEY,
      corner: 'bottom-right',
      action: { label: 'Restart to update', run: installUpdate },
      onDismiss: dismissToast
    });
    toastShown = true;
  } else if (toastShown) {
    (registry?.dismissToast as ((key: string) => void) | undefined)?.(TOAST_KEY);
    toastShown = false;
  }
}

function apply(next: AppUpdateState | null): void {
  const changed = next?.status !== state?.status || next?.version !== state?.version;
  state = next;
  if (changed) for (const listener of listeners) listener(state);
  syncToast();
}

/** Quit normally, install the staged version, relaunch. */
export function installUpdate(): void {
  void window.powermove?.updates?.install();
}

/** Hide the toast for this version; the home sidebar still shows the update. */
export function dismissToast(): void {
  if (!api || !state) return;
  api.storage.set(DISMISSED, state.version ?? 'unknown');
  syncToast();
}

export function appUpdateState(): AppUpdateState | null {
  return state;
}

export function subscribeAppUpdates(listener: Listener): () => void {
  listeners.add(listener);
  listener(state);
  return () => { listeners.delete(listener); };
}

export function installAppUpdates(PM: PMRegistry): () => void {
  const kernel = PM.Kernel as { api?: (id: string) => PowermoveAPI } | undefined;
  const bridge = window.powermove?.updates;
  if (typeof kernel?.api !== 'function' || !bridge) return () => {};
  api = kernel.api('app-updates');
  registry = PM;
  const offChanged = bridge.onChanged(apply);
  void bridge.status().then(apply).catch(() => undefined);
  const offScreen = (PM.bus as { on?: (event: string, fn: () => void) => (() => void) | undefined } | undefined)?.on?.('projects:screen', syncToast);
  return () => {
    offChanged();
    offScreen?.();
    registry = null;
    api = null;
    state = null;
    toastShown = false;
    listeners.clear();
  };
}
