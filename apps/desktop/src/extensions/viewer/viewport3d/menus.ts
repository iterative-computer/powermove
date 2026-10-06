import type { Layer, MenuContribution, PowermoveAPI } from 'powermove';
import { openPie, type PieItem } from './pie';
import { icon } from './icons';

/*
 * Blender's viewport menus: the header's View, Select, Add and Object menus,
 * Shift+A, the object context menu, Delete's confirmation, and the Z, `, .,
 * comma and Shift+S pies. Every entry runs a named viewport operator.
 */

type Item = MenuContribution;
const run = (api: PowermoveAPI, operator: string, ...args: unknown[]) => () => api.scene3d.viewport.run(operator, ...args);
const command = (api: PowermoveAPI, id: string, ...args: unknown[]) => () => api.commands.run(id, ...args);

export const ORIENTATIONS = [
  { id: 'global', label: 'Global' }, { id: 'local', label: 'Local' }, { id: 'view', label: 'View' }
] as const;
export const PIVOTS = [
  { id: 'bounds', label: 'Bounding Box Center', icon: 'bounds' }, { id: 'cursor', label: '3D Cursor', icon: 'cursor' },
  { id: 'individual', label: 'Individual Origins', icon: 'individual' }, { id: 'median', label: 'Median Point', icon: 'median' },
  { id: 'active', label: 'Active Element', icon: 'active' }
] as const;
export const SHADINGS = [
  { id: 'wireframe', label: 'Wireframe', icon: 'wireframe' }, { id: 'solid', label: 'Solid', icon: 'solid' },
  { id: 'material', label: 'Material Preview', icon: 'material' }, { id: 'rendered', label: 'Rendered', icon: 'rendered' }
] as const;
export const TOOLS = [
  { id: 'tweak', label: 'Tweak', key: 'W', icon: 'tweak' }, { id: 'box', label: 'Select Box', key: 'W', icon: 'box' },
  { id: 'cursor', label: 'Cursor', key: '', icon: 'cursor' }, { id: 'move', label: 'Move', key: 'G', icon: 'move' },
  { id: 'rotate', label: 'Rotate', key: 'R', icon: 'rotate' }, { id: 'scale', label: 'Scale', key: 'S', icon: 'scale' }
] as const;
const MESHES = [
  ['plane', 'Plane'], ['box', 'Cube'], ['sphere', 'UV Sphere'], ['icosahedron', 'Ico Sphere'],
  ['cylinder', 'Cylinder'], ['cone', 'Cone'], ['torus', 'Torus'], ['capsule', 'Capsule']
] as const;

export function viewMenu(api: PowermoveAPI): Item[] {
  const state = api.scene3d.viewport.state();
  return [
    { label: 'Toolbar', kb: 'T', on: state.toolbar, run: run(api, 'toolbar.toggle') },
    { label: 'Sidebar', kb: 'N', on: state.sidebar, run: run(api, 'sidebar.toggle') },
    '-',
    { label: 'Frame Selected', kb: 'Numpad .', disabled: !state.selected.length, run: run(api, 'view.selected') },
    { label: 'Frame All', kb: 'Home', run: run(api, 'view.all') },
    { label: 'Walk Navigation', kb: 'Shift `', run: run(api, 'view.walk') },
    { label: 'Local View', kb: 'Numpad /', on: state.view.label.endsWith('(Local)'), run: run(api, 'view.local') },
    { label: 'Zoom Border', kb: 'Shift B', run: run(api, 'view.zoom-border') },
    '-',
    { header: 'Viewpoint' },
    { label: 'Camera', kb: 'Numpad 0', on: state.view.mode === 'camera', run: run(api, 'view.camera') },
    { label: 'Top', kb: 'Numpad 7', on: state.view.axis === 'top', run: run(api, 'view.top') },
    { label: 'Bottom', kb: 'Ctrl Numpad 7', on: state.view.axis === 'bottom', run: run(api, 'view.bottom') },
    { label: 'Front', kb: 'Numpad 1', on: state.view.axis === 'front', run: run(api, 'view.front') },
    { label: 'Back', kb: 'Ctrl Numpad 1', on: state.view.axis === 'back', run: run(api, 'view.back') },
    { label: 'Right', kb: 'Numpad 3', on: state.view.axis === 'right', run: run(api, 'view.right') },
    { label: 'Left', kb: 'Ctrl Numpad 3', on: state.view.axis === 'left', run: run(api, 'view.left') },
    { label: 'Perspective/Orthographic', kb: 'Numpad 5', run: run(api, 'view.projection') },
    '-',
    { header: 'Cameras' },
    { label: 'Set Active Object as Camera', kb: 'Ctrl Numpad 0', disabled: !state.active || (api.model.layer(state.active)?.d as { definition?: string } | undefined)?.definition !== 'powermove.3d.camera', run: run(api, 'view.set-active-camera') },
    { label: 'Align Active Camera to View', kb: 'Ctrl Alt Numpad 0', disabled: !state.hasCamera, run: run(api, 'view.align-camera') },
    { label: 'Lock Camera to View', on: state.lockCamera, run: run(api, 'view.lock-camera') },
    '-',
    { label: 'Center Cursor and Frame All', kb: 'Shift C', run: run(api, 'view.cursor-reset') }
  ];
}

