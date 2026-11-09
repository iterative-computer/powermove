import * as THREE from 'three';
import { compositionRuntime } from './service';
import { layer3DRole } from './layers';
import { isExtrudedLayer } from './extrusion-data';
import { frameView } from './navigation';
import { studioLayer } from '../studio';
import {
  applySidePose, applyViewportPose, getViewportMode, onViewportChange, setViewportMode, sideDirection, viewportKey,
  viewportPose, viewportSide, type ViewPose, type ViewSide
} from './viewport';

/* After Effects' view layouts: 1, 2 or 4 views of the same composition, each
   looking through the Active Camera, along an axis (Front, Left, Top, Back,
   Right, Bottom) or from a Custom View.

   The selected pane is the viewer itself: it owns the Free View / Active
   Camera state, gizmos, picking and Studio's refine. Selecting another pane
   hands that state over. The other panes are pictures from their own Studio
   session at Live quality; they re-render only when what they show changes. */

export type PaneView = 'camera' | ViewSide | 'custom';
export type ViewLayout = 1 | 2 | 4;
export type ViewPane = { view: PaneView; pose: ViewPose | null };
export type ViewsState = { layout: ViewLayout; active: number; panes: ViewPane[] };
export const PANE_VIEWS: ReadonlyArray<{ id: PaneView; label: string }> = [
  { id: 'camera', label: 'Active Camera' },
  { id: 'front', label: 'Front' }, { id: 'left', label: 'Left' }, { id: 'top', label: 'Top' },
  { id: 'back', label: 'Back' }, { id: 'right', label: 'Right' }, { id: 'bottom', label: 'Bottom' },
  { id: 'custom', label: 'Custom View' }
];
export const paneViewLabel = (view: PaneView): string => PANE_VIEWS.find(v => v.id === view)?.label ?? 'Custom View';

type Entry = { layout: ViewLayout; active: number; panes: ViewPane[] };
const entries = new WeakMap<object, Map<string, Entry>>();
const listeners = new WeakMap<object, Set<() => void>>();
function entry(PM: any): Entry {
  let map = entries.get(PM); if (!map) { map = new Map(); entries.set(PM, map); }
  const key = viewportKey(PM); let value = map.get(key);
  if (!value) { value = { layout: 1, active: 0, panes: [{ view: 'camera', pose: null }] }; map.set(key, value); }
  return value;
}
function notify(PM: any): void { for (const listener of listeners.get(PM) || []) listener(); PM.invalidate?.('render'); }
export function onViewsChange(PM: any, listener: () => void): () => void {
  let set = listeners.get(PM); if (!set) { set = new Set(); listeners.set(PM, set); } set.add(listener);
  // The selected pane follows the viewer's own view (Active Camera / Free View / Top…).
  const off = onViewportChange(PM, () => listener());
  return () => { set!.delete(listener); off(); };
}

/** What the selected pane shows right now, read from the viewer's view. */
function current(PM: any): ViewPane {
  if (getViewportMode(PM) === 'camera') return { view: 'camera', pose: null };
  return { view: viewportSide(PM) ?? 'custom', pose: viewportPose(PM) };
}
export function viewsState(PM: any): ViewsState {
  const e = entry(PM);
  return { layout: e.layout, active: e.active, panes: e.panes.map((pane, i) => i === e.active ? current(PM) : { ...pane }) };
}

/** Frame every model along an axis, as a Top or Front view first opens. */
export function sidePose(PM: any, side: ViewSide): ViewPose {
  const world = compositionRuntime(PM), box = new THREE.Box3();
  for (const [id, object] of world.objects as Map<string, THREE.Object3D>) {
    const layer = PM.L?.(id);
    if (!layer || layer.d?.data?.object?.world) continue;
    if (layer3DRole(layer) === 'object' || isExtrudedLayer(layer) || layer.threeD) box.expandByObject(object);
  }
  if (box.isEmpty()) box.setFromCenterAndSize(new THREE.Vector3(), new THREE.Vector3(2, 2, 2));
  // Fit the box's face seen along the axis (not its bounding sphere), so a
  // wide, flat landscape fills a Top view and a thin profile a Front view.
  const target = box.getCenter(new THREE.Vector3()), size = box.getSize(new THREE.Vector3());
  const lens = world.camera as THREE.PerspectiveCamera;
  if (!lens.isPerspectiveCamera) return frameView({ position: target.clone().add(sideDirection(side)), target, zoom: 1 }, box, world.camera);
  const [across, up, depth] = side === 'top' || side === 'bottom' ? [size.x, size.z, size.y] : side === 'left' || side === 'right' ? [size.z, size.y, size.x] : [size.x, size.y, size.z];
  const tanV = Math.tan(THREE.MathUtils.degToRad(lens.fov / 2)), tanH = tanV * Math.max(.1, lens.aspect || 16 / 9);
  const distance = Math.max(.5, 1.5 * Math.max(across / 2 / tanH, up / 2 / tanV) + depth / 2);
  return { position: target.clone().addScaledVector(sideDirection(side), distance), target, zoom: 1 };
}

function load(PM: any, pane: ViewPane): void {
  if (pane.view === 'camera') { setViewportMode(PM, 'camera'); return; }
  if (pane.view === 'custom') { if (pane.pose) applyViewportPose(PM, pane.pose); else setViewportMode(PM, 'editor'); return; }
  applySidePose(PM, pane.view, pane.pose ?? sidePose(PM, pane.view));
}

