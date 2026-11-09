import type { PowermoveAPI } from 'powermove';

/** The scene3d view, navigation and gizmo members in the v1.2.0 API. */
export default function activate(api: PowermoveAPI): void {
  api.commands.register({ id: `${api.id}.run`, label: 'Probe released 3D controls', run: async (input: unknown) => {
    const member = String(input);
    const scene = api.scene3d;
    const stage = document.createElement('div');
    stage.style.cssText = 'position:absolute;left:-10000px;top:-10000px;width:480px;height:270px;pointer-events:none';
    const initial = { view: scene.getView(), navigation: scene.getNavigationMode(), mode: scene.getMode(), space: scene.getSpace() };
    const attached: { navigation?: ReturnType<typeof scene.createNavigation>; gizmo?: ReturnType<typeof scene.createGizmo> } = {};
    const rect = { x: 0, y: 0, width: 480, height: 270 };
    const assert = (condition: unknown, detail: unknown) => { if (!condition) throw new Error(JSON.stringify(detail)); };
    const press = (button = 1) => new PointerEvent('pointerdown', { button, clientX: -9760, clientY: -9865, cancelable: true });
    const wheel = () => new WheelEvent('wheel', { deltaY: 100, clientX: -9760, clientY: -9865, cancelable: true });
    const makeNavigation = () => { const navigation = scene.createNavigation(stage); attached.navigation = navigation; navigation.update({ width: 480, height: 270 }, ['probe-cube']); return navigation; };
    const makeGizmo = () => { const gizmo = scene.createGizmo(stage); attached.gizmo = gizmo; gizmo.update(rect, ['probe-cube']); return gizmo; };
    try {
      document.body.appendChild(stage);
      scene.select(null, ['probe-cube']);
      scene.setView('editor');
      switch (member) {
        case 'scene.getView': assert(scene.getView() === 'editor', scene.getView()); break;
        case 'scene.setView': scene.setView('camera'); assert(scene.getView() === 'camera', 'camera'); scene.setView('editor'); assert(scene.getView() === 'editor', 'editor'); break;
        case 'scene.onViewChange': {
          const seen: string[] = []; const off = scene.onViewChange((mode) => seen.push(mode));
          scene.setView('camera'); assert(seen.includes('camera'), seen); off.dispose(); const count = seen.length;
          scene.setView('editor'); assert(seen.length === count, seen); break;
        }
        case 'scene.getNavigationMode': assert(['select', 'orbit', 'pan', 'dolly'].includes(scene.getNavigationMode()), scene.getNavigationMode()); break;
        case 'scene.setNavigationMode': scene.setNavigationMode('orbit'); assert(scene.getNavigationMode() === 'orbit', scene.getNavigationMode()); scene.setNavigationMode('select'); break;
        case 'scene.onNavigationChange': {
          const seen: string[] = []; const off = scene.onNavigationChange((mode) => seen.push(mode));
          scene.setNavigationMode('pan'); assert(seen.includes('pan'), seen); off.dispose(); const count = seen.length;
          scene.setNavigationMode('select'); assert(seen.length === count, seen); break;
        }
        case 'scene.createNavigation': {
          const n = makeNavigation(); assert(typeof n.pointerDown === 'function' && typeof n.wheel === 'function' && stage.hasAttribute('tabindex'), 'attached navigation'); break;
        }
        case 'scene.frame': {
          const seen: string[] = []; const off = scene.onViewChange((mode) => seen.push(mode));
          scene.frame(true); off.dispose(); assert(seen.includes('editor'), seen); break;
        }
        case 'scene.getMode': assert(['translate', 'rotate', 'scale'].includes(scene.getMode()), scene.getMode()); break;
        case 'scene.setMode': scene.setMode('rotate'); assert(scene.getMode() === 'rotate', scene.getMode()); break;
        case 'scene.getSpace': assert(['world', 'local'].includes(scene.getSpace()), scene.getSpace()); break;
        case 'scene.setSpace': scene.setSpace('local'); assert(scene.getSpace() === 'local', scene.getSpace()); break;
        case 'scene.onGizmoChange': {
          const seen: string[] = []; const off = scene.onGizmoChange((state) => seen.push(state.mode));
          scene.setMode(scene.getMode() === 'rotate' ? 'scale' : 'rotate'); assert(seen.length === 1, seen);
          off.dispose(); scene.setMode(initial.mode); assert(seen.length === 1, seen); break;
        }
        case 'scene.createGizmo': {
          const g = makeGizmo(); assert(stage.querySelector('canvas[data-scene-gizmo]') && g.covers(['probe-cube']), 'attached gizmo'); break;
        }
        case 'navigation.update': { const n = makeNavigation(); n.update({ width: 360, height: 200 }, ['probe-cube']); assert(stage.hasAttribute('tabindex') && !n.active(), 'resized navigation'); break; }
        case 'navigation.pointerDown': {
          const n = makeNavigation(), event = press(); assert(n.pointerDown(event) && event.defaultPrevented && n.active(), 'middle-button navigation');
          window.dispatchEvent(new PointerEvent('pointerup', { button: 1, cancelable: true })); assert(!n.active(), 'gesture ended'); break;
        }
        case 'navigation.wheel': {
          const n = makeNavigation(); scene.setNavigationMode('dolly'); const event = wheel();
          assert(n.wheel(event) && event.defaultPrevented && scene.getView() === 'editor', 'dolly consumed wheel'); break;
        }
        case 'navigation.frame': {
          const n = makeNavigation(), seen: string[] = []; const off = scene.onViewChange((mode) => seen.push(mode));
          n.frame(true); off.dispose(); assert(seen.includes('editor'), seen); break;
        }
        case 'navigation.active': { const n = makeNavigation(); assert(!n.active(), 'idle'); assert(n.pointerDown(press()) && n.active(), 'moving'); n.cancel(); assert(!n.active(), 'cancelled'); break; }
        case 'navigation.cancel': { const n = makeNavigation(); assert(n.pointerDown(press()), 'gesture'); n.cancel(); assert(!n.active() && scene.getView() === 'editor', 'restored'); break; }
        case 'navigation.dispose': { const n = makeNavigation(); n.dispose(); n.dispose(); assert(!stage.hasAttribute('tabindex') && !n.pointerDown(press()), 'detached navigation'); break; }
        case 'gizmo.update': { const g = makeGizmo(); g.update({ x: 10, y: 15, width: 360, height: 200 }, ['probe-cube']); assert(stage.querySelector('canvas[data-scene-gizmo]') && g.covers(['probe-cube']), 'updated gizmo'); break; }
        case 'gizmo.hover': assert(!makeGizmo().hover(-20000, -20000), 'outside handle'); break;
        case 'gizmo.pointerDown': { const g = makeGizmo(), event = press(2); assert(!g.pointerDown(event) && !event.defaultPrevented, 'right click falls through'); break; }
        case 'gizmo.active': assert(!makeGizmo().active(), 'idle'); break;
        case 'gizmo.covers': { const g = makeGizmo(); assert(g.covers(['probe-cube', 'probe-light']) && !g.covers([]) && !g.covers(['missing']), '3D-only coverage'); break; }
        case 'gizmo.cancel': { const g = makeGizmo(); g.cancel(); assert(!g.active(), 'cancelled'); break; }
        case 'gizmo.dispose': { const g = makeGizmo(); g.dispose(); g.dispose(); assert(!stage.querySelector('canvas[data-scene-gizmo]') && !g.active(), 'detached gizmo'); break; }
        default: throw new Error(`Unknown released 3D member ${member}`);
      }
      return { ok: true, member, apiVersion: api.manifest.apiVersion };
    } catch (error) {
      return { ok: false, member, apiVersion: api.manifest.apiVersion, error: String((error as Error).message) };
    } finally {
      try {
        attached.navigation?.cancel(); attached.navigation?.dispose(); attached.gizmo?.cancel(); attached.gizmo?.dispose();
        scene.setNavigationMode(initial.navigation); scene.setMode(initial.mode); scene.setSpace(initial.space); scene.setView(initial.view);
      } finally { stage.remove(); }
    }
  } });
}