export function selectMenu(api: PowermoveAPI): Item[] {
  return [
    { label: 'All', kb: 'A', run: run(api, 'select.all') },
    { label: 'None', kb: 'Alt A', run: run(api, 'select.none') },
    { label: 'Invert', kb: 'Ctrl I', run: run(api, 'select.invert') },
    { label: 'Box Select', kb: 'B', run: run(api, 'select.box-arm') },
    '-',
    { header: 'Select All by Type' },
    { label: 'Mesh', run: run(api, 'select.type', 'object') },
    { label: 'Light', run: run(api, 'select.type', 'light') },
    { label: 'Camera', run: run(api, 'select.type', 'camera') }
  ];
}

export function addMenu(api: PowermoveAPI): Item[] {
  const text = api.scene3d.modelRecipes.find((recipe) => recipe.id === 'text');
  return [
    { header: 'Mesh' },
    ...MESHES.map(([id, label]) => ({ label, icon: 'cube', run: command(api, '3d.add-object', id) })),
    '-',
    ...(text ? [{ label: 'Text', icon: 'cube', run: command(api, `3d.model.${text.id}`) }, '-' as const] : []),
    { header: 'Models' },
    ...api.scene3d.modelRecipes.filter((recipe) => recipe.id !== 'text').map((recipe) => ({ label: recipe.label, icon: 'cube', run: command(api, `3d.model.${recipe.id}`) })),
    '-',
    { header: 'Light' },
    { label: 'Point', icon: 'sun', run: command(api, '3d.add-light', 'point') },
    { label: 'Sun', icon: 'sun', run: command(api, '3d.add-light', 'sun') },
    { label: 'Spot', icon: 'sun', run: command(api, '3d.add-light', 'spot') },
    { label: 'Area', icon: 'sun', run: command(api, '3d.add-light', 'area') },
    '-',
    { label: 'Camera', icon: 'camera', run: command(api, '3d.add-camera') },
    '-',
    { label: 'Import Blender, glTF or OBJ…', icon: 'download', run: command(api, '3d.import-obj') }
  ];
}

