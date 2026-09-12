const OVERLAY_HOST_IDS = [
  'app',
  'projects-screen',
  'library-overlay',
  'library-screen',
  'workspace-editbar'
] as const;

export type InertSnapshot = Array<{ element: HTMLElement; inert: boolean }>;

export function inertOverlayHosts(): InertSnapshot {
  const snapshot: InertSnapshot = [];
  for (const id of OVERLAY_HOST_IDS) {
    const element = document.getElementById(id);
    if (!element) continue;
    snapshot.push({ element, inert: element.inert });
    element.inert = true;
  }
  return snapshot;
}

export function restoreOverlayHosts(snapshot: InertSnapshot): void {
  for (const { element, inert } of snapshot) {
    if (element.isConnected) element.inert = inert;
  }
}
