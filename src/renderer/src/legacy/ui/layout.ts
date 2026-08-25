/* Ported from js/ui/layout.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const h = PM.h, $ = PM.$;

PM.PANELS = {};      // id -> {title, icon, build(body, panel), header(el), persist}
PM.panelInst = {};   // id -> {el, body, def, cache, spec}

PM.registerPanel = (id: any, def: any) => { PM.PANELS[id] = Object.assign({ id, title: id }, def); };

const L: any = { root: null, ws: null };
PM.Layout = L;

/* Panels that own live GL/canvas state and cannot be safely re-hosted in a popout. */
const NO_POPOUT = new Set(['viewer', 'timeline']);

/* ── panel construction ────────────────────────────────── */
function buildPanel(spec: any, dock: any) {
  const def = PM.PANELS[spec.id];
  if (!def) return null;
  const inst = PM.panelInst[spec.id] || (PM.panelInst[spec.id] = { def, cache: null });
  /* Reuse the live panel element for persist canvases so apply() never destroys
     the timeline/viewer backing store (the source of clip/gutter misalignment). */
  if (def.persist && inst.el && inst.built) {
    inst.spec = spec; inst.dock = dock;
    applyPanelSize(inst.el, spec, def);
    inst.el.style.minHeight = (spec.min || 56) + 'px';
    inst.el.querySelector('.ptitle')?.replaceChildren(spec.title || def.title);
    setPanelCollapsed(spec.id, !!spec.collapsed, false);
    return inst.el;
  }
  const headless = !!(spec.headless || def.headless);
  const cls = '.panel' + (def.flush ? '.flush' : '') + (def.noscroll ? '.noscroll' : '') + (headless ? '.headless' : '');
  const el = h('div' + cls, { id: 'panel-' + spec.id });
  el.dataset.panel = spec.id;

  const hdr = h('header');
  const grip = h('span.grip', { title: 'Drag to move · right-click for options' }, PM.icon('grip'));
  const title = h('span.ptitle', spec.title || def.title);
  const options = h('button.panel-options', { title: `${def.title} options`, 'aria-label': `${def.title} panel options` }, PM.icon('more'));
  hdr.append(grip, title, h('span.sp'), options);
  el.appendChild(hdr);

  let body = h('div.body');
  if (def.persist && inst.cache) body = inst.cache;
  el.appendChild(body);

  /* Headless canvas panels still need an explicit, non-canvas drag surface.
     Keeping it small prevents panel movement from stealing timeline/stage input. */
  const moveHandle = headless && !def.hideMoveHandle
    ? h('button.panel-move-handle', {
      title: `Move ${def.title} · right-click for options`,
      'aria-label': `Move ${def.title} panel`,
    }, PM.icon('grip'))
    : null;
  if (moveHandle) el.appendChild(moveHandle);

  inst.el = el; inst.body = body; inst.header = hdr; inst.spec = spec; inst.dock = dock; inst.moveHandle = moveHandle;
  if (def.persist) inst.cache = body;

  /* sizing */
  applyPanelSize(el, spec, def);
  el.style.minHeight = (spec.min || 56) + 'px';

  if (!def.persist || !inst.built) {
    body.textContent = '';
    try { def.build && def.build(body, inst); } catch (e: any) { console.error('[panel]' + spec.id, e); body.appendChild(h('div.empty', 'Panel error: ' + e.message)); }
    inst.built = true;
  }
  if (moveHandle && def.moveSlot) {
    const slot = body.querySelector(def.moveSlot);
    if (slot) { moveHandle.classList.add('inline'); slot.insertBefore(moveHandle, slot.firstChild); }
  }
  try { def.header && def.header(hdr, inst); } catch (e) { }
  if (!hdr.querySelector('.grip')) hdr.insertBefore(grip, hdr.firstChild);

  const liveLocation = () => (L.ws && findPanel(L.ws, spec.id)) || { spec: inst.spec || spec, dock: inst.dock || dock };
  const beginMove = (e: any) => {
    if (e.button !== 0) return;
    const current = liveLocation();
    if (!current || !current.dock || !current.spec) return;
    e.stopPropagation();
    startPanelDrag(e, current.spec, current.dock, el);
  };

  /* Header interactions use the live workspace location. Persisted canvas panels
      keep their DOM nodes while moving, so captured initial dock objects go stale.
      Legacy chat panels are stripped by workspace normalization. */
   hdr.addEventListener('pointerdown', (e: any) => {
     if (e.target.closest('button')) return;
     if (spec.id === 'chat') return;
     beginMove(e);
   });
   hdr.addEventListener('dblclick', (e: any) => {
     if (e.target.closest('button')) return;
     if (spec.id === 'chat') return;
    setPanelCollapsed(spec.id, el.dataset.collapsed !== '1');
  });
  const openPanelMenu = (e: any) => {
    e.preventDefault(); e.stopPropagation();
    const current = liveLocation();
    if (current && current.dock && current.spec) panelMenu(e, current.spec, current.dock);
  };
  options.addEventListener('click', openPanelMenu);
  hdr.addEventListener('contextmenu', (e: any) => {
    if (spec.id === 'chat') { e.preventDefault(); return; }
    openPanelMenu(e);
  });
  if (moveHandle) {
    moveHandle.addEventListener('pointerdown', beginMove);
    moveHandle.addEventListener('contextmenu', openPanelMenu);
  }
  if (spec.collapsed) setPanelCollapsed(spec.id, true, false);
  return el;
}

function applyPanelSize(el: any, spec: any, def: any) {
  if (spec.flex || (!spec.size && !def.size)) el.style.flex = '1 1 auto';
  else el.style.flex = '0 0 ' + (spec.size || def.size) + 'px';
}

