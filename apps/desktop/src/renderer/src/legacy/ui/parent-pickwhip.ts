import type { PMRegistry } from '../registry';
import { timelineService, viewerService } from '../core/services';

/** One cancellable pickwhip, shared by the inspector and timeline. */
export function installParentPickwhip(PM: PMRegistry): void {
  let cancel: (() => void) | null = null;
  PM.beginParentPick = (event: PointerEvent, ids: string[]) => {
    cancel?.(); event.preventDefault(); event.stopPropagation();
    const layers = ids.map(id => PM.L(id)).filter((layer: any) => layer && !layer.lock && PM.TYPE_META[layer.type]?.transform !== false);
    if (!layers.length) return;
    const ns = 'http://www.w3.org/2000/svg';
    const overlay = document.createElementNS(ns, 'svg');
    overlay.setAttribute('style', 'position:fixed;inset:0;width:100vw;height:100vh;pointer-events:none;z-index:100000');
    const line = document.createElementNS(ns, 'path');
    line.setAttribute('fill', 'none'); line.setAttribute('stroke', 'var(--accent)'); line.setAttribute('stroke-width', '2');
    const label = document.createElementNS(ns, 'text'); label.setAttribute('fill', 'var(--tx)'); label.setAttribute('font-size', '12');
    overlay.append(line, label); document.body.append(overlay);
    let target: any = null;
    const move = (pointer: PointerEvent) => {
      const candidate = timelineService(PM)?.layerAtPoint(pointer.clientX, pointer.clientY)
        || viewerService(PM)?.layerAtPoint?.(pointer.clientX, pointer.clientY);
      target = candidate && PM.TYPE_META[candidate.type]?.transform !== false && layers.every((layer: any) => layer.id !== candidate.id && !PM.wouldCycle(layer, candidate.id)) ? candidate : null;
      line.setAttribute('d', `M ${event.clientX} ${event.clientY} C ${event.clientX + 60} ${event.clientY}, ${pointer.clientX - 60} ${pointer.clientY}, ${pointer.clientX} ${pointer.clientY}`);
      label.setAttribute('x', String(Math.min(window.innerWidth - 200, pointer.clientX + 12))); label.setAttribute('y', String(pointer.clientY - 12));
      label.textContent = target ? `Parent to ${target.name}` : 'Pick a parent · Esc to cancel';
    };
    const cleanup = () => {
      overlay.remove(); window.removeEventListener('pointermove', move, true); window.removeEventListener('pointerup', up, true);
      window.removeEventListener('pointercancel', cleanup, true); window.removeEventListener('keydown', key, true); window.removeEventListener('blur', cleanup); cancel = null;
    };
    const up = (pointer: PointerEvent) => {
      pointer.preventDefault(); pointer.stopPropagation(); move(pointer); const parent = target; cleanup();
      if (parent) PM.Edit.apply(layers.map((layer: any) => ({ type: 'set_layer', target: layer.id, patch: { parent: parent.id } })), { label: 'Parent layers', origin: 'interface' });
    };
    const key = (keyboard: KeyboardEvent) => { if (keyboard.key === 'Escape') { keyboard.preventDefault(); keyboard.stopImmediatePropagation(); cleanup(); } };
    cancel = cleanup; move(event);
    window.addEventListener('pointermove', move, true); window.addEventListener('pointerup', up, true);
    window.addEventListener('pointercancel', cleanup, true); window.addEventListener('keydown', key, true); window.addEventListener('blur', cleanup);
  };
}
