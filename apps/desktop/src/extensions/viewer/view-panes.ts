import type { PowermoveAPI } from 'powermove';

/* After Effects' 1 View / 2 Views / 4 Views for 3D compositions.

   The selected pane IS the viewer's stage: the live canvas, gizmos, light
   markers, navigation and Studio's refine all keep working there unchanged.
   The stage shrinks to that pane with margins (and a spacer for the second
   row); the other panes sit beside it, each a picture of the composition
   from its own view, rendered by Studio at Live quality in its own session.
   Clicking a pane selects it and passes the click on, so the layer under
   the pointer is selected in the same click. */

type PaneView = 'camera' | 'front' | 'left' | 'top' | 'back' | 'right' | 'bottom' | 'custom';
type Views = { layout: 1 | 2 | 4; active: number; panes: { view: PaneView }[] };
type Rect = { x: number; y: number; w: number; h: number };

export const VIEW_PANE_STYLES = `
  #panel-viewer .body{position:relative}
  #viewer-panes{position:absolute;left:0;right:0;pointer-events:none;z-index:0}
  .viewer-pane{position:absolute;pointer-events:auto;overflow:hidden;background:var(--bg-panel-2);box-shadow:0 0 0 1px var(--line);cursor:default}
  .viewer-pane canvas{position:absolute;display:block;background:#000}
  .viewer-pane:hover{background:color-mix(in srgb,var(--tx) 3%,var(--bg-panel-2))}
  #panel-viewer.multi-view #stage{z-index:1}
  #viewer-pane-frame{position:absolute;inset:0;z-index:8;pointer-events:none;box-shadow:inset 0 0 0 1px var(--accent)}
  .viewer-pane-label{position:absolute;left:6px;top:6px;z-index:9;display:flex;align-items:center;gap:4px;height:20px;padding:0 6px 0 7px;border:0;border-radius:var(--r-sm);
    background:color-mix(in srgb,var(--bg-float) 88%,transparent);color:var(--tx-2);font:11px/20px var(--f-ui);white-space:nowrap;cursor:default;pointer-events:auto}
  .viewer-pane-label:hover{color:var(--tx);background:var(--bg-float)}
  .viewer-pane-label:focus-visible{outline:2px solid var(--accent);outline-offset:1px}
  .viewer-pane-label svg{width:8px;height:8px;color:var(--tx-3)}
  #stage > .viewer-pane-label{color:var(--tx)}
`;

/** Pane rectangles inside the viewer's area, reading order. */
export function paneRects(layout: 1 | 2 | 4, width: number, height: number, gap = 1): Rect[] {
  const cols = layout === 1 ? 1 : 2, rows = layout === 4 ? 2 : 1;
  const w = (width - gap * (cols - 1)) / cols, h = (height - gap * (rows - 1)) / rows;
  return Array.from({ length: layout }, (_, i) => {
    const col = layout === 1 ? 0 : i % 2, row = layout === 4 ? Math.floor(i / 2) : 0;
    return { x: col * (w + gap), y: row * (h + gap), w, h };
  });
}
/** The composition frame inside a pane, fit with the viewer's 8 px padding. */
export function fitFrame(pane: { w: number; h: number }, comp: { w: number; h: number }, pad = 8): Rect {
  const z = Math.max(0.01, Math.min((pane.w - pad * 2) / comp.w, (pane.h - pad * 2) / comp.h));
  const w = comp.w * z, h = comp.h * z;
  return { x: (pane.w - w) / 2, y: (pane.h - h) / 2, w, h };
}

