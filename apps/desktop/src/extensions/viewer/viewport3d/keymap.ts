import type { Disposable, PowermoveAPI } from 'powermove';
import { addMenu, toolMenu, openViewportPie, type PieKind } from './menus';

/*
 * Blender's 3D Viewport keymap (Object Mode, with the number row emulating
 * the numpad). Bindings run before the editor's own keymap, but only apply in
 * the viewport context: the pointer is over the viewer, the composition has
 * 3D layers and no 2D layers are selected. Otherwise, or when an operator has
 * nothing to act on, the key falls through to its normal Powermove shortcut.
 */

export const VIEWPORT_KEYMAP: ReadonlyArray<readonly [key: string, operator: string, args?: readonly unknown[]]> = [
  ['g', 'transform.translate'], ['r', 'transform.rotate'], ['s', 'transform.scale'],
  ['alt+g', 'transform.clear', ['translate']], ['alt+r', 'transform.clear', ['rotate']], ['alt+s', 'transform.clear', ['scale']],
  ['a', 'select.all'], ['alt+a', 'select.none'], ['ctrl+i', 'select.invert'], ['b', 'select.box-arm'],
  ['w', 'tool.cycle-select'], ['shift+space', 'menu.tools'],
  ['x', 'menu.delete'], ['shift+d', 'object.duplicate'], ['alt+d', 'object.duplicate'],
  ['h', 'object.hide'], ['shift+h', 'object.hide-unselected'], ['alt+h', 'object.reveal'],
  ['i', 'object.keyframe-insert'], ['alt+i', 'object.keyframe-delete'],
  ['ctrl+p', 'object.parent'], ['alt+p', 'object.parent-clear'],
  ['shift+a', 'menu.add'], ['shift+s', 'pie.snap'], ['shift+c', 'view.cursor-reset'],
  ['z', 'pie.shading'], ['shift+z', 'shading.wireframe-toggle'], ['alt+shift+z', 'overlays.toggle'], ['alt+z', 'xray.toggle'],
  ['`', 'pie.view'], ['shift+`', 'view.walk'], ['shift+~', 'view.walk'], ['.', 'period'], [',', 'pie.orientation'],
  ['home', 'view.all'],
  ['1', 'view.front'], ['ctrl+1', 'view.back'], ['3', 'view.right'], ['ctrl+3', 'view.left'], ['7', 'view.top'], ['ctrl+7', 'view.bottom'],
  ['5', 'view.projection'], ['0', 'view.camera'], ['ctrl+0', 'view.set-active-camera'], ['ctrl+alt+0', 'view.align-camera'],
  ['2', 'view.orbit', [0, 15]], ['8', 'view.orbit', [0, -15]], ['4', 'view.orbit', [-15, 0]], ['6', 'view.orbit', [15, 0]], ['9', 'view.flip'], ['shift+4', 'view.roll', [15]], ['shift+6', 'view.roll', [-15]], ['/', 'view.local'], ['shift+b', 'view.zoom-border'],
  ['=', 'view.zoom', [-150]], ['+', 'view.zoom', [-150]], ['-', 'view.zoom', [150]],
  ['tab', 'edit.toggle'], ['e', 'edit.extrude'], ['m', 'edit.merge'],
  ['n', 'sidebar.toggle'], ['t', 'toolbar.toggle'], ['shift+tab', 'snapping.toggle'],
  ['f2', 'rename'], ['f3', 'search']
];
export const VIEWPORT_KEY_COMMAND = '3d.viewport.key';

export interface ViewportKeyHost {
  /** Pointer position over the viewer, for menus and pies that open under it. */
  pointer(): { clientX: number; clientY: number } | null;
}