/* ── drag to move panels between docks ─────────────────── */
const DOCK_LABELS: any = { left: 'Left dock', center: 'Center dock', right: 'Right dock' };
const dockLabel = (id: any) => DOCK_LABELS[id] || id.replace(/^\w/, (c: any) => c.toUpperCase()) + ' dock';
const DOCK_ORDER: any = { left: 0, center: 1, right: 2 };

/** Convert a pre-removal drop index into a post-removal insertion index.
    The drop target is measured against DOM order that still contains the
    dragged panel; removing it first shifts every later slot down by one.
    Returns null when the drop would reproduce the current position (no-op),
    which keeps stray drops from forking built-in workspaces into "(edited)"
    copies. */
L.resolveDropIndex = (fromDockId: any, fromIndex: any, toDockId: any, index: any) => {
  if (!Number.isInteger(index)) return null;
  if (toDockId !== fromDockId) return index;
  const adj = index > fromIndex ? index - 1 : index;
  return adj === fromIndex ? null : adj;
};

/** Geometry-only drop targeting. Dock rectangles are expanded equally on all
    sides, so top/bottom placement is as forgiving as crossing into left/right
    docks. Panel centers divide the vertical insertion slots; the gaps and dock
    edges therefore remain large targets instead of precision-only lines. */
L.hitTestDockPlacement = (docks: any, x: any, y: any, tolerance: any = 28) => {
  let best = null;
  for (const dock of docks || []) {
    const r = dock.rect;
    if (!r || x < r.left - tolerance || x > r.right + tolerance || y < r.top - tolerance || y > r.bottom + tolerance) continue;
    const dx = x < r.left ? r.left - x : x > r.right ? x - r.right : 0;
    const dy = y < r.top ? r.top - y : y > r.bottom ? y - r.bottom : 0;
    const distance = dx * dx + dy * dy;
    if (!best || distance < best.distance) best = { dock, distance };
  }
  if (!best) return null;
  const panels = best.dock.panels || [];
  let index = panels.length;
  for (let i = 0; i < panels.length; i++) {
    const r = panels[i].rect;
    if (r && y < r.top + r.height / 2) { index = i; break; }
  }
  return { dockId: best.dock.id, index };
};

/** Add generous edge targets for missing side docks. Saved layouts are allowed
    to contain only a center dock; without these virtual targets, left/right
    docking is literally unreachable by dragging. Virtual targets come first so
    they win intentional edge drops when the center currently spans the window. */
L.buildDockDropTargets = (docks: any, bodyRect: any, edgeWidth: any = 96) => {
  const targets = [...(docks || [])];
  if (!bodyRect) return targets;
  const virtual = [];
  const make = (id: any, left: any, right: any) => ({
    id, virtual: true, panels: [],
    rect: { left, right, top: bodyRect.top, bottom: bodyRect.bottom, width: right - left, height: bodyRect.bottom - bodyRect.top },
  });
  if (!targets.some((dock: any) => dock.id === 'left')) virtual.push(make('left', bodyRect.left, Math.min(bodyRect.right, bodyRect.left + edgeWidth)));
  if (!targets.some((dock: any) => dock.id === 'right')) virtual.push(make('right', Math.max(bodyRect.left, bodyRect.right - edgeWidth), bodyRect.right));
  return [...virtual, ...targets];
};

let dragState: any = null;

function startPanelDrag(e: any, spec: any, dock: any, el: any) {
  const label = h('span.panel-ghost-label', PM.PANELS[spec.id].title);
  const destination = h('span.panel-ghost-destination', '');
  const ghost = h('div.panel-ghost', h('div.panel-ghost-card', label, destination));
  /* Fixed overlay: unlike an inserted panel-sized slot, this never reflows the
     dock underneath the pointer while we are deciding where to drop. */
  const preview = h('div.panel-drop-preview');
  document.body.append(ghost, preview);
  el.classList.add('drag-src');
  const fromIndex = Math.max(0, dock.panels.findIndex((p: any) => p.id === spec.id));
  dragState = {
    spec, fromDock: dock.id, fromIndex, ghost, destination, preview, srcEl: el,
    moved: false, frame: 0, pendingEvent: null, targetKey: '', hoverPanel: null,
  };

  const renderDragFrame = () => {
    if (!dragState) return;
    dragState.frame = 0;
    const ev = dragState.pendingEvent;
    dragState.pendingEvent = null;
    if (!ev) return;
    updateDropTarget(ev);
    const x = PM.clamp(ev.clientX + 12, 8, window.innerWidth - ghost.offsetWidth - 8);
    const y = PM.clamp(ev.clientY + 12, 8, window.innerHeight - ghost.offsetHeight - 8);
    ghost.style.transform = `translate3d(${Math.round(x)}px,${Math.round(y)}px,0)`;
  };

  const queueDragFrame = (ev: any) => {
    if (!dragState) return;
    dragState.pendingEvent = ev;
    if (!dragState.frame) dragState.frame = window.requestAnimationFrame(renderDragFrame);
  };

  function finish(wasCancelled: any) {
    const state = dragState;
    if (!state) return;
    if (state.frame) window.cancelAnimationFrame(state.frame);
    dragState = null;
    window.removeEventListener('keydown', state.onKey, true);
    ghost.remove(); preview.remove();
    state.srcEl && state.srcEl.classList.remove('drag-src');
    document.body.classList.remove('panel-dragging');
    if (!state.moved) return;
    if (wasCancelled) { PM.toast('Move cancelled'); return; }
    const target = state.dropTarget;
    if (!target) { PM.toast('Drop on a panel or dock to move ' + PM.PANELS[spec.id].title); return; }
    const index = L.resolveDropIndex(state.fromDock, state.fromIndex, target.dockId, target.index);
    if (index == null) { PM.toast(PM.PANELS[spec.id].title + ' is already here'); return; }
    const previousRects = capturePanelRects();
    PM.WS.mutate((w: any) => {
      removePanel(w, spec.id);
      insertPanel(w, spec, target.dockId, index);
    });
    animatePanelLayout(previousRects);
    PM.toast('Moved ' + PM.PANELS[spec.id].title + ' → ' + dockLabel(target.dockId));
  }

  const drag = PM.drag(e, {
    cursor: 'grabbing',
    move: (dx: any, dy: any, ev: any) => {
      if (!dragState.moved && Math.hypot(dx, dy) < 5) return;
      if (!dragState.moved) {
        dragState.moved = true;
        destination.textContent = 'Move panel';
        document.body.classList.add('panel-dragging');
      }
      ghost.classList.add('on');
      queueDragFrame(ev);
    },
    up: (dx: any, dy: any, ev: any) => {
      /* Commit against the pointer's final position even when pointerup arrives
         before the queued animation frame. */
      if (dragState && dragState.moved) {
        dragState.pendingEvent = ev;
        renderDragFrame();
      }
      finish(false);
    },
    cancel: () => finish(true),
  });
  /* Esc aborts an in-flight drag without touching the workspace */
  dragState.onKey = (ev: any) => {
    if (ev.key !== 'Escape') return;
    ev.preventDefault();
    ev.stopPropagation();
    drag.cancel();
  };
  window.addEventListener('keydown', dragState.onKey, true);
}

