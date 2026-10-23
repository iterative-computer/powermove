/*
 * Offline media at export (plan section 14.8, appendix D "Offline media and
 * Blender at export"): the clips an export of a composition needs, and which
 * of them the media store cannot hand the engine. Interactive exports ask the
 * native three-button message before anything starts; queued jobs wait in the
 * job engine ("Waiting for media · 2 clips offline"), which runs the same
 * check (`pm_export::render::offline_media`).
 */
import { dialog, type BrowserWindow } from 'electron';

export type OfflineClip = { asset: string; name: string };

type Project = Record<string, any>;

/** Video, image, and audio assets the open composition (and every composition
 *  it nests) uses, in first-use order. Layers whose asset has no record draw
 *  nothing, as today, and are skipped. */
export function referencedClips(project: Project, composition: string = project['compId']): OfflineClip[] {
  const assets: Record<string, any> = project['assets'] && typeof project['assets'] === 'object' ? project['assets'] : {};
  const comps: Record<string, any> = project['comps'] && typeof project['comps'] === 'object' ? project['comps'] : {};
  const open = typeof project['compId'] === 'string' ? project['compId'] : project['id'];
  const layersOf = (id: string): any[] | null => {
    if (id === open && Array.isArray(project['layers'])) return project['layers'];
    const comp = comps[id];
    return comp && Array.isArray(comp.layers) ? comp.layers : null;
  };
  const out: OfflineClip[] = [];
  const seen = new Set<string>();
  const stack = [composition];
  const visited = new Set<string>();
  while (stack.length) {
    const id = stack.pop()!;
    if (visited.has(id)) continue;
    visited.add(id);
    for (const layer of layersOf(id) ?? []) {
      if (!layer || typeof layer !== 'object') continue;
      const d = layer.d && typeof layer.d === 'object' ? layer.d : {};
      if (layer.type === 'precomp' && d.comp) {
        const target = String(d.comp);
        if (!(target === open && Array.isArray(project['layers']))) stack.push(target);
      } else if ((layer.type === 'video' || layer.type === 'image' || layer.type === 'audio') && d.asset != null) {
        const asset = String(d.asset);
        const record = assets[asset];
        if (!record || seen.has(asset)) continue;
        seen.add(asset);
        out.push({ asset, name: typeof record.name === 'string' && record.name ? record.name : asset });
      }
    }
  }
  return out;
}

/** The referenced clips whose files the media store cannot resolve. */
export async function offlineClips(project: Project, resolve: (asset: string) => Promise<unknown>): Promise<OfflineClip[]> {
  const clips = referencedClips(project);
  const results = await Promise.all(clips.map(async (clip) => ({ clip, ok: await resolve(clip.asset).catch(() => null) })));
  return results.filter((r) => !r.ok).map((r) => r.clip);
}

export type OfflineChoice = 'locate' | 'proxies' | 'cancel';

/** "2 clips are offline". */
export function offlineTitle(count: number): string {
  return count === 1 ? '1 clip is offline' : `${count} clips are offline`;
}

/** The native message's options. "Export with Proxies" appears only when every
 *  offline clip has a proxy to draft from. Buttons follow the HIG: the
 *  default action on the right (index 0), Cancel on the left. */
export function offlineMessage(clips: OfflineClip[], proxies: boolean): Electron.MessageBoxOptions {
  const names = clips.slice(0, 5).map((c) => c.name).join(', ') + (clips.length > 5 ? ` and ${clips.length - 5} more` : '');
  return {
    type: 'warning',
    message: offlineTitle(clips.length),
    detail: proxies
      ? `Their original files can’t be found. Locate them to export at full quality, or export these clips from their proxies at lower resolution.\n\n${names}`
      : `Their original files can’t be found. Locate them to export.\n\n${names}`,
    buttons: proxies ? ['Locate Media…', 'Export with Proxies', 'Cancel'] : ['Locate Media…', 'Cancel'],
    defaultId: 0,
    cancelId: proxies ? 2 : 1,
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
  return 'cancel';
}

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
