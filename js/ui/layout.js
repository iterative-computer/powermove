/* Powermove — dock layout engine. The workspace JSON is the single source of UI truth.
   Panels are resizable (splitters), movable (drag header between docks), and can be
   popped out into their own native window via the panelWindow bridge. */
(() => {
const PM = window.PM, h = PM.h, $ = PM.$;

PM.PANELS = {};      // id -> {title, icon, build(body, panel), header(el), persist}
PM.panelInst = {};   // id -> {el, body, def, cache, spec}

PM.registerPanel = (id, def) => { PM.PANELS[id] = Object.assign({ id, title: id }, def); };

const L = { root: null, ws: null };
PM.Layout = L;

/* Panels that own live GL/canvas state and cannot be safely re-hosted in a popout. */
const NO_POPOUT = new Set(['viewer', 'timeline']);

/* ── panel construction ────────────────────────────────── */
function buildPanel(spec, dock) {
  const def = PM.PANELS[spec.id];
  if (!def) return null;
  const inst = PM.panelInst[spec.id] || (PM.panelInst[spec.id] = { def, cache: null });
  /* Reuse the live panel element for persist canvases so apply() never destroys
     the timeline/viewer backing store (the source of clip/gutter misalignment). */
  if (def.persist && inst.el && inst.built) {
    inst.spec = spec; inst.dock = dock;
    applyPanelSize(inst.el, spec, def);
    inst.el.style.minHeight = (spec.min || 56) + 'px';
    inst.el.dataset.collapsed = '0';
    if (inst.body) inst.body.style.display = '';
    return inst.el;
  }
  const headless = !!(spec.headless || def.headless);
  const cls = '.panel' + (def.flush ? '.flush' : '') + (def.noscroll ? '.noscroll' : '') + (headless ? '.headless' : '');
  const el = h('div' + cls, { id: 'panel-' + spec.id });
  el.dataset.panel = spec.id;

  const hdr = h('header');
  const grip = h('span.grip', { title: 'Drag to move · right-click for options' }, PM.icon('grip'));
  const title = h('span.ptitle', spec.title || def.title);
  hdr.append(grip, title, h('span.sp'));
  el.appendChild(hdr);

  let body = h('div.body');
  if (def.persist && inst.cache) body = inst.cache;
  el.appendChild(body);

  /* Headless canvas panels still need an explicit, non-canvas drag surface.
     Keeping it small prevents panel movement from stealing timeline/stage input. */
  const moveHandle = headless
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
    try { def.build && def.build(body, inst); } catch (e) { console.error('[panel]' + spec.id, e); body.appendChild(h('div.empty', 'Panel error: ' + e.message)); }
    inst.built = true;
  }
  if (moveHandle && def.moveSlot) {
    const slot = body.querySelector(def.moveSlot);
    if (slot) { moveHandle.classList.add('inline'); slot.insertBefore(moveHandle, slot.firstChild); }
  }
  try { def.header && def.header(hdr, inst); } catch (e) { }
  if (!hdr.querySelector('.grip')) hdr.insertBefore(grip, hdr.firstChild);

  const liveLocation = () => (L.ws && findPanel(L.ws, spec.id)) || { spec: inst.spec || spec, dock: inst.dock || dock };
  const beginMove = (e) => {
    if (e.button !== 0) return;
    const current = liveLocation();
    if (!current || !current.dock || !current.spec) return;
    e.stopPropagation();
    startPanelDrag(e, current.spec, current.dock, el);
  };

  /* Header interactions use the live workspace location. Persisted canvas panels
     keep their DOM nodes while moving, so captured initial dock objects go stale. */
  hdr.addEventListener('pointerdown', (e) => {
    if (e.target.closest('button')) return;
    beginMove(e);
  });
  hdr.addEventListener('dblclick', (e) => {
    if (e.target.closest('button')) return;
    const current = liveLocation();
    const liveSpec = current && current.spec ? current.spec : spec;
    const collapsed = el.dataset.collapsed === '1';
    el.dataset.collapsed = collapsed ? '0' : '1';
    body.style.display = collapsed ? '' : 'none';
    el.style.flex = collapsed ? (liveSpec.flex ? '1 1 auto' : '0 0 ' + (liveSpec.size || def.size) + 'px') : '0 0 var(--hdr-h)';
    if (!collapsed) PM.bus.emit('layout:applied');
  });
  const openPanelMenu = (e) => {
    e.preventDefault(); e.stopPropagation();
    const current = liveLocation();
    if (current && current.dock && current.spec) panelMenu(e, current.spec, current.dock);
  };
  hdr.addEventListener('contextmenu', openPanelMenu);
  if (moveHandle) {
    moveHandle.addEventListener('pointerdown', beginMove);
    moveHandle.addEventListener('contextmenu', openPanelMenu);
  }
  return el;
}

function applyPanelSize(el, spec, def) {
  if (spec.flex || (!spec.size && !def.size)) el.style.flex = '1 1 auto';
  else el.style.flex = '0 0 ' + (spec.size || def.size) + 'px';
}

/* ── drag to move panels between docks ─────────────────── */
let dragState = null;
function startPanelDrag(e, spec, dock, el) {
  const rect = el.getBoundingClientRect();
  const ghost = h('div.panel-ghost', h('span', PM.PANELS[spec.id].title));
  ghost.style.width = rect.width + 'px';
  ghost.style.height = Math.min(rect.height, 120) + 'px';
  document.body.appendChild(ghost);
  dragState = { spec, fromDock: dock.id, ghost, moved: false };

  const move = (dx, dy, ev) => {
    if (!dragState.moved && Math.hypot(dx, dy) < 5) return;
    dragState.moved = true;
    ghost.classList.add('on');
    ghost.style.left = ev.clientX + 12 + 'px';
    ghost.style.top = ev.clientY + 12 + 'px';
    updateDropTarget(ev);
  };
  const up = (dx, dy, ev) => {
    ghost.remove();
    clearDropHints();
    const target = dragState.dropTarget;
    const wasMoved = dragState.moved;
    dragState = null;
    if (!wasMoved) return;
    if (target) {
      PM.WS.mutate(w => {
        removePanel(w, spec.id);
        insertPanel(w, spec, target.dockId, target.index);
      });
      PM.toast('Moved ' + PM.PANELS[spec.id].title + ' → ' + target.dockId);
    }
  };
  PM.drag(e, { move, up, cursor: 'grabbing' });
}

function updateDropTarget(ev) {
  clearDropHints();
  dragState.dropTarget = null;
  const el = document.elementFromPoint(ev.clientX, ev.clientY);
  if (!el) return;
  const dockEl = el.closest('.dock');
  if (!dockEl) return;
  const dockId = dockEl.id.replace('dock-', '');
  /* find insertion index from hovered panel */
  const panelEl = el.closest('.panel');
  let index = null;
  if (panelEl && dockEl.contains(panelEl)) {
    const panels = [...dockEl.querySelectorAll(':scope > .panel')];
    const i = panels.indexOf(panelEl);
    const r = panelEl.getBoundingClientRect();
    const before = ev.clientY < r.top + r.height / 2;
    index = before ? i : i + 1;
    panelEl.classList.add(before ? 'drop-before' : 'drop-after');
  } else {
    dockEl.classList.add('drop-into');
    index = dockEl.querySelectorAll(':scope > .panel').length;
  }
  dragState.dropTarget = { dockId, index };
}
function clearDropHints() {
  PM.$$('.drop-before,.drop-after,.drop-into').forEach(x => x.classList.remove('drop-before', 'drop-after', 'drop-into'));
}

/* ── panel context menu ────────────────────────────────── */
function panelMenu(e, spec, dock) {
  const ws = L.ws;
  const def = PM.PANELS[spec.id];
  const items = [
    { header: def.title },
    !NO_POPOUT.has(spec.id) ? { label: 'Pop out to window', run: () => PM.Popout.open(spec.id) } : null,
    { label: 'Hide panel', run: () => PM.WS.mutate(w => { removePanel(w, spec.id); }) },
    '-',
    { header: 'Move to' },
    { label: 'Left dock', run: () => PM.WS.mutate(w => movePanel(w, spec.id, 'left')) },
    { label: 'Center dock', run: () => PM.WS.mutate(w => movePanel(w, spec.id, 'center')) },
    { label: 'Right dock', run: () => PM.WS.mutate(w => movePanel(w, spec.id, 'right')) },
    '-',
    { header: 'Add panel' },
    ...Object.values(PM.PANELS).filter(p => !hasPanel(ws, p.id) && p.id !== 'toolbar').map(p => ({
      label: p.title, run: () => PM.WS.mutate(w => addPanel(w, p.id, dock.id)),
    })),
  ].filter(Boolean);
  PM.menu(document.body, items, { x: e.clientX, y: e.clientY });
}

/* ── workspace model ops ───────────────────────────────── */
const eachDock = (ws, fn) => ws.layout.docks.forEach(fn);
const hasPanel = (ws, id) => ws.layout.docks.some(d => d.panels.some(p => p.id === id));
function removePanel(ws, id) { eachDock(ws, d => { d.panels = d.panels.filter(p => p.id !== id); }); }
function findPanel(ws, id) {
  for (const d of ws.layout.docks) { const s = d.panels.find(p => p.id === id); if (s) return { dock: d, spec: s }; }
  return null;
}
function addPanel(ws, id, dockId) {
  removePanel(ws, id);
  const d = ws.layout.docks.find(d => d.id === (dockId || 'right')) || ws.layout.docks[0];
  d.panels.push({ id, flex: d.panels.length === 0 });
  if (d.hidden) d.hidden = false;
}
function insertPanel(ws, spec, dockId, index) {
  const d = ws.layout.docks.find(d => d.id === dockId) || ws.layout.docks[0];
  const clean = { id: spec.id };
  if (spec.size) clean.size = spec.size;
  if (spec.flex) clean.flex = true;
  const i = index == null ? d.panels.length : Math.max(0, Math.min(index, d.panels.length));
  d.panels.splice(i, 0, clean);
  if (d.hidden) d.hidden = false;
}
function movePanel(ws, id, dockId) { addPanel(ws, id, dockId); }
L.removePanel = removePanel; L.addPanel = addPanel; L.hasPanel = hasPanel; L.findPanel = findPanel;

/* ── splitters ─────────────────────────────────────────── */
/* A splitter always resizes the nearest *fixed* pane and leaves the flex pane
   to absorb the change. Sign is chosen so dragging the bar toward a pane shrinks
   that pane — never the opposite direction. */
function isFlexDock(d) { return !d || d.id === 'center' || !!d.flex; }
function isFlexPanel(spec) {
  if (!spec) return true;
  const def = PM.PANELS[spec.id] || {};
  return !!(spec.flex || (!spec.size && !def.size));
}
function vSplit(prev, next) {
  const s = h('div.splitter');
  /* Prefer the side dock. Dragging right grows the left dock / shrinks the right dock. */
  const useLeft = !isFlexDock(prev);
  const target = useLeft ? prev : next;
  const sign = useLeft ? 1 : -1;
  s.addEventListener('pointerdown', (e) => {
    const el = document.getElementById('dock-' + target.id);
    if (!el) return;
    const start = el.getBoundingClientRect().width;
    s.classList.add('drag');
    let raf = 0;
    PM.drag(e, {
      cursor: 'col-resize',
      move: (dx) => {
        const w = PM.clamp(start + sign * dx, 200, 760);
        el.style.flex = '0 0 ' + w + 'px';
        target.size = Math.round(w);
        target.flex = false;
        if (!raf) raf = requestAnimationFrame(() => { raf = 0; PM.bus.emit('layout:applied'); });
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
function hSplit(aboveSpec, aboveEl, belowSpec, belowEl) {
  const s = h('div.splitter.h');
  const useAbove = !isFlexPanel(aboveSpec);
  const spec = useAbove ? aboveSpec : belowSpec;
  const node = useAbove ? aboveEl : belowEl;
  const sign = useAbove ? 1 : -1;
  s.addEventListener('pointerdown', (e) => {
    const start = node.getBoundingClientRect().height;
    s.classList.add('drag');
    let raf = 0;
    PM.drag(e, {
      cursor: 'row-resize',
      move: (dx, dy) => {
        const hh = PM.clamp(start + sign * dy, 72, innerHeight - 160);
        node.style.flex = '0 0 ' + hh + 'px';
        spec.size = Math.round(hh);
        delete spec.flex;
        const other = spec === aboveSpec ? belowSpec : aboveSpec;
        const otherEl = spec === aboveSpec ? belowEl : aboveEl;
        if (other && otherEl) {
          other.flex = true; delete other.size;
          otherEl.style.flex = '1 1 auto';
        }
        if (!raf) raf = requestAnimationFrame(() => { raf = 0; PM.bus.emit('layout:applied'); });
      },
      up: () => {
        s.classList.remove('drag');
        PM.WS.save();
        PM.bus.emit('layout');
        PM.bus.emit('layout:applied');
      },
    });
  });
  return s;
}

/* ── apply ─────────────────────────────────────────────── */
L.apply = (ws) => {
  L.ws = ws;
  const root = $('#body');
  /* Lift persist panels out before wiping the dock tree so their canvases
     stay alive and ResizeObservers do not record a 0×0 frame. */
  const park = document.createElement('div');
  park.style.cssText = 'position:absolute;left:-9999px;top:0;width:1px;height:1px;overflow:hidden;';
  document.body.appendChild(park);
  Object.values(PM.panelInst).forEach(inst => {
    if (inst.def && inst.def.persist && inst.el && inst.el.parentNode) park.appendChild(inst.el);
  });
  root.textContent = '';
  const docks = ws.layout.docks;
  let prevVisible = null;
  docks.forEach((dock, di) => {
    if (dock.hidden || !dock.panels.length) return;
    const el = h('div.dock.col', { id: 'dock-' + dock.id });
    el.dataset.dock = dock.id;
    if (dock.id === 'center' || dock.flex) el.style.flex = '1 1 auto';
    else el.style.flex = '0 0 ' + (dock.size || (dock.id === 'right' ? 300 : 250)) + 'px';
    const built = [];
    dock.panels.forEach((spec) => {
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
  PM.$$('#pm-persist-park').forEach(x => x.remove());
  if (park.childNodes.length) {
    park.id = 'pm-persist-park';
  } else {
    park.remove();
  }
  /* Two frames: first after DOM insert, second after flex layout resolves. */
  requestAnimationFrame(() => {
    PM.bus.emit('layout:applied');
    requestAnimationFrame(() => { PM.bus.emit('layout:applied'); PM.invalidate(); });
  });
};

function applyTheme(t) {
  const r = document.documentElement.style;
  const map = {
    accent: '--accent', bg: '--bg-window', panel: '--bg-panel', text: '--tx',
    line: '--line', radius: null, font: '--f-ui', mono: '--f-mono',
  };
  ['accent', 'bg', 'panel', 'text', 'line', 'font', 'mono'].forEach(k => {
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
function hexA(hex, a) {
  const [r, g, b] = PM.hex2rgb(hex);
  return `rgba(${Math.round(r * 255)},${Math.round(g * 255)},${Math.round(b * 255)},${a})`;
}
L.applyTheme = applyTheme;

/* Refresh a single panel body without rebuilding the whole layout. */
L.refresh = (id) => {
  const inst = PM.panelInst[id];
  if (!inst || !inst.el || !inst.el.isConnected) return;
  inst.body.textContent = '';
  try { inst.def.build && inst.def.build(inst.body, inst); } catch (e) { console.error(e); }
  inst.def.header && inst.def.header(inst.header, inst);
};

/* ── pop-out panels ────────────────────────────────────── */
/* Reparents the live panel element into a real child window opened via window.open
   (WKWebView createWebViewWith). Same JS realm, same element — all listeners, bus
   subscriptions and build state survive. Styles are inlined because the child webview
   has no file: read access. Canvas panels (viewer/timeline) stay docked. */
PM.Popout = {
  wins: {},
  open(id) {
    const def = PM.PANELS[id]; if (!def) return;
    if (NO_POPOUT.has(id)) { PM.toast(def.title + ' stays docked'); return; }
    const inst = PM.panelInst[id];
    if (!inst || !inst.el) { PM.toast('Panel not mounted'); return; }
    if (this.wins[id] && !this.wins[id].closed) { this.wins[id].focus(); return; }

    const w = window.open('', 'pm-popout-' + id, 'width=480,height=620,left=160,top=120');
    if (!w) { PM.toast('Pop-out blocked'); return; }
    this.wins[id] = w;
    const el = inst.el;

    const setup = () => {
      if (w.closed) return;
      const d = w.document;
      if (!d || !d.body) { setTimeout(setup, 30); return; }
      d.title = def.title + ' — Powermove';
      /* inline every stylesheet rule so the popout needs no file access */
      let css = 'html,body{height:100%;margin:0;overflow:hidden;background:var(--bg-panel)}';
      for (const ss of document.styleSheets) {
        try { for (const r of ss.cssRules) css += r.cssText + '\n'; } catch (e) { }
      }
      const st = d.createElement('style'); st.textContent = css; d.head.appendChild(st);
      d.documentElement.dataset.density = document.documentElement.dataset.density;
      d.body.style.cssText = 'display:flex;flex-direction:column;background:var(--bg-panel)';

      /* popout titlebar with a dock-back button */
      const bar = h('div.pop-bar',
        h('span.pop-title', def.title),
        h('span.sp'),
        h('button.iconbtn', { title: 'Dock back in main window', onclick: () => PM.Popout.dock(id) }, PM.icon('panelL')));
      d.body.appendChild(bar);

      /* move the live panel into the popout */
      el.classList.add('popped');
      d.body.appendChild(el);

      /* drop it from the docked model while popped */
      PM.WS.mutate(ws => removePanel(ws, id));
      PM.toast(def.title + ' popped out');

      /* watch for the user closing the OS window */
      const timer = setInterval(() => {
        if (w.closed) { clearInterval(timer); PM.Popout.reclaim(id); }
      }, 350);
    };
    setup();
  },
  /* element returns to the docked layout (window already closed or closing) */
  reclaim(id) {
    const inst = PM.panelInst[id];
    delete this.wins[id];
    if (inst && inst.el) inst.el.classList.remove('popped');
    PM.WS.mutate(ws => addPanel(ws, id, 'right'));
  },
  /* user pressed the in-popout dock button */
  dock(id) {
    const w = this.wins[id];
    delete this.wins[id];
    const inst = PM.panelInst[id];
    if (inst && inst.el) inst.el.classList.remove('popped');
    PM.WS.mutate(ws => addPanel(ws, id, 'right'));
    if (w && !w.closed) w.close();
  },
};
})();