export function mountViewPanes(api: PowermoveAPI, body: HTMLElement, runtime: any): () => void {
  const scene = api.scene3d as any;
  if (!scene?.views) return () => {};
  const panel = body.closest<HTMLElement>('.panel');
  const container = document.createElement('div');
  container.id = 'viewer-panes';
  body.append(container);
  const style = document.createElement('style');
  style.textContent = VIEW_PANE_STYLES;
  container.append(style);
  const spacer = document.createElement('div');
  spacer.id = 'viewer-pane-spacer';
  spacer.setAttribute('aria-hidden', 'true');
  spacer.style.cssText = 'flex:1 1 0;min-height:0;display:none';
  const frame = document.createElement('div');
  frame.id = 'viewer-pane-frame';
  frame.setAttribute('aria-hidden', 'true');
  const activeLabel = document.createElement('button');
  activeLabel.type = 'button';
  activeLabel.className = 'viewer-pane-label';

  type PaneEl = { el: HTMLElement; canvas: HTMLCanvasElement; label: HTMLButtonElement; version: string; size: string };
  const panes = new Map<number, PaneEl>();
  let state: Views = scene.views.get();
  let rects: Rect[] = [];
  let disposed = false;

  const has3D = () => api.model.curComp().layers.some((l: any) => String(l.d?.definition ?? '').startsWith('powermove.3d.') || l.threeD);
  const labelFor = (view: PaneView) => scene.views.options.find((o: any) => o.id === view)?.label ?? 'Custom View';
  const chooseView = (index: number, anchor: HTMLElement) => {
    const current = scene.views.get().panes[index]?.view;
    api.ui.menu(anchor, scene.views.options.map((o: any) => ({ label: o.label, on: o.id === current, run: () => scene.views.setPaneView(index, o.id) })));
  };
  const fillLabel = (label: HTMLButtonElement, index: number, view: PaneView) => {
    const text = labelFor(view);
    if (label.dataset.text !== text) {
      label.dataset.text = text;
      label.innerHTML = `<span></span>${api.ui.icon('chevD')}`;
      label.firstElementChild!.textContent = text;
    }
    label.dataset.pane = String(index);
    label.setAttribute('aria-label', `View ${index + 1}: ${text}`);
  };
  activeLabel.addEventListener('pointerdown', event => event.stopPropagation());
  activeLabel.addEventListener('click', () => chooseView(state.active, activeLabel));

  const area = () => {
    const head = body.querySelector<HTMLElement>(':scope > #viewer-head');
    const foot = body.querySelector<HTMLElement>(':scope > #viewer-foot');
    const top = head ? head.offsetTop + head.offsetHeight : 0;
    const bottom = foot ? foot.offsetTop : body.clientHeight;
    return { top, width: body.clientWidth, height: Math.max(0, bottom - top) };
  };

  const ensurePane = (index: number): PaneEl => {
    let pane = panes.get(index);
    if (pane) return pane;
    const el = document.createElement('div');
    el.className = 'viewer-pane';
    el.dataset.pane = String(index);
    const canvas = document.createElement('canvas');
    const label = document.createElement('button');
    label.type = 'button';
    label.className = 'viewer-pane-label';
    el.append(canvas, label);
    label.addEventListener('pointerdown', event => event.stopPropagation());
    label.addEventListener('click', event => { event.stopPropagation(); chooseView(index, label); });
    el.addEventListener('pointerdown', event => activate(index, event));
    container.append(el);
    pane = { el, canvas, label, version: '', size: '' };
    panes.set(index, pane);
    return pane;
  };

  /* Select a pane, then hand the same click to the stage once it has moved
     there, so one click both selects the view and the layer under it. */
  const activate = (index: number, event: PointerEvent) => {
    if (event.button !== 0) { scene.views.setActive(index); return; }
    event.preventDefault();
    const { clientX, clientY } = event;
    scene.views.setActive(index);
    apply();
    runtime.layout?.();
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (disposed) return;
      const target = document.elementFromPoint(clientX, clientY);
      if (!target || !runtime.stage?.contains(target)) return;
      const init = { bubbles: true, cancelable: true, composed: true, clientX, clientY, button: 0, buttons: 1, pointerId: 1, pointerType: 'mouse', isPrimary: true };
      target.dispatchEvent(new PointerEvent('pointerdown', init));
      target.dispatchEvent(new MouseEvent('mousedown', init));
      window.dispatchEvent(new PointerEvent('pointerup', { ...init, buttons: 0 }));
      target.dispatchEvent(new MouseEvent('mouseup', { ...init, buttons: 0 }));
      target.dispatchEvent(new MouseEvent('click', { ...init, buttons: 0 }));
    }));
  };

  /* Place the stage in the selected pane and the pictures in the others. */
  const apply = () => {
    if (disposed) return;
    const stage: HTMLElement | undefined = runtime.stage;
    if (!stage) return;
    state = scene.views.get();
    const multi = state.layout > 1 && has3D();
    const layout = multi ? state.layout : 1;
    panel?.classList.toggle('multi-view', multi);
    const active = multi ? state.active : 0;
    const col = layout === 1 ? 0 : active % 2, row = layout === 4 ? Math.floor(active / 2) : 0;
    stage.style.marginLeft = layout > 1 && col === 1 ? 'calc(50% + 0.5px)' : '';
    stage.style.marginRight = layout > 1 && col === 0 ? 'calc(50% + 0.5px)' : '';
    stage.style.flex = layout === 4 ? '1 1 0' : '';
    if (layout === 4) {
      spacer.style.display = 'block';
      stage.style.marginTop = row === 1 ? '1px' : '';
      if (row === 1) stage.before(spacer); else stage.after(spacer);
    } else { spacer.style.display = 'none'; spacer.remove(); stage.style.marginTop = ''; }
    if (multi) {
      if (frame.parentElement !== stage) stage.append(frame);
      if (activeLabel.parentElement !== stage) stage.append(activeLabel);
      fillLabel(activeLabel, active, state.panes[active]!.view);
    } else { frame.remove(); activeLabel.remove(); }
    const a = area();
    container.style.top = `${a.top}px`;
    container.style.height = `${a.height}px`;
    rects = multi ? paneRects(layout, a.width, a.height) : [];
    for (const [index, pane] of panes) if (!multi || index === active || index >= layout) { pane.el.remove(); panes.delete(index); }
    if (!multi) return;
    rects.forEach((rect, index) => {
      if (index === active) return;
      const pane = ensurePane(index);
      Object.assign(pane.el.style, { left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.w}px`, height: `${rect.h}px` });
      fillLabel(pane.label, index, state.panes[index]!.view);
      const p = api.project.get(), fit = fitFrame(rect, { w: p.w, h: p.h });
      Object.assign(pane.canvas.style, { left: `${fit.x}px`, top: `${fit.y}px`, width: `${fit.w}px`, height: `${fit.h}px` });
      // Retina-sized Live pictures: Studio's Live tier traces fewer pixels and upscales.
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      const w = Math.max(2, Math.round(fit.w * dpr)), h = Math.max(2, Math.round(fit.h * dpr)), size = `${w}x${h}`;
      if (pane.size !== size) { pane.size = size; pane.canvas.width = w; pane.canvas.height = h; pane.version = ''; }
    });
  };

  /* Poll every pane on the display link. Studio bounds each pane to one
     in-flight Live image; only changed pictures copy into the visible canvas. */
  let raf = 0;
  const stats = { renders: 0, ms: 0 };
  runtime.viewPaneStats = stats;
  const loop = () => {
    raf = requestAnimationFrame(loop);
    if (!panes.size) return;
    const start = performance.now();
    for (const [index, pane] of panes) {
      let picture: { canvas: CanvasImageSource; version: string } | null = null, outline: number[][] = [];
      try {
        picture = scene.views.render(index, pane.canvas.width, pane.canvas.height);
        outline = scene.views.selection(index, pane.canvas.width, pane.canvas.height);
      } catch { picture = null; }
      // Redraw when the picture or the selection outline changed.
      const version = picture ? `${picture.version}|${outline.map(l => l.map(v => Math.round(v)).join(',')).join(';')}` : '';
      if (!picture || version === pane.version) continue;
      const c = pane.canvas.getContext('2d');
      if (!c) continue;
      const bg = String((api.project.get() as any).bg ?? '#000000');
      c.fillStyle = /^#[0-9a-f]{3,8}$/i.test(bg) ? bg : '#000';
      c.fillRect(0, 0, pane.canvas.width, pane.canvas.height);
      c.drawImage(picture.canvas, 0, 0, pane.canvas.width, pane.canvas.height);
      if (outline.length) {
        c.save();
        c.strokeStyle = getComputedStyle(container).getPropertyValue('--accent').trim() || '#FF6B1A';
        c.lineWidth = Math.max(1, Math.round(pane.canvas.width / Math.max(1, pane.canvas.clientWidth)));
        c.beginPath();
        for (const [x1, y1, x2, y2] of outline) { c.moveTo(x1!, y1!); c.lineTo(x2!, y2!); }
        c.stroke(); c.restore();
      }
      pane.version = version;
      stats.renders++;
    }
    stats.ms = performance.now() - start;
  };
  raf = requestAnimationFrame(loop);

  const observer = new ResizeObserver(() => apply());
  observer.observe(body);
  const subscription = scene.views.onChange(() => { apply(); runtime.layout?.(); });
  const structure = api.events.on('project:changed', ({ kind }: any) => { if (kind !== 'values') apply(); });
  apply();

  return () => {
    disposed = true;
    cancelAnimationFrame(raf);
    observer.disconnect();
    subscription.dispose();
    structure.dispose();
    const stage: HTMLElement | undefined = runtime.stage;
    if (stage) { stage.style.marginLeft = stage.style.marginRight = stage.style.marginTop = stage.style.flex = ''; }
    panel?.classList.remove('multi-view');
    spacer.remove(); frame.remove(); activeLabel.remove(); container.remove();
  };
}