export function objectMenu(api: PowermoveAPI): Item[] {
  const state = api.scene3d.viewport.state(), none = !state.selected.length;
  return [
    { header: 'Transform' },
    { label: 'Move', kb: 'G', disabled: none, run: run(api, 'transform.translate') },
    { label: 'Rotate', kb: 'R', disabled: none, run: run(api, 'transform.rotate') },
    { label: 'Scale', kb: 'S', disabled: none, run: run(api, 'transform.scale') },
    '-',
    { header: 'Clear' },
    { label: 'Location', kb: 'Alt G', disabled: none, run: run(api, 'transform.clear', 'translate') },
    { label: 'Rotation', kb: 'Alt R', disabled: none, run: run(api, 'transform.clear', 'rotate') },
    { label: 'Scale', kb: 'Alt S', disabled: none, run: run(api, 'transform.clear', 'scale') },
    '-',
    { header: 'Snap' },
    { label: 'Selection to Cursor', disabled: none, run: run(api, 'snap', 'selection-to-cursor') },
    { label: 'Selection to Active', disabled: state.selected.length < 2, run: run(api, 'snap', 'selection-to-active') },
    { label: 'Cursor to Selected', disabled: none, run: run(api, 'snap', 'cursor-to-selected') },
    { label: 'Cursor to World Origin', run: run(api, 'snap', 'cursor-to-origin') },
    '-',
    { label: 'Duplicate Objects', kb: 'Shift D', disabled: none, run: run(api, 'object.duplicate') },
    { label: 'Parent to Active', kb: 'Ctrl P', disabled: state.selected.length < 2, run: run(api, 'object.parent') },
    { label: 'Clear Parent', kb: 'Alt P', disabled: none, run: run(api, 'object.parent-clear') },
    '-',
    { header: 'Animation' },
    { label: 'Insert Keyframe', kb: 'I', disabled: none, run: run(api, 'object.keyframe-insert') },
    { label: 'Delete Keyframes', kb: 'Alt I', disabled: none, run: run(api, 'object.keyframe-delete') },
    '-',
    { header: 'Show/Hide' },
    { label: 'Hide Selected', kb: 'H', disabled: none, run: run(api, 'object.hide') },
    { label: 'Hide Unselected', kb: 'Shift H', run: run(api, 'object.hide-unselected') },
    { label: 'Show Hidden', kb: 'Alt H', run: run(api, 'object.reveal') },
    '-',
    { label: 'Delete', kb: 'X', disabled: none, run: run(api, 'object.delete') }
  ];
}

/** Right click on a model: Blender's Object Context Menu, then the layer's own menu. */
/** Edit Mode's Mesh menu. */
export function meshMenu(api: PowermoveAPI): Item[] {
  const state = api.scene3d.viewport.state(), none = !state.editMode?.selected;
  return [
    { header: 'Transform' },
    { label: 'Move', kb: 'G', disabled: none, run: run(api, 'transform.translate') },
    { label: 'Rotate', kb: 'R', disabled: none, run: run(api, 'transform.rotate') },
    { label: 'Scale', kb: 'S', disabled: none, run: run(api, 'transform.scale') },
    '-',
    { label: 'Extrude', kb: 'E', disabled: none, run: run(api, 'edit.extrude') },
    { label: 'Merge at Center', kb: 'M', disabled: (state.editMode?.selected ?? 0) < 2, run: run(api, 'edit.merge') },
    { label: 'Delete Vertices', kb: 'X', disabled: none, run: run(api, 'object.delete') },
    '-',
    { label: 'Select All', kb: 'A', run: run(api, 'select.all') },
    { label: 'Select None', kb: 'Alt A', run: run(api, 'select.none') },
    { label: 'Invert Selection', kb: 'Ctrl I', run: run(api, 'select.invert') }
  ];
}
export function modeMenu(api: PowermoveAPI): Item[] {
  const editing = !!api.scene3d.viewport.state().editMode;
  return [
    { label: 'Object Mode', on: !editing, run: () => { if (editing) api.scene3d.viewport.run('edit.toggle'); } },
    { label: 'Edit Mode', kb: 'Tab', on: editing, run: () => { if (!editing) api.scene3d.viewport.run('edit.toggle'); } }
  ];
}