const dockPanels = (dockEl: any) =>
  [...dockEl.querySelectorAll(':scope > .panel:not(.panel-drop-preview)')];

function updateDropTarget(ev: any) {
  const preview = dragState.preview;
  dragState.dropTarget = null;
  const docks = [...document.querySelectorAll('#body > .dock')].map((el: any) => ({
    id: el.id.replace('dock-', ''), el, rect: el.getBoundingClientRect(),
    panels: dockPanels(el).map((panel: any) => ({ el: panel, rect: panel.getBoundingClientRect() })),
  }));
  const bodyRect = document.getElementById('body')?.getBoundingClientRect();
  const targets = L.buildDockDropTargets(docks, bodyRect);
  const target = L.hitTestDockPlacement(targets, ev.clientX, ev.clientY);
  if (!target) {
    preview.classList.remove('on');
    dragState.targetKey = '';
    dragState.hoverPanel = null;
    dragState.destination.textContent = 'Not a drop zone';
    return;
  }
  const dock = targets.find((item: any) => item.id === target.dockId);
  const dockEl = dock.el;
  const panels = dock.panels;
  const before = panels[target.index];
  const after = target.index ? panels[target.index - 1] : null;
  if (before) dragState.destination.textContent = 'Above ' + (PM.PANELS[before.el.dataset.panel]?.title || 'panel');
  else if (after) dragState.destination.textContent = 'Below ' + (PM.PANELS[after.el.dataset.panel]?.title || 'panel');
  else dragState.destination.textContent = 'Place in ' + dockLabel(target.dockId);
  dragState.dropTarget = target;
  if (dock.virtual) placeEdgePreview(preview, dock);
  else placePreview(preview, dockEl, target.index);
}

function placeEdgePreview(preview: any, dock: any) {
  const r = dock.rect;
  const width = Math.min(dock.id === 'right' ? 300 : 250, Math.max(120, Math.round((r.right - r.left) * 2.5)));
  preview.classList.add('dock-edge');
  preview.style.left = Math.round(dock.id === 'right' ? r.right - width : r.left) + 'px';
  preview.style.top = Math.round(r.top) + 'px';
  preview.style.width = width + 'px';
  preview.style.height = Math.max(80, Math.round(r.bottom - r.top)) + 'px';
  const key = `virtual:${dock.id}`;
  if (dragState.targetKey !== key) {
    dragState.targetKey = key; preview.classList.remove('on');
    window.requestAnimationFrame(() => { if (preview.isConnected) preview.classList.add('on'); });
  } else preview.classList.add('on');
}

/** Show a fixed placement bar between panels. It is deliberately outside the
    dock flow, so moving it cannot alter hit-testing geometry. */
function placePreview(preview: any, dockEl: any, index: any) {
  preview.classList.remove('dock-edge');
  preview.style.height = '';
  const panels = dockPanels(dockEl);
  const dockRect = dockEl.getBoundingClientRect();
  const before = panels[index];
  const after = index > 0 ? panels[index - 1] : null;
  let y;
  if (before) y = before.getBoundingClientRect().top - 3;
  else if (after) y = after.getBoundingClientRect().bottom + 3;
  else y = dockRect.top + 8;
  y = PM.clamp(y, dockRect.top + 4, dockRect.bottom - 8);
  const key = `${dockEl.id}:${index}`;
  preview.style.left = Math.round(dockRect.left + 8) + 'px';
  preview.style.top = Math.round(y) + 'px';
  preview.style.width = Math.max(24, Math.round(dockRect.width - 16)) + 'px';
  if (dragState.targetKey !== key) {
    dragState.targetKey = key;
    preview.classList.remove('on');
    window.requestAnimationFrame(() => { if (preview.isConnected) preview.classList.add('on'); });
  } else {
    preview.classList.add('on');
  }
}

function capturePanelRects() {
  const rects = new Map();
  document.querySelectorAll('#body .panel[data-panel]').forEach((node: any) => {
    rects.set(node.dataset.panel, node.getBoundingClientRect());
  });
  return rects;
}

/** FLIP-style translation after a drop. Only compositor transforms animate;
    panel contents and editor canvases are never continuously resized. */