/** 1 View, 2 Views (side by side) or 4 Views. The view you were looking
    through stays selected: on the right of 2 Views, bottom right of 4 Views
    (Top | Front over Right | your view), as After Effects lays them out. */
export function setViewLayout(PM: any, layout: ViewLayout): void {
  if (layout !== 1 && layout !== 2 && layout !== 4) throw new Error('Unknown view layout');
  const e = entry(PM); if (e.layout === layout) return;
  const mine = current(PM);
  const others: PaneView[] = layout === 4 ? ['top', 'front', 'right'] : layout === 2 ? ['top'] : [];
  // Keep a pane's earlier choice when the layout grows back.
  const previous = e.panes.filter((_, i) => i !== e.active);
  const extra = others.map((view, i) => previous[i] && previous[i]!.view !== mine.view ? previous[i]! : { view, pose: null });
  e.panes = [...extra, mine]; e.active = e.panes.length - 1; e.layout = layout;
  notify(PM);
}
/** Select a pane: the viewer takes over its view. */
export function setActivePane(PM: any, index: number): void {
  const e = entry(PM); if (!Number.isInteger(index) || index < 0 || index >= e.panes.length || index === e.active) return;
  e.panes[e.active] = current(PM); e.active = index;
  load(PM, e.panes[index]!);
  notify(PM);
}
/** Choose what a pane looks through. A side view starts framed on the scene. */
export function setPaneView(PM: any, index: number, view: PaneView): void {
  const e = entry(PM); if (!PANE_VIEWS.some(v => v.id === view)) throw new Error('Unknown view');
  if (index === e.active) {
    const pane: ViewPane = view === 'custom' ? { view, pose: e.panes[index]?.view === 'custom' ? e.panes[index]!.pose : null } : { view, pose: null };
    if (view === 'custom' && !pane.pose && getViewportMode(PM) === 'editor') pane.pose = viewportPose(PM);
    load(PM, pane); notify(PM); return;
  }
  if (!e.panes[index]) return;
  e.panes[index] = { view, pose: null };
  notify(PM);
}

/** The camera a pane looks through, at its size. */
export function paneCamera(PM: any, pane: ViewPane, width: number, height: number): THREE.Camera {
  const world = compositionRuntime(PM);
  const camera = world.camera.clone() as THREE.PerspectiveCamera | THREE.OrthographicCamera;
  if (pane.view !== 'camera') {
    const pose = pane.pose ?? (pane.view === 'custom' ? viewportPose(PM) : sidePose(PM, pane.view));
    camera.position.copy(pose.position); camera.zoom = pose.zoom;
    if (pane.view === 'top') camera.up.set(0, 0, -1); else if (pane.view === 'bottom') camera.up.set(0, 0, 1); else camera.up.set(0, 1, 0);
    camera.lookAt(pose.target);
  }
  if ((camera as THREE.PerspectiveCamera).isPerspectiveCamera) (camera as THREE.PerspectiveCamera).aspect = width / Math.max(1, height);
  camera.updateProjectionMatrix(); camera.updateMatrixWorld(true);
  return camera;
}

/** A pane's picture from its own Studio session, at Live quality. Null until the first frame arrives. */
export function renderPane(PM: any, index: number, width: number, height: number): { canvas: CanvasImageSource; version: string } | null {
  const pane = viewsState(PM).panes[index]; if (!pane) return null;
  const time = PM.time || 0, world = compositionRuntime(PM, time);
  const ids = new Set<string>([...(world.objects as Map<string, unknown>).keys()]);
  if (!ids.size) return null;
  const camera = paneCamera(PM, pane, width, height);
  return studioLayer(PM, world, width, height, '', ids, true, camera, undefined, time, { slot: `pane-${index}` });
}

/** The selected 3D layers' boxes as line segments in a pane's pixels, so the
    selection reads in every view, as in After Effects. Empty when nothing
    3D is selected or it is behind the pane's camera. */
export function paneSelection(PM: any, index: number, width: number, height: number): number[][] {
  const pane = viewsState(PM).panes[index]; if (!pane) return [];
  const ids: string[] = PM.sel?.layers || []; if (!ids.length) return [];
  const world = compositionRuntime(PM), camera = paneCamera(PM, pane, width, height);
  const lines: number[][] = [];
  for (const id of ids) {
    const object = (world.objects as Map<string, THREE.Object3D>).get(id), layer = PM.L?.(id);
    if (!object || !layer || layer.d?.data?.object?.world) continue;
    const box = new THREE.Box3().setFromObject(object); if (box.isEmpty()) continue;
    const corners = [0, 1, 2, 3, 4, 5, 6, 7].map(i => new THREE.Vector3(i & 1 ? box.max.x : box.min.x, i & 2 ? box.max.y : box.min.y, i & 4 ? box.max.z : box.min.z));
    const view = corners.map(c => c.clone().applyMatrix4(camera.matrixWorldInverse));
    if (view.every(v => v.z > -((camera as THREE.PerspectiveCamera).near ?? 0.01))) continue;
    const screen = corners.map(c => { const p = c.clone().project(camera); return [(p.x + 1) / 2 * width, (1 - p.y) / 2 * height]; });
    for (const [a, b] of [[0, 1], [2, 3], [4, 5], [6, 7], [0, 2], [1, 3], [4, 6], [5, 7], [0, 4], [1, 5], [2, 6], [3, 7]] as const) {
      if (view[a]!.z > 0 || view[b]!.z > 0) continue;
      lines.push([...screen[a]!, ...screen[b]!]);
    }
  }
  return lines;
}