export function installViewportKeys(api: PowermoveAPI, host: ViewportKeyHost): Disposable {
  let lastCode: string | null = null;
  const track = (event: KeyboardEvent) => { lastCode = event.code; };
  window.addEventListener('keydown', track, true);
  const at = () => { const p = host.pointer(); return p ? { x: p.clientX, y: p.clientY } : { x: window.innerWidth / 2, y: window.innerHeight / 2 }; };
  const viewport = api.scene3d.viewport;

  function rename(): boolean {
    const id = viewport.state().active, layer = id ? api.model.layer(id) : null;
    if (!layer) return false;
    const input = document.createElement('input');
    input.type = 'text'; input.value = layer.name; input.setAttribute('aria-label', 'Name');
    input.style.cssText = 'width:100%;box-sizing:border-box';
    const apply = () => {
      const name = input.value.trim();
      if (name && name !== layer.name) {
        const result = api.project.apply({ type: 'set_layer', target: layer.id, patch: { name } }, { label: 'Rename' });
        if (!result.ok) api.ui.toast(result.message, { error: true });
      }
    };
    const modal = api.ui.modal({ title: 'Rename Active Item', body: input, width: 320, actions: [{ label: 'Cancel' }, { label: 'Rename', pri: true, run: apply }] });
    input.addEventListener('keydown', (event) => { if (event.key === 'Enter') { event.preventDefault(); apply(); modal.close(); } });
    queueMicrotask(() => { input.focus(); input.select(); });
    return true;
  }

  function runKey(operator: string, ...args: unknown[]): unknown {
    if (!viewport.inContext()) return false;
    // The key event being dispatched tells Numpad . from the main keyboard's period.
    const current = (globalThis as { event?: Event }).event;
    if (current instanceof KeyboardEvent) lastCode = current.code;
    const state = viewport.state();
    switch (operator) {
      case 'menu.add': api.ui.menu(at(), addMenu(api)); return true;
      case 'menu.tools': api.ui.menu(at(), toolMenu(api)); return true;
      case 'menu.delete':
        if (!state.selected.length) return false;
        if (state.editMode && !state.editMode.selected) return false;
        api.ui.menu(at(), [{ label: state.editMode ? 'Delete Vertices' : 'Delete', kb: 'X', run: () => viewport.run('object.delete') }]);
        return true;
      case 'pie.view': case 'pie.shading': case 'pie.pivot': case 'pie.orientation': case 'pie.snap':
        openViewportPie(api, operator.slice(4) as PieKind, at(), lastCode);
        return true;
      // Numpad . frames the selection; the main keyboard's period opens the pivot pie.
      case 'period':
        if (lastCode === 'NumpadDecimal') return viewport.run('view.selected');
        openViewportPie(api, 'pivot', at(), lastCode);
        return true;
      case 'shading.wireframe-toggle':
        return viewport.run('shading', state.shading === 'wireframe' ? 'solid' : 'wireframe');
      case 'view.set-active-camera':
        if (!state.active || (api.model.layer(state.active)?.d as { definition?: string } | undefined)?.definition !== 'powermove.3d.camera') return false;
        return viewport.run(operator);
      case 'rename': return rename();
      case 'search': return api.commands.run('palette') !== false;
    }
    return viewport.run(operator, ...args);
  }

  const disposables: Disposable[] = [
    api.commands.register({ id: VIEWPORT_KEY_COMMAND, label: '3D viewport key', category: '3D Viewport', when: () => false, run: (operator, ...args) => runKey(String(operator), ...args) })
  ];
  for (const [key, operator, args] of VIEWPORT_KEYMAP)
    disposables.push(api.keybindings.bind({ key, command: VIEWPORT_KEY_COMMAND, args: [operator, ...(args ?? [])], priority: 50, contextual: true }));

  // Palette entries for discoverability; they act on the composition without needing the pointer.
  const palette: Array<[string, string, string | null]> = [
    ['3d.view.camera', 'Toggle Camera View', 'Numpad 0'], ['3d.view.front', 'Front View', 'Numpad 1'], ['3d.view.right', 'Right View', 'Numpad 3'],
    ['3d.view.top', 'Top View', 'Numpad 7'], ['3d.view.projection', 'Toggle Perspective/Orthographic', 'Numpad 5'],
    ['3d.view.selected', 'Frame Selected', 'Numpad .'], ['3d.view.all', 'Frame All', 'Home'],
    ['3d.view.align-camera', 'Align Active Camera to View', 'Ctrl Alt Numpad 0'], ['3d.view.lock-camera', 'Lock Camera to View', null],
    ['3d.shading.wireframe', 'Wireframe Shading', 'Z'], ['3d.shading.solid', 'Solid Shading', 'Z'],
    ['3d.shading.material', 'Material Preview Shading', 'Z'], ['3d.shading.rendered', 'Rendered Shading', 'Z']
  ];
  for (const [id, label, kb] of palette) {
    const [operator, args] = id.startsWith('3d.shading.') ? ['shading', [id.slice(11)]] : [id.slice(3), []];
    disposables.push(api.commands.register({ id, label: `3D Viewport: ${label}`, category: '3D Viewport', kb: kb ?? null,
      when: () => viewport.state().hasScene, run: () => viewport.run(operator, ...args) }));
  }
  return { dispose() { window.removeEventListener('keydown', track, true); for (const item of disposables.splice(0)) item.dispose(); } };
}