function animatePanelLayout(previousRects: any) {
  if (!previousRects.size || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  document.querySelectorAll('#body .panel[data-panel]').forEach((node: any) => {
    const before = previousRects.get(node.dataset.panel);
    if (!before || !node.animate) return;
    const after = node.getBoundingClientRect();
    const dx = before.left - after.left;
    const dy = before.top - after.top;
    if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
    node.animate([
      { transform: `translate3d(${Math.round(dx)}px,${Math.round(dy)}px,0)` },
      { transform: 'translate3d(0,0,0)' },
    ], { duration: 180, easing: 'cubic-bezier(.22,1,.36,1)' });
  });
}

/* ── panel context menu ────────────────────────────────── */
function panelMenu(e: any, spec: any, dock: any) {
  const ws = L.ws;
  const def = PM.PANELS[spec.id];
  const position = dock.panels.findIndex((item: any) => item.id === spec.id);
  const items = [
    { header: def.title },
    !NO_POPOUT.has(spec.id) ? { label: 'Pop out to window', run: () => PM.Popout.open(spec.id) } : null,
    spec.id !== 'viewer' ? { label: 'Hide panel', run: () => PM.WS.mutate((w: any) => hidePanel(w, spec.id)) } : null,
    '-',
    { header: 'Move to' },
    { label: 'Left dock', disabled: dock.id === 'left', run: () => PM.WS.mutate((w: any) => movePanel(w, spec.id, 'left')) },
    { label: 'Center dock', disabled: dock.id === 'center', run: () => PM.WS.mutate((w: any) => movePanel(w, spec.id, 'center')) },
    { label: 'Right dock', disabled: dock.id === 'right', run: () => PM.WS.mutate((w: any) => movePanel(w, spec.id, 'right')) },
    { label: 'Move up', disabled: position <= 0, run: () => PM.WS.mutate((w: any) => movePanelBy(w, spec.id, -1)) },
    { label: 'Move down', disabled: position < 0 || position >= dock.panels.length - 1, run: () => PM.WS.mutate((w: any) => movePanelBy(w, spec.id, 1)) },
    '-',
    { header: 'Add panel' },
    ...Object.values(PM.PANELS).filter((p: any) => !hasPanel(ws, p.id) && !(ws.hiddenPanels || []).some((item: any) => item.id === p.id) && p.id !== 'toolbar').map((p: any) => ({
      label: p.title, run: () => PM.WS.mutate((w: any) => addPanel(w, p.id, dock.id)),
    })),
    ...(ws.hiddenPanels || []).filter((item: any) => PM.PANELS[item.id]).map((item: any) => ({
      label: `Restore ${PM.PANELS[item.id].title}`, run: () => PM.WS.mutate((w: any) => restorePanel(w, item.id)),
    })),
  ].filter(Boolean);
  PM.menu(document.body, items, { x: e.clientX, y: e.clientY });
}

/* ── workspace model ops ───────────────────────────────── */
const eachDock = (ws: any, fn: any) => ws.layout.docks.forEach(fn);
const hasPanel = (ws: any, id: any) => ws.layout.docks.some((d: any) => d.panels.some((p: any) => p.id === id));
function ensureDockFill(dock: any) {
  if (dock?.panels?.length && !dock.panels.some((panel: any) => panel.flex)) dock.panels[dock.panels.length - 1].flex = true;
  return dock;
}
function removePanel(ws: any, id: any) { eachDock(ws, (d: any) => { d.panels = d.panels.filter((p: any) => p.id !== id); ensureDockFill(d); }); }
function ensureDock(ws: any, id: any) {
  let dock = ws.layout.docks.find((item: any) => item.id === id);
  if (dock) return dock;
  dock = { id, panels: [] };
  if (id === 'center') dock.flex = true;
  else dock.size = id === 'right' ? 300 : 250;
  const order = DOCK_ORDER[id] ?? 3;
  const index = ws.layout.docks.findIndex((item: any) => (DOCK_ORDER[item.id] ?? 3) > order);
  ws.layout.docks.splice(index < 0 ? ws.layout.docks.length : index, 0, dock);
  return dock;
}
function hidePanel(ws: any, id: any) {
  if (id === 'viewer') return false;
  const found = findPanel(ws, id); if (!found) return false;
  const index = found.dock.panels.indexOf(found.spec);
  ws.hiddenPanels = (ws.hiddenPanels || []).filter((item: any) => item.id !== id);
  ws.hiddenPanels.push({
    id, dockId: found.dock.id, index, dockIndex: ws.layout.docks.indexOf(found.dock), spec: { ...found.spec },
    dock: { id: found.dock.id, size: found.dock.size, flex: found.dock.flex },
  });
  found.dock.panels.splice(index, 1);
  ensureDockFill(found.dock);
  return true;
}
function restorePanel(ws: any, id: any) {
  const hidden = (ws.hiddenPanels || []).find((item: any) => item.id === id);
  if (!hidden) return false;
  if (hasPanel(ws, id)) { ws.hiddenPanels = ws.hiddenPanels.filter((item: any) => item.id !== id); return true; }
  let dock = ws.layout.docks.find((item: any) => item.id === hidden.dockId);
  if (!dock) {
    dock = { id: hidden.dockId, panels: [] };
    if (hidden.dock?.size) dock.size = hidden.dock.size;
    if (hidden.dock?.flex) dock.flex = true;
    const dockIndex = Math.max(0, Math.min(hidden.dockIndex ?? ws.layout.docks.length, ws.layout.docks.length));
    ws.layout.docks.splice(dockIndex, 0, dock);
  }
  const index = Math.max(0, Math.min(hidden.index, dock.panels.length));
  dock.panels.splice(index, 0, { ...hidden.spec, id });
  ensureDockFill(dock);
  ws.hiddenPanels = ws.hiddenPanels.filter((item: any) => item.id !== id);
  return true;
}
function findPanel(ws: any, id: any) {
  for (const d of ws.layout.docks) { const s = d.panels.find((p: any) => p.id === id); if (s) return { dock: d, spec: s }; }
  return null;
}
function addPanel(ws: any, id: any, dockId: any) {
  removePanel(ws, id);
  ws.hiddenPanels = (ws.hiddenPanels || []).filter((item: any) => item.id !== id);
  const d = ensureDock(ws, dockId || 'right');
  d.panels.push({ id, flex: d.panels.length === 0 });
  ensureDockFill(d);
  if (d.hidden) d.hidden = false;
}
function insertPanel(ws: any, spec: any, dockId: any, index?: any) {
  const d = ensureDock(ws, dockId || 'center');
  const clean: any = { id: spec.id };
  if (spec.size) clean.size = spec.size;
  if (spec.flex) clean.flex = true;
  const i = index == null ? d.panels.length : Math.max(0, Math.min(index, d.panels.length));
  d.panels.splice(i, 0, clean);
  ensureDockFill(d);
  if (d.hidden) d.hidden = false;
}
function movePanel(ws: any, id: any, dockId: any) {
  const found = findPanel(ws, id); if (!found || found.dock.id === dockId) return false;
  const spec = { ...found.spec };
  removePanel(ws, id);
  insertPanel(ws, spec, dockId);
  ws.hiddenPanels = (ws.hiddenPanels || []).filter((item: any) => item.id !== id);
  return true;
}
function movePanelBy(ws: any, id: any, delta: any) {
  const found = findPanel(ws, id); if (!found || !Number.isInteger(delta) || !delta) return false;
  const from = found.dock.panels.indexOf(found.spec);
  const to = Math.max(0, Math.min(from + delta, found.dock.panels.length - 1));
  if (to === from) return false;
  found.dock.panels.splice(from, 1);
  found.dock.panels.splice(to, 0, found.spec);
  return true;
}
function setPanelCollapsed(id: any, collapsed: any, emit: any = true) {
  if (id === 'viewer') return false;
  const inst = PM.panelInst[id];
  const current = L.ws && findPanel(L.ws, id);
  if (!inst?.el || !inst.body || !current?.spec) return false;
  current.spec.collapsed = !!collapsed;
  inst.el.dataset.collapsed = collapsed ? '1' : '0';
  inst.body.style.display = collapsed ? 'none' : '';
  inst.el.style.flex = collapsed
    ? '0 0 var(--hdr-h)'
    : (current.spec.flex ? '1 1 auto' : '0 0 ' + (current.spec.size || inst.def.size || 180) + 'px');
  if (emit) { PM.WS.save(); PM.bus.emit('layout:applied'); }
  return true;
}
L.removePanel = removePanel; L.hidePanel = hidePanel; L.restorePanel = restorePanel;
L.addPanel = addPanel; L.movePanel = movePanel; L.movePanelBy = movePanelBy; L.ensureDock = ensureDock; L.hasPanel = hasPanel; L.findPanel = findPanel;
L.setCollapsed = setPanelCollapsed;

/* ── splitters ─────────────────────────────────────────── */
/* A splitter always resizes the nearest *fixed* pane and leaves the flex pane
   to absorb the change. Sign is chosen so dragging the bar toward a pane shrinks
   that pane — never the opposite direction. */
function isFlexDock(d: any) { return !d || d.id === 'center' || !!d.flex; }
function isFlexPanel(spec: any) {
  if (!spec) return true;
  const def = PM.PANELS[spec.id] || {};
  return !!(spec.flex || (!spec.size && !def.size));
}
function attachSplitterHover(s: any, axis: any) {
  const property = axis === 'x' ? '--splitter-hover-x' : '--splitter-hover-y';
  let current: any = null;
  let target: any = null;
  let raf = 0;
  const positionFromEvent = (e: any) => {
    const rect = s.getBoundingClientRect();
    const length = axis === 'x' ? rect.width : rect.height;
    const pointer = axis === 'x' ? e.clientX - rect.left : e.clientY - rect.top;
    return PM.clamp(pointer, 0, length);
  };
  const paint = () => {
    current += (target - current) * .36;
    if (Math.abs(target - current) < .1) current = target;
    s.style.setProperty(property, current.toFixed(2) + 'px');
    raf = current === target ? 0 : window.requestAnimationFrame(paint);
  };
  const track = (e: any) => {
    target = positionFromEvent(e);
    if (current == null) {
      current = target;
      s.style.setProperty(property, current.toFixed(2) + 'px');
      return;
    }
    if (!raf) raf = window.requestAnimationFrame(paint);
  };
  s.addEventListener('pointerenter', track, { passive: true });
  s.addEventListener('pointermove', track, { passive: true });
  return track;
}
L.clampPanelHeight = (start: any, delta: any, sign: any, pairHeight: any, minHeight: any = 88, otherMinHeight: any = 88, gap: any = 8) => {
  const min = Math.max(72, Number(minHeight) || 88);
  const max = Math.max(min, (Number(pairHeight) || min * 2 + gap) - Math.max(72, Number(otherMinHeight) || 88) - gap);
  return PM.clamp(start + sign * delta, min, max);
};
function vSplit(prev: any, next: any) {
  const s = h('div.splitter');
  const trackPointer = attachSplitterHover(s, 'y');
  /* Prefer the side dock. Dragging right grows the left dock / shrinks the right dock. */
  const useLeft = !isFlexDock(prev);
  const target = useLeft ? prev : next;
  const sign = useLeft ? 1 : -1;
  s.addEventListener('pointerdown', (e: any) => {
    const el = document.getElementById('dock-' + target.id);
    if (!el) return;
    trackPointer(e);
    const start = el.getBoundingClientRect().width;
    s.classList.add('drag');
    let raf = 0;
    PM.drag(e, {
      cursor: 'col-resize',
      move: (dx: any, dy: any, ev: any) => {
        trackPointer(ev);
        const w = PM.clamp(start + sign * dx, 200, 760);
        el.style.flex = '0 0 ' + w + 'px';
        target.size = Math.round(w);
        target.flex = false;
        if (!raf) raf = window.requestAnimationFrame(() => { raf = 0; PM.bus.emit('layout:applied'); });
      },
      up: () => {
        s.classList.remove('drag');
        PM.WS.save();
        PM.bus.emit('layout');
        PM.bus.emit('layout:applied');
      },
    });
  });
  s.addEventListener('dblclick', () => {
    const def = target.id === 'right' ? 300 : 250;
    const el = document.getElementById('dock-' + target.id);
    if (!el) return;
    el.style.flex = '0 0 ' + def + 'px'; target.size = def;
    PM.WS.save(); PM.bus.emit('layout:applied');
  });
  return s;
}
function hSplit(aboveSpec: any, aboveEl: any, belowSpec: any, belowEl: any) {
  const s = h('div.splitter.h');
  const trackPointer = attachSplitterHover(s, 'x');
  const useAbove = !isFlexPanel(aboveSpec);
  const spec = useAbove ? aboveSpec : belowSpec;
  const node = useAbove ? aboveEl : belowEl;
  const other = useAbove ? belowSpec : aboveSpec;
  const otherEl = useAbove ? belowEl : aboveEl;
  const sign = useAbove ? 1 : -1;
  s.addEventListener('pointerdown', (e: any) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    trackPointer(e);
    const start = node.getBoundingClientRect().height;
    const otherStart = otherEl.getBoundingClientRect().height;
    const pairHeight = start + otherStart;
    const saved = { size: spec.size, flex: spec.flex };
    const otherSaved = { size: other.size, flex: other.flex };
    const min = spec.min || PM.PANELS[spec.id]?.min || 88;
    const otherMin = other.min || PM.PANELS[other.id]?.min || 88;
    let changed = false;
    s.classList.add('drag');
    let raf = 0;
    PM.drag(e, {
      cursor: 'row-resize',
      move: (dx: any, dy: any, ev: any) => {
        trackPointer(ev);
        if (!changed && Math.abs(dy) < 2) return;
        changed = true;
        const hh = L.clampPanelHeight(start, dy, sign, pairHeight, min, otherMin, parseFloat(window.getComputedStyle(s).height) || 8);
        node.style.flex = '0 0 ' + hh + 'px';
        spec.size = Math.round(hh);
        delete spec.flex;
        other.flex = true; delete other.size;
        otherEl.style.flex = '1 1 auto';
        if (!raf) raf = window.requestAnimationFrame(() => { raf = 0; PM.bus.emit('layout:applied'); });
      },
      up: () => {
        s.classList.remove('drag');
        if (!changed) return;
        PM.WS.save();
        PM.bus.emit('layout');
        PM.bus.emit('layout:applied');
      },
      cancel: () => {
        s.classList.remove('drag');
        if (!changed) return;
        if (saved.size == null) delete spec.size; else spec.size = saved.size;
        if (saved.flex == null) delete spec.flex; else spec.flex = saved.flex;
        if (otherSaved.size == null) delete other.size; else other.size = otherSaved.size;
        if (otherSaved.flex == null) delete other.flex; else other.flex = otherSaved.flex;
        applyPanelSize(node, spec, PM.PANELS[spec.id] || {});
        applyPanelSize(otherEl, other, PM.PANELS[other.id] || {});
        PM.bus.emit('layout:applied');
      },
    });
  });
  return s;
}

