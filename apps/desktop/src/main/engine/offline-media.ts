/*
 * Offline media at export (plan section 14.8, appendix D "Offline media and
 * Blender at export"): the clips an export of a composition needs, and which
 * of them the media store cannot hand the engine. Interactive exports ask the
 * native three-button message before anything starts; queued jobs wait in the
 * job engine ("Waiting for media · 2 clips offline"), which runs the same
 * check (`pm_export::render::offline_media`).
 */
import { dialog, type BrowserWindow } from 'electron';
import type { OfflineClip } from '../../shared/offline-clips';
export { offlineClips, referencedClips, type OfflineClip } from '../../shared/offline-clips';

type Project = Record<string, any>;

export type OfflineChoice = 'locate' | 'proxies' | 'without' | 'cancel';

/** The explicit no-media choice overrides a draft proxy preference. */
export function offlineExportOptions<T extends { draft?: boolean }>(options: T, choice: 'proxies' | 'without'): T {
  return { ...options, draft: choice === 'proxies' };
}

/** "2 clips are offline". */
export function offlineTitle(count: number): string {
  return count === 1 ? '1 clip is offline' : `${count} clips are offline`;
}

/** The native message's options. "Export with Proxies" appears only when every
 *  offline clip has a proxy to draft from. */
export function offlineMessage(clips: OfflineClip[], proxies: boolean): Electron.MessageBoxOptions {
  const names = clips.slice(0, 5).map((c) => c.name).join(', ') + (clips.length > 5 ? ` and ${clips.length - 5} more` : '');
  return {
    type: 'warning',
    message: offlineTitle(clips.length),
    detail: proxies
      ? `Their original files can’t be found. Locate them, export from proxies at lower resolution, or export without the missing clips.\n\n${names}`
      : `Their original files can’t be found. Locate them or export without the missing clips.\n\n${names}`,
    buttons: proxies ? ['Locate Media…', 'Export with Proxies', 'Export Without Missing Media', 'Cancel'] : ['Locate Media…', 'Export Without Missing Media', 'Cancel'],
    defaultId: 0,
    cancelId: proxies ? 3 : 2,
    noLink: true
  };
}

/** Shows the message as a sheet on `window`. */
export async function askAboutOfflineMedia(
  window: BrowserWindow,
  clips: OfflineClip[],
  proxies: boolean,
  show: (window: BrowserWindow, options: Electron.MessageBoxOptions) => Promise<Electron.MessageBoxReturnValue> = (w, o) => dialog.showMessageBox(w, o)
): Promise<OfflineChoice> {
  const options = offlineMessage(clips, proxies);
  const { response } = await show(window, options);
  if (response === 0) return 'locate';
  if (proxies && response === 1) return 'proxies';
  if (response === (proxies ? 2 : 1)) return 'without';
  return 'cancel';
}

/** "This composition renders with Blender…" (appendix D) when a composition
 *  needs Blender and none is installed. */
export const BLENDER_REQUIRED = {
  message: 'This composition renders with Blender',
  detail: 'Install Blender 4.5 or later, or set its renderer to Native.'
} as const;

/** Whether any layer the composition reaches renders with Blender
 *  (`render3d.enabled` on the composition or a nested one). */
export function needsBlender(project: Project, composition: string = project['compId']): boolean {
  const comps: Record<string, any> = project['comps'] && typeof project['comps'] === 'object' ? project['comps'] : {};
  const open = typeof project['compId'] === 'string' ? project['compId'] : project['id'];
  const compOf = (id: string): any => (id === open ? project : comps[id]);
  const stack = [composition];
  const visited = new Set<string>();
  while (stack.length) {
    const id = stack.pop()!;
    if (visited.has(id)) continue;
    visited.add(id);
    const comp = compOf(id);
    if (!comp) continue;
    if (comp.render3d && typeof comp.render3d === 'object' && comp.render3d.enabled === true) return true;
    for (const layer of Array.isArray(comp.layers) ? comp.layers : []) {
      if (layer?.type === 'precomp' && layer.d?.comp) stack.push(String(layer.d.comp));
    }
  }
  return false;
}