export function objectContextMenu(api: PowermoveAPI, layer: Layer | null): Item[] {
  const state = api.scene3d.viewport.state(), none = !state.selected.length;
  return [
    { header: 'Object' },
    { label: 'Insert Keyframe', kb: 'I', disabled: none, run: run(api, 'object.keyframe-insert') },
    { label: 'Delete Keyframes', kb: 'Alt I', disabled: none, run: run(api, 'object.keyframe-delete') },
    '-',
    { label: 'Duplicate Objects', kb: 'Shift D', disabled: none, run: run(api, 'object.duplicate') },
    { label: 'Parent to Active', kb: 'Ctrl P', disabled: state.selected.length < 2, run: run(api, 'object.parent') },
    { label: 'Clear Parent', kb: 'Alt P', disabled: none, run: run(api, 'object.parent-clear') },
    '-',
    { label: 'Selection to Cursor', disabled: none, run: run(api, 'snap', 'selection-to-cursor') },
    { label: 'Cursor to Selected', disabled: none, run: run(api, 'snap', 'cursor-to-selected') },
    '-',
    { label: 'Hide', kb: 'H', disabled: none, run: run(api, 'object.hide') },
    { label: 'Delete', kb: 'X', disabled: none, run: run(api, 'object.delete') },
    ...(layer ? ['-' as const, { label: 'Layer…', run: () => api.ui.showLayerMenu(layer, lastMenuPoint, 'viewer') }] : [])
  ];
}
let lastMenuPoint = { clientX: 0, clientY: 0 };
export function rememberMenuPoint(point: { clientX: number; clientY: number }): void { lastMenuPoint = { clientX: point.clientX, clientY: point.clientY }; }

export function toolMenu(api: PowermoveAPI): Item[] {
  const tool = api.scene3d.viewport.state().tool;
  return TOOLS.map((entry) => ({ label: entry.label, kb: entry.key || null, on: tool === entry.id, run: run(api, 'tool', entry.id) }));
}

export type PieKind = 'view' | 'shading' | 'pivot' | 'orientation' | 'snap';
export function openViewportPie(api: PowermoveAPI, kind: PieKind, at: { x: number; y: number }, releaseCode: string | null): void {
  const state = api.scene3d.viewport.state();
  const item = (label: string, operator: string, args: unknown[] = [], extra: Partial<Exclude<PieItem, null>> = {}): PieItem => ({ label, run: run(api, operator, ...args), ...extra });
  const pies: Record<PieKind, { title: string; items: PieItem[] }> = {
    view: { title: 'View', items: [
      item('Left', 'view.left'), item('Right', 'view.right'), item('Bottom', 'view.bottom'), item('Top', 'view.top'),
      item('Front', 'view.front'), item('Back', 'view.back'), item('View Camera', 'view.camera', [], { on: state.view.mode === 'camera' }),
      item('View Selected', 'view.selected', [], { disabled: !state.selected.length })
    ] },
    shading: { title: 'Shading', items: [
      ...SHADINGS.map((shading) => item(shading.label, 'shading', [shading.id], { icon: icon(shading.icon, 15), on: state.shading === shading.id })),
      item('Toggle X-Ray', 'xray.toggle', [], { icon: icon('xray', 15), on: state.xray }), item('Overlays', 'overlays.toggle', [], { icon: icon('overlays', 15), on: state.overlays })
    ] },
    pivot: { title: 'Pivot Point', items: PIVOTS.map((pivot) => item(pivot.label, 'pivot', [pivot.id], { icon: icon(pivot.icon, 15), on: state.pivot === pivot.id })) },
    orientation: { title: 'Transform Orientation', items: [
      item('Global', 'orientation', ['global'], { on: state.orientation === 'global' }), item('Local', 'orientation', ['local'], { on: state.orientation === 'local' }),
      null, null, item('View', 'orientation', ['view'], { on: state.orientation === 'view' })
    ] },
    snap: { title: 'Snap', items: [
      item('Cursor to Grid', 'snap', ['cursor-to-grid']), item('Selection to Grid', 'snap', ['selection-to-grid'], { disabled: !state.selected.length }),
      item('Cursor to Selected', 'snap', ['cursor-to-selected'], { disabled: !state.selected.length }), item('Selection to Cursor', 'snap', ['selection-to-cursor'], { disabled: !state.selected.length }),
      item('Selection to Cursor (Keep Offset)', 'snap', ['selection-to-cursor-offset'], { disabled: !state.selected.length }),
      item('Selection to Active', 'snap', ['selection-to-active'], { disabled: state.selected.length < 2 }),
      item('Cursor to World Origin', 'snap', ['cursor-to-origin']), item('Cursor to Active', 'snap', ['cursor-to-active'], { disabled: !state.active })
    ] }
  };
  openPie({ ...pies[kind], at, releaseCode });
}