/* ── apply ─────────────────────────────────────────────── */
L.visibleDockPlan = (ws: any, isDetached: any = (id: any) => !!PM.Popout?.isOpen(id)) => (ws.layout?.docks || [])
  .map((dock: any) => ({ dock, specs: (dock.panels || []).filter((spec: any) => !isDetached(spec.id)) }))
  .filter((item: any) => !item.dock.hidden && item.specs.length);

L.apply = (ws: any) => {
  L.ws = ws;
  const root = $('#body');
  /* Lift persist panels out before wiping the dock tree so their canvases
     stay alive and ResizeObservers do not record a 0×0 frame. */
  const park = document.createElement('div');
  park.style.cssText = 'position:absolute;left:-9999px;top:0;width:1px;height:1px;overflow:hidden;';
  document.body.appendChild(park);
  Object.values(PM.panelInst).forEach((inst: any) => {
    if (inst.def && inst.def.persist && inst.el && inst.el.parentNode) park.appendChild(inst.el);
  });
  root.textContent = '';
  let prevVisible: any = null;
  L.visibleDockPlan(ws).forEach(({ dock, specs: visibleSpecs }: any) => {
    const el = h('div.dock.col', { id: 'dock-' + dock.id });
    el.dataset.dock = dock.id;
    if (dock.id === 'center' || dock.flex) el.style.flex = '1 1 auto';
    else el.style.flex = '0 0 ' + (dock.size || (dock.id === 'right' ? 300 : 250)) + 'px';
    const built: any = [];
    visibleSpecs.forEach((spec: any) => {
      const p = buildPanel(spec, dock);
      if (!p) return;
      if (built.length) {
        const prev = built[built.length - 1];
        el.appendChild(hSplit(prev.spec, prev.el, spec, p));
      }
      el.appendChild(p);
      built.push({ spec, el: p });
    });
    if (prevVisible) root.appendChild(vSplit(prevVisible, dock));
    root.appendChild(el);
    prevVisible = dock;
  });
  applyTheme(ws.theme || {});
  document.documentElement.dataset.density = ws.density || 'normal';
  PM.bus.emit('layout');
  PM.$$('#pm-persist-park').forEach((x: any) => x.remove());
  if (park.childNodes.length) {
    park.id = 'pm-persist-park';
  } else {
    park.remove();
  }
  /* Two frames: first after DOM insert, second after flex layout resolves. */
  window.requestAnimationFrame(() => {
    PM.bus.emit('layout:applied');
    window.requestAnimationFrame(() => { PM.bus.emit('layout:applied'); PM.invalidate(); });
  });
};

