import type { PowermoveAPI, Scene3DViewportController } from 'powermove';

/** Each installed copy is one member × one API version/trust context.
 * Calls the public facade from a compiled extension; no mock or host.pm. */
export default function activate(api: PowermoveAPI): void {
  api.commands.register({ id: `${api.id}.run`, label: 'Probe viewport member', run: async (member: string) => {
    const s = api.scene3d, v = s.viewport;
    const stage = document.createElement('div');
    stage.style.cssText = 'position:absolute;left:-10000px;top:-10000px;width:480px;height:270px;pointer-events:none';
    // Walk's optional pointer lock must never capture the owner's pointer.
    stage.requestPointerLock = async () => {};
    document.body.appendChild(stage);
    const calls: unknown[] = [];
    let controller: Scene3DViewportController | null = null;
    const enter = () => {
      stage.dispatchEvent(new PointerEvent('pointerenter'));
      stage.dispatchEvent(new PointerEvent('pointermove', { clientX: -9760, clientY: -9865 }));
    };
    const tick = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    const assert = (condition: unknown, detail: unknown) => { if (!condition) throw new Error(JSON.stringify(detail)); };
    try {
      s.setView('editor');
      v.run('tool', 'tweak'); v.run('overlays.toggle');
      if (!v.state().overlays) v.run('overlays.toggle');
      s.select(null, ['probe-cube']);
      controller = v.attach(stage, { panComposition: (x, y) => calls.push(['pan', x, y]), zoomComposition: (f, x, y) => calls.push(['zoom', f, x, y]) });
      controller.update({ x: 0, y: 0, width: 480, height: 270 }, ['probe-cube']);
      enter();
      await tick();
      const c = controller;
      const event = (type = 'pointerdown', button = 0) => new PointerEvent(type, { button, clientX: -9760, clientY: -9865, cancelable: true });
      switch (member) {
        case 'attach': assert(stage.querySelectorAll('canvas').length > 0 && c.covers(['probe-cube']), 'attached overlay and controller'); break;
        case 'state': { const state = v.state(); assert(state.hasScene && state.inContext && state.selected.includes('probe-cube') && state.active === 'probe-cube' && state.view.mode === 'editor', state); break; }
        case 'onChange': {
          const seen: unknown[] = []; const off = v.onChange((state) => seen.push(state.snap));
          const before = v.state().snap; v.run('snapping.toggle'); await tick();
          assert(seen.includes(!before), seen); off.dispose(); const n = seen.length;
          v.run('snapping.toggle'); await tick(); assert(seen.length === n, seen); break;
        }
        case 'inContext': assert(v.inContext(), 'pointer in 3D stage'); stage.dispatchEvent(new PointerEvent('pointerleave')); assert(!v.inContext(), 'pointer left 3D stage'); break;
        case 'operators': assert(v.operators.includes('view.front') && v.operators.includes('transform.translate') && v.operators.includes('cursor.set') && new Set(v.operators).size === v.operators.length, v.operators); break;
        case 'run': assert(v.run('view.front') && v.state().view.axis === 'front' && v.run('cursor.set', [1, 2, 3]) && JSON.stringify(v.state().cursor) === '[1,2,3]', v.state()); v.run('cursor.set', [0, 0, 0]); break;
        case 'update': c.update({ x: 12, y: 15, width: 360, height: 200 }, []); await tick(); assert(stage.querySelectorAll('canvas').length > 0 && !c.modalActive(), 'updated overlay, no modal'); break;
        case 'pointerDown': { const e = event('pointerdown', 2); assert(c.pointerDown(e) === false && !e.defaultPrevented && !c.active(), 'plain right click falls through'); break; }
        case 'objectPress': { s.select(null, []); const e = event(); assert(c.objectPress(e, 'probe-cube') === true && s.selection().ids.includes('probe-cube'), s.selection()); window.dispatchEvent(event('pointerup')); break; }
        case 'hover': assert(c.hover(-20000, -20000) === false, 'outside gizmo'); break;
        case 'cursorAt': assert(c.cursorAt(-20000, -20000) === null, 'outside gizmo'); c.armBoxSelect(); assert(c.cursorAt(-9760, -9865) === 'crosshair', 'armed box cursor'); c.takeBoxSelect(); break;
        case 'wheel': { const before = v.state().view; const e = new WheelEvent('wheel', { deltaY: 100, clientX: -9760, clientY: -9865, cancelable: true }); assert(c.wheel(e) === true && e.defaultPrevented && v.state().view.mode === before.mode, 'wheel handled in editor view'); break; }
        case 'contextMenu': { const no = new MouseEvent('contextmenu', { cancelable: true }); assert(c.contextMenu(no) === false && !no.defaultPrevented, 'ordinary menu falls through'); const e = new MouseEvent('contextmenu', { shiftKey: true, cancelable: true, clientX: -9760, clientY: -9865 }); assert(c.contextMenu(e) === true && e.defaultPrevented && v.state().cursor.every(Number.isFinite), v.state().cursor); break; }
        case 'startTransform': assert(c.startTransform('translate') === true && c.modalActive(), 'transform begins synchronously'); c.cancel(); assert(!c.modalActive(), 'cancelled'); break;
        case 'armBoxSelect': assert(c.armBoxSelect() && c.cursorAt(-9760, -9865) === 'crosshair' && c.takeBoxSelect(), 'box armed and consumed'); break;
        case 'armZoomBorder': assert(c.armZoomBorder() && c.cursorAt(-9760, -9865) === 'crosshair', 'zoom border armed'); window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true })); assert(c.cursorAt(-20000, -20000) === null, 'escape disarmed border'); break;
        case 'startWalk': assert(c.startWalk() && c.modalActive(), 'walk starts'); c.cancel(); assert(!c.modalActive(), 'walk cancelled'); break;
        case 'extrude': {
          assert(c.extrude() === false && !c.modalActive(), 'no edit-mode selection');
          const added = s.edit({ operation: 'add_object', object: { id: 'probe-mesh', source: { mesh: { positions: [-1, -1, 0, 1, -1, 0, 0, 1, 0], indices: [0, 1, 2] } } } });
          assert(added.ok, added); s.select(null, ['probe-mesh']);
          assert(v.run('edit.toggle') && v.run('select.all'), 'mesh points selected');
          assert(c.extrude() && c.modalActive(), 'extrusion starts a modal transform');
          c.cancel(); assert(!c.modalActive(), 'extrusion cancelled');
          assert((api.model.layer('probe-mesh') as any).d.data.object.source.mesh.positions.length === 9, 'cancel restored the mesh');
          v.run('edit.toggle'); break;
        }
        case 'takeBoxSelect': assert(!c.takeBoxSelect() && c.armBoxSelect() && c.takeBoxSelect() && !c.takeBoxSelect(), 'box consumed exactly once'); break;
        case 'modalActive': assert(!c.modalActive(), 'idle'); c.startTransform('rotate'); assert(c.modalActive(), 'modal'); c.cancel(); assert(!c.modalActive(), 'idle again'); break;
        case 'pointer': assert(JSON.stringify(c.pointer()) === JSON.stringify({ clientX: -9760, clientY: -9865 }), c.pointer()); stage.dispatchEvent(new PointerEvent('pointerleave')); assert(c.pointer() === null, 'outside stage'); break;
        case 'covers': assert(c.covers(['probe-cube', 'probe-light']) && !c.covers([]) && !c.covers(['missing']), 'covers only real 3D items'); break;
        case 'glyphsIn': {
          c.hover(-20000, -20000);
          const box = { x0: -100000, y0: -100000, x1: 100000, y1: 100000 };
          const ids = c.glyphsIn(box);
          assert(ids.includes('probe-light') && !ids.includes('probe-cube') && c.glyphsIn({ x0: -100, y0: -100, x1: -90, y1: -90 }).length === 0, ids);
          api.transport.setTime(1);
          const afterSeek = c.glyphsIn(box);
          assert(!afterSeek.includes('probe-light'), { afterSeek, time: api.transport.time() });
          api.transport.setTime(0);
          assert(c.glyphsIn(box).includes('probe-light'), 'the light is pickable again at its active time');
          break;
        }
        case 'active': assert(!c.active(), 'idle'); c.startTransform('scale'); assert(c.active(), 'transform active'); c.cancel(); assert(!c.active(), 'idle after cancel'); break;
        case 'cancel': { const before = JSON.stringify(api.model.layer('probe-cube')); c.startTransform('translate'); window.dispatchEvent(new KeyboardEvent('keydown', { key: 'x' })); window.dispatchEvent(new KeyboardEvent('keydown', { key: '2' })); c.cancel(); assert(!c.active() && JSON.stringify(api.model.layer('probe-cube')) === before, 'cancel restores transform'); break; }
        case 'dispose': c.startTransform('translate'); c.dispose(); c.dispose(); assert(!c.active() && !c.modalActive() && stage.querySelectorAll('canvas').length === 0 && !v.inContext(), 'idempotent disposal removes overlays and controller'); break;
        default: throw new Error(`Unknown probe member ${member}`);
      }
      return { ok: true, member, apiVersion: api.manifest.apiVersion, calls };
    } catch (error) {
      return { ok: false, member, apiVersion: api.manifest.apiVersion, error: String((error as Error).message) };
    } finally { controller?.takeBoxSelect(); window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })); controller?.cancel(); controller?.dispose(); stage.remove(); }
  } });
}
