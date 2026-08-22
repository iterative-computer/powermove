/* Powermove — command registry, After Effects-style keymap, command palette. */
(() => {
const PM = window.PM, h = PM.h;

const C = {};
PM.commands = C;
const def = (id, label, kb, run, cat = 'General') => { C[id] = { id, label, kb, run, cat }; };
PM.cmd = (id, ...a) => { const c = C[id]; if (c) return c.run(...a); console.warn('no cmd', id); };

const center = () => ({ 'position.x': PM.proj.w / 2, 'position.y': PM.proj.h / 2 });
function addLayer(type, opts = {}) {
  const from = opts.from == null ? PM.snapF(PM.time, PM.proj.fps) : opts.from;
  const result = PM.Edit.apply({
    type: 'add_layer', layerType: type, name: opts.name,
    from, duration: opts.dur == null ? Math.max(1, PM.proj.dur - from) : opts.dur,
    content: opts.d || {}, properties: opts.p || {}, color: opts.color, select: true,
  }, { label: 'New ' + type, origin: 'command' });
  const id = result.ok && result.data.results[0].data.id;
  return id ? PM.L(id) : null;
}
PM.addLayerCmd = addLayer;

/* ── layer creation ────────────────────────────────────── */
def('newText', 'New text layer', '⌘T', () => addLayer('text', { name: 'Headline', p: center() }), 'Create');
def('newSolid', 'New solid', '⌘Y', () => addLayer('solid', { name: 'Solid' }), 'Create');
def('newShape', 'New shape', '⌘⇧Y', () => addLayer('shape', { name: 'Shape', p: center() }), 'Create');
def('newShader', 'New shader layer', '⌘⇧G', () => { const L = addLayer('shader', { name: 'Shader' }); PM.syncShaderUniforms(L); PM.openShaderEditor(L); return L; }, 'Create');
def('newNull', 'New null object', '⌘⌥⇧Y', () => addLayer('null', { name: 'Null', p: center() }), 'Create');
def('import', 'Import media…', '⌘I', () => PM.pickFiles(), 'Create');
def('toolSelect', 'Selection tool', 'V', () => PM.setTool('select'), 'Tool');
def('toolHand', 'Hand tool', 'H', () => PM.setTool('hand'), 'Tool');
def('toolZoom', 'Zoom tool', 'Z', () => PM.setTool('zoom'), 'Tool');
def('addFromAsset', 'Add layer from asset', null, (id) => {
  const a = PM.proj.assets[id]; if (!a) return;
  const type = a.kind === 'audio' ? 'audio' : a.kind === 'video' ? 'video' : 'image';
  const from = PM.snapF(PM.time, PM.proj.fps);
  const L = addLayer(type, {
    name: a.name, from, dur: a.dur ? Math.min(a.dur, PM.proj.dur - from) : undefined,
    d: { asset: id, w: a.w || PM.proj.w, h: a.h || PM.proj.h },
  });
  return L;
}, 'Create');

/* ── editing ───────────────────────────────────────────── */
def('duplicate', 'Duplicate layers', '⌘D', () => PM.hist.do('Duplicate', () => {
  const sels = PM.selLayers(); if (!sels.length) return;
  const ids = [];
  sels.forEach(L => { const c = PM.cloneLayer(L); PM.proj.layers.splice(PM.proj.layers.indexOf(L), 0, c); ids.push(c.id); });
  PM.bus.emit('layers'); PM.selectLayers(ids);
}), 'Edit');
def('delete', 'Delete layers', '⌫', () => {
  if (!PM.sel.keys.length) return PM.Edit.apply({ type: 'delete_layers', targets: PM.sel.layers }, { label: 'Delete', origin: 'command' });
  return PM.hist.do('Delete', () => {
  PM.selLayers().forEach(L => PM.allProps(L).forEach(p => { p.prop.kf = p.prop.kf.filter(k => !PM.sel.keys.some(s => s.i === k.i)); }));
  PM.sel.keys = []; PM.touch();
  });
}, 'Edit');
def('split', 'Split at playhead', '⌘⇧D', () => PM.hist.do('Split', () => {
  PM.selLayers().forEach(L => {
    if (PM.time <= L.from || PM.time >= L.from + L.dur) return;
    const c = PM.cloneLayer(L);
    c.from = PM.time; c.dur = L.from + L.dur - PM.time;
    L.dur = PM.time - L.from;
    PM.proj.layers.splice(PM.proj.layers.indexOf(L), 0, c);
  });
  PM.bus.emit('layers');
}), 'Edit');
def('selectAll', 'Select all layers', '⌘A', () => PM.selectLayers(PM.proj.layers.map(l => l.id)), 'Edit');
def('deselect', 'Deselect', '⎋', () => { PM.selectLayers([]); PM.sel.keys = []; }, 'Edit');
def('precompose', 'Precompose selected layers…', '⌘⇧C', () => {
  const sels = PM.selLayers(); if (!sels.length) return PM.toast('Select layers to precompose');
  const name = h('input', { value: 'Precomp' });
  PM.modal({ title: 'Precompose ' + sels.length + (sels.length === 1 ? ' layer' : ' layers'), body: h('div.field', name), width: 400, actions: [
    { label: 'Cancel' },
    { label: 'Create', pri: true, run: () => PM.hist.do('Precompose', () => PM.precompose(sels.map(l => l.id), name.value.trim() || undefined)) },
  ] });
  setTimeout(() => { name.focus(); name.select(); }, 30);
}, 'Edit');

/* ── layer clipboard ───────────────────────────────────── */
let layerClip = null;
def('copyLayers', 'Copy layers', '⌘C', () => {
  const sels = PM.selLayers(); if (!sels.length) return;
  layerClip = sels.map(L => JSON.parse(JSON.stringify(L)));
  PM.toast(`Copied ${layerClip.length} ${layerClip.length === 1 ? 'layer' : 'layers'}`);
}, 'Edit');
def('pasteLayers', 'Paste layers', '⌘V', () => {
  if (!layerClip || !layerClip.length) return;
  PM.hist.do('Paste layers', () => {
    /* clones keep their relative stack order; parenting inside the set survives */
    const pairs = layerClip.map(src => [src, PM.cloneLayer(JSON.parse(JSON.stringify(src)))]);
    const map = new Map(pairs.map(([src, c]) => [src.id, c]));
    pairs.forEach(([src, c]) => { c.parent = src.parent && map.has(src.parent) ? map.get(src.parent).id : null; });
    const pasted = pairs.map(([, c]) => c);
    for (let i = pasted.length - 1; i >= 0; i--) PM.addLayer(pasted[i], 0);
    PM.selectLayers(pasted.map(c => c.id));
  });
  PM.toast(`Pasted ${layerClip.length} ${layerClip.length === 1 ? 'layer' : 'layers'}`);
}, 'Edit');
def('undo', 'Undo', '⌘Z', () => PM.hist.undo(), 'Edit');
def('redo', 'Redo', '⌘⇧Z', () => PM.hist.redo(), 'Edit');

/* ── transport ─────────────────────────────────────────── */
def('play', 'Play / pause', '␣', () => PM.toggle(), 'Transport');
def('gotoStart', 'Go to start', '⇱', () => PM.setTime(0), 'Transport');
def('gotoEnd', 'Go to end', '⇲', () => PM.setTime(PM.proj.dur), 'Transport');
def('nextFrame', 'Next frame', '→', () => PM.step(1), 'Transport');
def('prevFrame', 'Previous frame', '←', () => PM.step(-1), 'Transport');
def('nextEdge', 'Next edge', '⇧→', () => PM.setTime(PM.TL.nextEdge()), 'Transport');
def('prevEdge', 'Previous edge', '⇧←', () => PM.setTime(PM.TL.prevEdge()), 'Transport');
def('workIn', 'Work area in', 'B', () => PM.Edit.apply({ type: 'set_composition', patch: { workArea: [Math.min(PM.time, PM.proj.work[1] - 1 / PM.proj.fps), PM.proj.work[1]] } }, { label: 'Work area', origin: 'command' }), 'Transport');
def('workOut', 'Work area out', 'N', () => PM.Edit.apply({ type: 'set_composition', patch: { workArea: [PM.proj.work[0], Math.max(PM.time, PM.proj.work[0] + 1 / PM.proj.fps)] } }, { label: 'Work area', origin: 'command' }), 'Transport');
def('marker', 'Add marker', '*', () => PM.Edit.apply({ type: 'add_marker', time: PM.time }, { label: 'Marker', origin: 'command' }), 'Transport');

/* ── reveal properties (AE muscle memory) ──────────────── */
const reveal = (keys) => () => {
  const sels = PM.selLayers(); if (!sels.length) return;
  sels.forEach(L => { L.collapsed = false; L._reveal = keys; });
  PM.sel.chan = keys[0];
  PM.invalidate('timeline');
};
def('revealPos', 'Reveal position', 'P', reveal(['position.x', 'position.y']), 'Reveal');
def('revealScale', 'Reveal scale', 'S', reveal(['scale.x', 'scale.y']), 'Reveal');
def('revealRot', 'Reveal rotation', 'R', reveal(['rotation']), 'Reveal');
def('revealOpacity', 'Reveal opacity', 'T', reveal(['opacity']), 'Reveal');
def('revealAnchor', 'Reveal anchor point', 'A', reveal(['anchor.x', 'anchor.y']), 'Reveal');
def('revealKeys', 'Reveal animated properties', 'U', () => {
  PM.selLayers().forEach(L => { L.collapsed = false; L._reveal = null; });
  PM.invalidate('timeline');
}, 'Reveal');
def('graph', 'Toggle graph editor', 'G', () => { PM.TL.graph = !PM.TL.graph; PM.invalidate('timeline'); }, 'Reveal');

/* ── keyframes ─────────────────────────────────────────── */
def('easeOut', 'Easy ease keys', 'F9', () => PM.hist.do('Easy ease', () => {
  const keys = PM.sel.keys.length ? PM.sel.keys : allSelKeys();
  PM.applyEaseTo(keys, 'easeInOut'); PM.invalidate();
}), 'Keyframes');
def('easePower', 'Powermove curve', '⇧F9', () => PM.hist.do('Power ease', () => {
  PM.applyEaseTo(PM.sel.keys.length ? PM.sel.keys : allSelKeys(), 'power'); PM.invalidate();
}), 'Keyframes');
def('easeLinear', 'Linear keys', '⌘⇧F9', () => PM.hist.do('Linear', () => {
  PM.applyEaseTo(PM.sel.keys.length ? PM.sel.keys : allSelKeys(), 'linear'); PM.invalidate();
}), 'Keyframes');
function allSelKeys() {
  const out = [];
  PM.selLayers().forEach(L => PM.allProps(L).forEach(p => out.push(...p.prop.kf)));
  return out;
}

/* ── view / files ──────────────────────────────────────── */
def('fitView', 'Fit composition in view', '⇧F', () => { PM.Viewer.fit = true; PM.Viewer.layout(); PM.TL.frameView(); }, 'View');
def('palette', 'Command palette', '⌘K', () => palette(), 'View');
def('save', 'Save project', '⌘S', () => PM.saveProject(), 'File');
def('open', 'Open project…', '⌘O', () => PM.openProject(), 'File');
def('export', 'Export…', '⌘E', () => PM.Export.dialog(), 'File');
def('projects', 'Projects screen', '⌘P', () => PM.ProjectsScreen && PM.ProjectsScreen.toggle(), 'File');
def('newProject', 'New project', '⌘N', () => PM.newProject(), 'File');
def('takeSave', 'Save take', '⌘⇧S', () => { PM.takes.save(); PM.toast('Take saved'); }, 'File');
def('focusChat', 'Toggle assistant', '⌘L', () => {
  if (PM.ChatRail.isOpen) {
    const inp = PM.$('#cinput');
    if (document.activeElement === inp) PM.ChatRail.close();
    else if (inp) { PM.ChatRail.open(); inp.focus(); }
    else PM.ChatRail.open();
  } else PM.ChatRail.open();
}, 'View');

/* ── keymap ────────────────────────────────────────────── */
const isField = (e) => {
  const t = e.target;
  return t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
};
addEventListener('keydown', (e) => {
  if (isField(e)) {
    /* The assistant toggle must still work while its own composer has focus. */
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'l') {
      e.preventDefault(); PM.cmd('focusChat');
      return;
    }
    if (e.key === 'Escape') e.target.blur();
    return;
  }
  const m = e.metaKey || e.ctrlKey, s = e.shiftKey, a = e.altKey;
  const k = e.key;
  const go = (id) => { e.preventDefault(); PM.cmd(id); };

  if (m && k.toLowerCase() === 'k') return go('palette');
  if (m && k.toLowerCase() === 'z') return go(s ? 'redo' : 'undo');
  if (m && k.toLowerCase() === 'y' && !s && !a) return go('newSolid');
  if (m && s && k.toLowerCase() === 'y') return go('newShape');
  if (m && k.toLowerCase() === 't') return go('newText');
  if (m && s && k.toLowerCase() === 'g') return go('newShader');
  if (m && k.toLowerCase() === 'd') return go(s ? 'split' : 'duplicate');
  if (m && k.toLowerCase() === 'c') return go(s ? 'precompose' : 'copyLayers');
  if (m && k.toLowerCase() === 'v' && !s) return go('pasteLayers');
  if (m && k.toLowerCase() === 'a') return go('selectAll');
  if (m && k.toLowerCase() === 'i') return go('import');
  if (m && k.toLowerCase() === 's') return go(s ? 'takeSave' : 'save');
  if (m && k.toLowerCase() === 'o') return go('open');
  if (m && k.toLowerCase() === 'e') return go('export');
  if (m && k.toLowerCase() === 'p' && !s) return go('projects');
  if (m && k.toLowerCase() === 'n') return go('newProject');
  if (m && k.toLowerCase() === 'l') return go('focusChat');  if (k === 'F9') return go(m ? 'easeLinear' : s ? 'easePower' : 'easeOut');

  switch (k) {
    case ' ': return go('play');
    case 'Home': return go('gotoStart');
    case 'End': return go('gotoEnd');
    case 'ArrowRight': return go(s ? 'nextEdge' : 'nextFrame');
    case 'ArrowLeft': return go(s ? 'prevEdge' : 'prevFrame');
    case 'Backspace': case 'Delete': return go('delete');
    case 'Escape': return go('deselect');
  }
  if (a || m) return;
  switch (k.toLowerCase()) {
    case 'v': return go('toolSelect');
    case 'h': return go('toolHand');
    case 'z': return go('toolZoom');
    case 'p': return go('revealPos');
    case 's': return go('revealScale');
    case 'r': return go('revealRot');
    case 't': return go('revealOpacity');
    case 'a': return go('revealAnchor');
    case 'u': return go('revealKeys');
    case 'g': return go('graph');
    case 'b': return go('workIn');
    case 'n': return go('workOut');
    case 'f': if (s) return go('fitView'); break;
    case '*': return go('marker');
    case 'j': e.preventDefault(); return PM.setTime(PM.TL.prevEdge());
    case 'k': e.preventDefault(); return PM.pause();
    case 'l': e.preventDefault(); return PM.play();
    case 'i': e.preventDefault(); return PM.hist.do('Trim in', () => PM.selLayers().forEach(L => { const d = PM.time - L.from; L.dur -= d; L.from = PM.time; }));
    case 'o': e.preventDefault(); return PM.hist.do('Trim out', () => PM.selLayers().forEach(L => { L.dur = Math.max(1 / PM.proj.fps, PM.time - L.from); }));
  }
});

/* ── command palette ───────────────────────────────────── */
function palette() {
  PM.closeMenus();
  const scrim = PM.$('#scrim'); scrim.classList.add('on');
  const inp = h('input', { placeholder: 'Search commands, layers, workspaces…' });
  const list = h('div.plist');
  const el = h('div.modal#palette', h('div.field', inp), list);
  document.body.appendChild(el);
  const close = () => { el.remove(); scrim.classList.remove('on'); scrim.onclick = null; };
  scrim.onclick = close;

  let items = [], sel = 0;
  const build = (q) => {
    q = q.toLowerCase().trim();
    items = [];
    Object.values(C).forEach(c => { if (!q || c.label.toLowerCase().includes(q)) items.push({ label: c.label, cat: c.cat, kb: c.kb, run: () => PM.cmd(c.id) }); });
    PM.proj.layers.forEach(L => { if (q && L.name.toLowerCase().includes(q)) items.push({ label: L.name, cat: 'Layer', run: () => PM.selectLayers(L.id) }); });
    PM.WS.list().forEach(w => { if (!q || w.name.toLowerCase().includes(q)) items.push({ label: 'Workspace · ' + w.name, cat: 'Workspace', run: () => PM.WS.activate(w.id) }); });
    Object.entries(PM.FX).forEach(([k, d]) => { if (q && d.label.toLowerCase().includes(q)) items.push({ label: 'Effect · ' + d.label, cat: 'Effect', run: () => { const L = PM.firstSel(); if (L) { PM.hist.do('Add effect', () => L.fx.push(PM.mkEffect(k))); PM.Inspector.refresh(); PM.invalidate(); } } }); });
    items = items.slice(0, 60);
    sel = 0; paint();
  };
  const paint = () => {
    list.textContent = '';
    items.forEach((it, i) => {
      const row = h('div.pitem' + (i === sel ? '.on' : ''), h('span.cat', it.cat), h('span', it.label), it.kb ? h('span.kb', it.kb) : null);
      row.onclick = () => { close(); it.run(); };
      list.appendChild(row);
    });
    if (!items.length) list.appendChild(h('div.empty', 'No matches'));
  };
  inp.addEventListener('input', () => build(inp.value));
  inp.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'ArrowDown') { sel = Math.min(items.length - 1, sel + 1); paint(); e.preventDefault(); }
    if (e.key === 'ArrowUp') { sel = Math.max(0, sel - 1); paint(); e.preventDefault(); }
    if (e.key === 'Enter') { const it = items[sel]; close(); it && it.run(); }
    if (e.key === 'Escape') close();
  });
  build('');
  inp.focus();
}
PM.palette = palette;
})();