function applyTheme(t: any) {
  const r = document.documentElement.style;
  const map: any = {
    accent: '--accent', bg: '--bg-window', panel: '--bg-panel', text: '--tx',
    line: '--line', radius: null, font: '--f-ui', mono: '--f-mono',
  };
  ['accent', 'bg', 'panel', 'text', 'line', 'font', 'mono'].forEach((k: any) => {
    if (t[k]) r.setProperty(map[k], t[k]); else r.removeProperty(map[k]);
  });
  if (t.accent) {
    r.setProperty('--accent-dim', hexA(t.accent, .16));
    r.setProperty('--accent-tx', t.accent);
  } else { r.removeProperty('--accent-dim'); r.removeProperty('--accent-tx'); }
  if (t.radius != null) {
    r.setProperty('--r-lg', t.radius + 'px');
    r.setProperty('--r-md', Math.max(2, t.radius - 3) + 'px');
    r.setProperty('--r-sm', Math.max(2, t.radius - 5) + 'px');
  } else { r.removeProperty('--r-lg'); r.removeProperty('--r-md'); r.removeProperty('--r-sm'); }
}
function hexA(hex: any, a: any) {
  const [r, g, b] = PM.hex2rgb(hex);
  return `rgba(${Math.round(r * 255)},${Math.round(g * 255)},${Math.round(b * 255)},${a})`;
}
L.applyTheme = applyTheme;

/* Refresh a single panel body without rebuilding the whole layout. */
L.refresh = (id: any) => {
  const inst = PM.panelInst[id];
  if (!inst || !inst.el || !inst.el.isConnected) return;
  inst.body.textContent = '';
  try { inst.def.build && inst.def.build(inst.body, inst); } catch (e) { console.error(e); }
  inst.def.header && inst.def.header(inst.header, inst);
};

/* ── pop-out panels ────────────────────────────────────── */
/* Opens a real child window while retaining one authoritative live panel in the main
   document. WebKit does not permit adopting DOM nodes across separate WKWebViews, so
   the child is a synchronized interaction mirror: its controls forward to the hidden
   source panel and a MutationObserver reflects source updates back into the child.
   Canvas panels (viewer/timeline) stay docked. */
PM.Popout = {
  wins: {},
  openIds() { return Object.keys(this.wins).filter((id: any) => this.isOpen(id)); },
  closeAll() { this.openIds().forEach((id: any) => this.dock(id)); },
  isOpen(id: any) { return !!this.wins[id] && !this.wins[id].window.closed; },
  open(id: any, { silent = false }: any = {}) {
    if ((window as any).powermove) {
      if (!silent) PM.toast('Pop-out windows are not available in this build');
      return;
    }
    PM.closeMenus?.();
    const def = PM.PANELS[id]; if (!def) return;
    if (NO_POPOUT.has(id)) { PM.toast(def.title + ' stays docked'); return; }
    const inst = PM.panelInst[id];
    if (!inst || !inst.el) { PM.toast('Panel not mounted'); return; }
    if (this.isOpen(id)) { this.wins[id].window.focus(); return true; }

    const w = window.open('', 'pm-popout-' + id, 'width=480,height=620,left=160,top=120');
    if (!w) { PM.toast('Pop-out blocked'); return false; }
    this.wins[id] = { window: w, timer: 0 };
    const el = inst.el;

    const setup = async () => {
      if (w.closed) return;
      const d = w.document;
      if (!d || !d.body) { window.setTimeout(setup, 30); return; }
      d.title = def.title + ' — Powermove';
      (window as any).webkit?.messageHandlers?.pmPanelTitle?.postMessage(def.title);
      /* inline every stylesheet rule so the popout needs no file access */
      let css = 'html,body{height:100%;margin:0;overflow:hidden;background:var(--bg-panel)}';
      for (const ss of document.styleSheets) {
        if (ss.href) {
          try {
            const response = await window.fetch(ss.href);
            if (response.ok) { css += await response.text(); continue; }
          } catch (e) { }
        }
        try { for (const r of ss.cssRules) css += r.cssText + '\n'; } catch (e) { }
      }
      const st = d.createElement('style'); st.textContent = css; d.head.appendChild(st);
      d.documentElement.dataset.density = document.documentElement.dataset.density;
      d.documentElement.dataset.theme = document.documentElement.dataset.theme;
      d.documentElement.style.cssText = document.documentElement.style.cssText;
      d.body.style.cssText = 'display:flex;flex-direction:column;background:var(--bg-panel)';

      /* Preserve the structured layout while freeing the panel's entire dock slot.
         The original remains the single hidden owner; L.apply omits detached IDs,
         so neighboring content reflows without mutating the saved manifest. */
      const sourceHost = document.createElement('div');
      sourceHost.hidden = true; sourceHost.setAttribute('aria-hidden', 'true');
      sourceHost.dataset.popoutSource = id;
      if (el.isConnected) sourceHost.appendChild(el);
      document.body.appendChild(sourceHost);
      const mirrorHost = d.createElement('div'); mirrorHost.className = 'pop-mirror'; d.body.appendChild(mirrorHost);

      const pathTo = (root: any, node: any) => {
        const path = [];
        while (node && node !== root) {
          const parent = node.parentNode; if (!parent) return null;
          path.unshift(Array.prototype.indexOf.call(parent.childNodes, node)); node = parent;
        }
        return node === root ? path : null;
      };
      const atPath = (root: any, path: any) => path && path.reduce((node: any, index: any) => node && node.childNodes[index], root);
      const resizeMirroredTextarea = (input: any) => {
        if (!input?.matches?.('textarea[data-autosize="true"]')) return;
        input.style.height = 'auto';
        const style = w.getComputedStyle(input);
        const min = Math.max(0, Number.parseFloat(style.minHeight) || 0);
        const maxValue = Number.parseFloat(style.maxHeight);
        const max = Number.isFinite(maxValue) ? Math.max(min, maxValue) : Number.POSITIVE_INFINITY;
        const contentHeight = Math.max(0, Math.ceil(input.scrollHeight || 0));
        const height = Math.min(max, Math.max(min, contentHeight));
        input.style.height = `${height}px`;
        input.style.overflowY = contentHeight > height + 1 ? 'auto' : 'hidden';
      };
      let syncing = false;
      let suppressInputMirror = false;
      let releaseInputMirrorFrame = 0;
      const renderMirror = () => {
        if (syncing || w.closed) return;
        syncing = true;
        const oldClone = mirrorHost.firstChild;
        const active: any = d.activeElement;
        const focusedPath = oldClone && oldClone.contains(active) ? pathTo(oldClone, active) : null;
        const selection = focusedPath && 'selectionStart' in active
          ? { start: active.selectionStart, end: active.selectionEnd, direction: active.selectionDirection, scrollTop: active.scrollTop }
          : null;
        const clone = el.cloneNode(true); clone.classList.add('popped');
        mirrorHost.replaceChildren(clone);
        clone.querySelectorAll('textarea[data-autosize="true"]').forEach(resizeMirroredTextarea);
        const nextActive = focusedPath ? atPath(clone, focusedPath) : null;
        if (nextActive?.focus) {
          try { nextActive.focus({ preventScroll: true }); } catch { nextActive.focus(); }
          if (selection && nextActive.setSelectionRange) {
            nextActive.setSelectionRange(selection.start, selection.end, selection.direction || 'none');
            nextActive.scrollTop = selection.scrollTop;
          }
        }
        syncing = false;
      };
      const forward = (event: any) => {
        const clone = mirrorHost.firstChild;
        const target = atPath(el, pathTo(clone, event.target));
        if (!target) return;
        if ('value' in event.target && 'value' in target) target.value = event.target.value;
        if ('checked' in event.target && 'checked' in target) target.checked = event.target.checked;
        if (event.type === 'input' && event.target.matches?.('textarea[data-autosize="true"]')) {
          /* The source has the current draft now. Its only input listener sizes
             the hidden copy, so dispatching another input is unnecessary and
             lets WebKit rebuild the mirror between individual keystrokes. */
          suppressInputMirror = true;
          if (releaseInputMirrorFrame) w.cancelAnimationFrame(releaseInputMirrorFrame);
          releaseInputMirrorFrame = w.requestAnimationFrame(() => { suppressInputMirror = false; releaseInputMirrorFrame = 0; });
          resizeMirroredTextarea(event.target);
          return;
        }
        if (event.type === 'input') resizeMirroredTextarea(event.target);
        if (event.type === 'click' && typeof target.click === 'function') target.click();
        else target.dispatchEvent(new window.Event(event.type, { bubbles: true, cancelable: true }));
      };
      mirrorHost.addEventListener('click', forward);
      mirrorHost.addEventListener('input', forward);
      mirrorHost.addEventListener('change', forward);
      const belongsToAutosizingTextarea = (record: any) => {
        const node = record.target?.nodeType === 3 ? record.target.parentElement : record.target;
        return !!node?.closest?.('textarea[data-autosize="true"]');
      };
      const observer = new window.MutationObserver((records: any) => {
        /* WebKit may expose a forwarded textarea value/style update as a DOM
           mutation. The visible clone already owns that draft and its height;
           rebuilding it here would eject the caret after the first character. */
        if (suppressInputMirror || (records.length && records.every(belongsToAutosizingTextarea))) return;
        window.requestAnimationFrame(renderMirror);
      });
      observer.observe(el, { subtree: true, childList: true, characterData: true, attributes: true });
      renderMirror();
      Object.assign(PM.Popout.wins[id], { sourceHost, mirrorHost, observer });
      if (L.ws) L.apply(L.ws);
      PM.toast(def.title + ' popped out');

      /* watch for the user closing the OS window */
      const timer = window.setInterval(() => {
        if (w.closed) { window.clearInterval(timer); PM.Popout.reclaim(id); }
      }, 350);
      if (PM.Popout.wins[id]) PM.Popout.wins[id].timer = timer;
    };
    setup();
    return true;
  },
  /* element returns to the docked layout (window already closed or closing) */
  reclaim(id: any) {
    const entry = this.wins[id];
    if (!entry) return false;
    window.clearInterval(entry.timer);
    const inst = PM.panelInst[id];
    delete this.wins[id];
    entry.observer?.disconnect();
    entry.sourceHost?.remove();
    if (inst && inst.el) inst.el.classList.remove('popped');
    if (L.ws) L.apply(L.ws);
    PM.toast((PM.PANELS[id]?.title || 'Panel') + ' returned to the layout');
    return true;
  },
  /* user pressed the in-popout dock button */
  dock(id: any) {
    const entry = this.wins[id];
    if (!entry) return false;
    const w = entry.window;
    window.clearInterval(entry.timer);
    delete this.wins[id];
    entry.observer?.disconnect();
    entry.sourceHost?.remove();
    const inst = PM.panelInst[id];
    if (inst && inst.el) inst.el.classList.remove('popped');
    if (L.ws) L.apply(L.ws);
    if (w && !w.closed) w.close();
    return true;
  },
};
}
