/* Powermove — inspector: content, transform, effects, shader uniforms, scene params. */
(() => {
const PM = window.PM, h = PM.h;
const I = { syncs: [] };
PM.Inspector = I;

PM.registerPanel('inspector', {
  title: 'Properties',
  build(body, inst) { I.body = body; render(); },
  header(hdr) {
    hdr.textContent = '';
    const L = PM.firstSel();
    const type = L && PM.TYPE_META[L.type];
    hdr.append(
      h('span', L ? L.name : 'Properties'),
      type ? h('span.sub', ' · ' + type.label) : h('span'),
      h('span.sp'),
      h('button.iconbtn', { title: 'New layer', onpointerdown: (e) => { e.preventDefault(); newLayerMenu(e.target); } }, PM.icon('layers')),
      h('button.iconbtn', { title: 'Add effect', onpointerdown: (e) => { e.preventDefault(); fxMenu(e.target); } }, PM.icon('plus')));
  },
});

I.refresh = () => { if (I.body && I.body.isConnected) render(); PM.Layout.refresh && refreshHeader(); };
function refreshHeader() {
  const inst = PM.panelInst.inspector;
  if (inst && inst.header && inst.header.isConnected) PM.PANELS.inspector.header(inst.header, inst);
}
PM.bus.on('sel', I.refresh);
PM.bus.on('layers', I.refresh);
PM.bus.on('history', I.refresh);
PM.bus.on('draw:ui', () => I.syncs.forEach(f => { try { f(); } catch (e) { } }));

function render() {
  const body = I.body; if (!body) return;
  body.textContent = '';
  I.syncs = [];
  const wrap = h('div.insp');
  body.appendChild(wrap);
  const L = PM.firstSel();
  if (!L) { sceneParams(wrap); return; }
  refreshHeader();
  content(wrap, L);
  transform(wrap, L);
  if (L.type === 'shader') shaderUniforms(wrap, L);
  effects(wrap, L);
  layerOptions(wrap, L);
}

/* ── rows ──────────────────────────────────────────────── */
function chRow(wrap, L, key, label) {
  const p = L.p[key];
  const meta = PM.CH[key] || {};
  const sw = h('button.stopwatch' + (p.kf.length ? '.on' : ''), { title: 'Animate ' + label },
    PM.svg('<circle cx="12" cy="12" r="7"/><path d="M12 8v4l2.5 1.5"/>'));
  sw.onclick = () => { PM.hist.do('Animate ' + label, () => PM.toggleStopwatch(L, key, PM.time)); I.refresh(); };
  const num = PM.numField(() => PM.ev(L, key, PM.time), (v) => {
    PM.setOrKey(L, key, v, PM.time); PM.invalidate();
  }, { label, step: meta.step || 1, unit: meta.unit, min: meta.min, max: meta.max, link: !!p.expr });
  I.syncs.push(num.sync);
  const kd = h('button.stopwatch', { title: 'Keyframe at playhead' }, PM.svg('<path d="M12 5l7 7-7 7-7-7z"/>'));
  const syncKd = () => {
    const on = !!PM.hasKeyAt(L, p, PM.time);
    kd.classList.toggle('on', on);
    kd.style.display = p.kf.length ? '' : 'none';
  };
  syncKd(); I.syncs.push(syncKd);
  kd.onclick = () => {
    PM.hist.do('Keyframe', () => {
      const at = PM.hasKeyAt(L, p, PM.time);
      if (at) PM.removeKey(p, at); else PM.setKeyOn(p, PM.time - L.from, PM.ev(L, key, PM.time), 'power', PM.proj.fps);
    });
    syncKd(); PM.invalidate();
  };
  const r = PM.row(label, h('div', { style: { display: 'flex', alignItems: 'center', gap: '4px' } }, num, kd), { left: sw });
  r.insertBefore(sw, r.firstChild);
  sw.style.marginRight = '2px';
  r.addEventListener('contextmenu', (e) => { e.preventDefault(); chanMenu(e, L, key, p, label); });
  r.addEventListener('pointerdown', () => { PM.sel.chan = key; PM.TL.reveal(L, [key]); });
  wrap.appendChild(r);
  return r;
}

function chanMenu(e, L, key, p, label) {
  PM.menu(document.body, [
    { header: label },
    { label: 'Add keyframe at playhead', run: () => PM.hist.do('Keyframe', () => PM.setKeyOn(p, PM.time - L.from, PM.evP(L, p, PM.time, key), 'power', PM.proj.fps)) },
    { label: 'Show in graph editor', run: () => { PM.sel.chan = key; PM.TL.graph = true; PM.TL.reveal(L, [key]); } },
    '-',
    { header: 'Easing for all keys' },
    ...['power', 'linear', 'easeInOut', 'expoOut', 'backOut', 'glide', 'snap'].map(n => ({
      label: n, run: () => PM.hist.do('Ease', () => PM.applyEaseTo(p.kf, n)),
    })),
    '-',
    { label: p.expr ? 'Edit expression…' : 'Add expression…', run: () => exprDialog(L, key, p, label) },
    p.expr ? { label: 'Remove expression', run: () => PM.hist.do('Remove expression', () => { p.expr = null; PM.touch(); I.refresh(); }) } : null,
    { label: 'Reset', run: () => PM.hist.do('Reset', () => { p.kf = []; p.expr = null; PM.touch(); I.refresh(); }) },
  ].filter(Boolean), { x: e.clientX, y: e.clientY });
}

function exprDialog(L, key, p, label) {
  const ta = h('textarea.code', { style: { height: '150px', borderRadius: '8px' } }, p.expr || 'value + wiggle(2, 20)');
  const hint = h('div', { style: { fontSize: '11px', color: 'var(--tx-3)', lineHeight: 1.6, fontFamily: 'var(--f-mono)' } },
    't · layer-local seconds   T · comp seconds   value · keyframed value',
    h('br'), 'wiggle(f,a) random(s) linear(x,x0,x1,y0,y1) ease(...) loop(d,x) param("name")');
  PM.modal({
    title: 'Expression · ' + L.name + ' · ' + label,
    body: h('div', { style: { display: 'flex', flexDirection: 'column', gap: '10px' } }, ta, hint),
    width: 540,
    actions: [{ label: 'Cancel' }, {
      label: 'Apply', pri: true, run: () => {
        PM.hist.do('Expression', () => { p.expr = ta.value.trim() || null; PM.touch(); });
        I.refresh(); PM.invalidate();
      },
    }],
  });
}

/* ── content ───────────────────────────────────────────── */
function content(wrap, L) {
  const d = L.d;
  wrap.appendChild(PM.section('Content'));
  const set = (k) => (v) => { d[k] = v; PM.touch(); PM.invalidate(); };
  const get = (k) => () => d[k];

  if (L.type === 'text') {
    const ta = h('textarea', {
      style: {
        width: '100%', minHeight: '54px', background: 'var(--bg-row)', borderRadius: 'var(--r-sm)',
        padding: '8px 10px', fontSize: '12.5px', lineHeight: 1.5, resize: 'vertical', color: 'var(--tx)',
      },
    }, d.text);
    ta.addEventListener('focus', () => PM.hist.begin('Edit text'));
    ta.addEventListener('input', () => { d.text = ta.value; PM.touch(); PM.invalidate(); });
    ta.addEventListener('blur', () => PM.hist.commit('Edit text'));
    ta.addEventListener('keydown', e => e.stopPropagation());
    I.textArea = ta;
    wrap.appendChild(ta);
    wrap.appendChild(PM.row('Font', PM.selectField(get('font'), set('font'),
      ['Geist', 'Geist Mono', 'Helvetica Neue', 'Georgia', 'Times New Roman', 'Courier New', 'Impact', 'Futura', 'Avenir Next', 'SF Pro Display'], { label: 'Font' })));
    wrap.appendChild(PM.row('Weight', PM.selectField(get('weight'), set('weight'),
      [{ v: 300, label: '300' }, { v: 400, label: '400' }, { v: 500, label: '500' }, { v: 600, label: '600' }, { v: 700, label: '700' }, { v: 800, label: '800' }], { label: 'Weight' })));
    numRow(wrap, 'Size', get('size'), set('size'), { step: 1, min: 4, unit: 'px' });
    numRow(wrap, 'Tracking', get('tracking'), set('tracking'), { step: .5, unit: 'px' });
    numRow(wrap, 'Leading', get('leading'), set('leading'), { step: .02, precision: 2 });
    wrap.appendChild(PM.row('Align', PM.selectField(get('align'), set('align'), ['left', 'center', 'right'], { label: 'Align' })));
    wrap.appendChild(PM.row('Color', PM.colorField(get('color'), set('color'), { label: 'Text color' })));
  }
  else if (L.type === 'solid' || L.type === 'shape') {
    wrap.appendChild(PM.row('Fill', PM.colorField(get('color'), set('color'), { label: 'Fill' })));
    if (L.type === 'shape') wrap.appendChild(PM.row('Shape', PM.selectField(get('shape'), set('shape'), ['rect', 'ellipse', 'polygon', 'star', 'line'], { label: 'Shape' })));
    numRow(wrap, 'Width', get('w'), set('w'), { step: 1, min: 1, unit: 'px' });
    numRow(wrap, 'Height', get('h'), set('h'), { step: 1, min: 1, unit: 'px' });
    numRow(wrap, 'Corner radius', get('radius'), set('radius'), { step: 1, min: 0, unit: 'px' });
    if (L.type === 'shape') {
      numRow(wrap, 'Stroke', get('stroke'), set('stroke'), { step: .5, min: 0, unit: 'px' });
      wrap.appendChild(PM.row('Stroke color', PM.colorField(get('strokeColor'), set('strokeColor'), { label: 'Stroke' })));
      if (d.shape === 'polygon' || d.shape === 'star') numRow(wrap, 'Points', get('points'), set('points'), { step: 1, min: 3, max: 24 });
    }
  }
  else if (L.type === 'image' || L.type === 'video') {
    const assets = Object.values(PM.proj.assets).filter(a => a.kind === (L.type === 'video' ? 'video' : 'image'));
    wrap.appendChild(PM.row('Source', PM.selectField(
      () => { const a = PM.proj.assets[d.asset]; return a ? a.name : 'none'; },
      (v) => { d.asset = v; PM.invalidate(); },
      assets.map(a => ({ v: a.id, label: a.name })).concat([{ v: null, label: 'none' }]), { label: 'Source' })));
    wrap.appendChild(PM.row('Fit', PM.selectField(get('fit'), set('fit'), ['cover', 'contain', 'stretch'], { label: 'Fit' })));
    numRow(wrap, 'Width', get('w'), set('w'), { step: 1, min: 1, unit: 'px' });
    numRow(wrap, 'Height', get('h'), set('h'), { step: 1, min: 1, unit: 'px' });
    if (L.type === 'video') {
      numRow(wrap, 'Trim start', get('trim'), set('trim'), { step: .05, precision: 2, unit: 's' });
      numRow(wrap, 'Speed', get('speed'), set('speed'), { step: .05, precision: 2, min: .05 });
    }
  }
  else if (L.type === 'audio') {
    const assets = Object.values(PM.proj.assets).filter(a => a.kind === 'audio');
    wrap.appendChild(PM.row('Source', PM.selectField(
      () => { const a = PM.proj.assets[d.asset]; return a ? a.name : 'none'; },
      (v) => { d.asset = v; }, assets.map(a => ({ v: a.id, label: a.name })), { label: 'Source' })));
    numRow(wrap, 'Gain', get('gain'), set('gain'), { step: .05, precision: 2, min: 0, max: 4 });
    numRow(wrap, 'Fade in', get('fadeIn'), set('fadeIn'), { step: .05, precision: 2, unit: 's' });
    numRow(wrap, 'Fade out', get('fadeOut'), set('fadeOut'), { step: .05, precision: 2, unit: 's' });
  }
  else if (L.type === 'shader') {
    const b = h('button.chip', { style: { width: '100%', justifyContent: 'center', height: '30px' }, onclick: () => PM.openShaderEditor(L) }, PM.icon('code'), 'Edit shader source');
    wrap.appendChild(b);
    const err = PM.GL.compileError(L._shaderKey);
    if (err) wrap.appendChild(h('div', { style: { fontFamily: 'var(--f-mono)', fontSize: '10.5px', color: 'var(--red)', padding: '6px 4px', whiteSpace: 'pre-wrap', maxHeight: '90px', overflow: 'auto' } }, err));
    numRow(wrap, 'Width', get('w'), set('w'), { step: 1, min: 1, unit: 'px' });
    numRow(wrap, 'Height', get('h'), set('h'), { step: 1, min: 1, unit: 'px' });
  }
}

function numRow(wrap, label, get, set, opt = {}) {
  const f = PM.numField(get, (v) => { set(v); }, { label, ...opt });
  I.syncs.push(f.sync);
  wrap.appendChild(PM.row(label, f));
  return f;
}

/* ── transform ─────────────────────────────────────────── */
function transform(wrap, L) {
  wrap.appendChild(PM.section('Transform'));
  chRow(wrap, L, 'position.x', 'Position X');
  chRow(wrap, L, 'position.y', 'Position Y');
  chRow(wrap, L, 'scale.x', 'Scale X');
  chRow(wrap, L, 'scale.y', 'Scale Y');
  chRow(wrap, L, 'rotation', 'Rotation');
  chRow(wrap, L, 'opacity', 'Opacity');
  chRow(wrap, L, 'anchor.x', 'Anchor X');
  chRow(wrap, L, 'anchor.y', 'Anchor Y');
  chRow(wrap, L, 'skew', 'Skew');
}

/* ── shader uniforms ───────────────────────────────────── */
function shaderUniforms(wrap, L) {
  PM.syncShaderUniforms(L);
  const defs = L._udefs || [];
  if (!defs.length) return;
  wrap.appendChild(PM.section('Shader'));
  defs.forEach(def => {
    const p = L.d.uniforms[def.name];
    if (!p) return;
    if (def.control === 'color') {
      wrap.appendChild(PM.row(def.label, PM.colorField(() => p.v, (v) => { p.v = v; PM.invalidate(); }, { label: def.label })));
    } else if (def.control === 'toggle') {
      wrap.appendChild(PM.row(def.label, PM.toggleField(() => p.v, (v) => { p.v = v; PM.invalidate(); }, { label: def.label })));
    } else {
      const sw = h('button.stopwatch' + (p.kf.length ? '.on' : ''), PM.svg('<circle cx="12" cy="12" r="7"/><path d="M12 8v4l2.5 1.5"/>'));
      sw.onclick = () => {
        PM.hist.do('Animate ' + def.label, () => {
          if (p.kf.length) { p.v = PM.evP(L, p, PM.time, def.name); p.kf = []; }
          else PM.setKeyOn(p, PM.time - L.from, p.v, 'power', PM.proj.fps);
        });
        I.refresh();
      };
      const f = PM.numField(() => PM.evP(L, p, PM.time, def.name), (v) => {
        if (p.kf.length) PM.setKeyOn(p, PM.time - L.from, v, 'power', PM.proj.fps); else p.v = v;
        PM.touch(); PM.invalidate();
      }, { label: def.label, step: (def.max - def.min) / 200 || .01, min: def.min, max: def.max, precision: 3 });
      I.syncs.push(f.sync);
      const r = PM.row(def.label, f, { left: sw });
      r.insertBefore(sw, r.firstChild);
      r.addEventListener('contextmenu', (e) => { e.preventDefault(); chanMenu(e, L, 'u.' + def.name, p, def.label); });
      wrap.appendChild(r);
    }
  });
}

PM.syncShaderUniforms = (L) => {
  const defs = PM.parseUniforms(L.d.code);
  L._udefs = defs;
  const u = L.d.uniforms;
  defs.forEach(d => { if (!u[d.name]) u[d.name] = PM.P(d.def); });
  for (const k in u) if (!defs.some(d => d.name === k)) delete u[k];
};

/* ── effects ───────────────────────────────────────────── */
function effects(wrap, L) {
  const sec = PM.section('Effects');
  wrap.appendChild(sec);
  if (!L.fx.length) {
    const b = h('button.chip', { style: { width: '100%', justifyContent: 'center', height: '28px' }, onpointerdown: (e) => { e.preventDefault(); fxMenu(e.target); } }, PM.icon('plus'), 'Add effect');
    wrap.appendChild(b);
  }
  L.fx.forEach((fx, i) => {
    const def = PM.FX[fx.type]; if (!def) return;
    const tw = h('span.twirl' + (fx.open ? '.open' : ''), PM.icon('chev'));
    const onBtn = h('button.stopwatch' + (fx.on ? '.on' : ''), PM.icon('eye'));
    onBtn.onclick = (e) => { e.stopPropagation(); PM.hist.do('Toggle effect', () => { fx.on = !fx.on; }); I.refresh(); PM.invalidate(); };
    const head = h('div.row', { style: { marginTop: '4px', background: 'rgba(128,128,136,.08)' } },
      tw, h('div.k', { style: { color: 'var(--tx)', fontWeight: 500 } }, def.label), onBtn,
      h('button.stopwatch', { onclick: (e) => { e.stopPropagation(); PM.hist.do('Remove effect', () => { L.fx.splice(i, 1); }); I.refresh(); PM.invalidate(); } }, PM.icon('x')));
    head.onclick = () => { fx.open = !fx.open; I.refresh(); };
    wrap.appendChild(head);
    if (!fx.open) return;
    const g = h('div.grp');
    def.params.forEach(pd => {
      const p = fx.p[pd.k];
      if (pd.type === 'color') { g.appendChild(PM.row(pd.label, PM.colorField(() => p.v, v => { p.v = v; PM.invalidate(); }, { label: pd.label }))); return; }
      const sw = h('button.stopwatch' + (p.kf.length ? '.on' : ''), PM.svg('<circle cx="12" cy="12" r="7"/><path d="M12 8v4l2.5 1.5"/>'));
      sw.onclick = () => {
        PM.hist.do('Animate ' + pd.label, () => {
          if (p.kf.length) { p.v = PM.evP(L, p, PM.time, pd.k); p.kf = []; }
          else PM.setKeyOn(p, PM.time - L.from, p.v, 'power', PM.proj.fps);
        });
        I.refresh();
      };
      const f = PM.numField(() => PM.evP(L, p, PM.time, pd.k), (v) => {
        if (p.kf.length) PM.setKeyOn(p, PM.time - L.from, v, 'power', PM.proj.fps); else p.v = v;
        PM.touch(); PM.invalidate();
      }, { label: pd.label, step: pd.step, min: pd.min, max: pd.max, unit: pd.unit });
      I.syncs.push(f.sync);
      const r = PM.row(pd.label, f, { left: sw });
      r.insertBefore(sw, r.firstChild);
      r.addEventListener('contextmenu', (e) => { e.preventDefault(); chanMenu(e, L, fx.id + '.' + pd.k, p, pd.label); });
      g.appendChild(r);
    });
    wrap.appendChild(g);
  });
}

function newLayerMenu(anchor) {
  PM.menu(anchor, [
    { header: 'New layer' },
    { label: 'Text', kb: '⌘T', run: () => PM.cmd('newText') },
    { label: 'Shape', kb: '⌘⇧Y', run: () => PM.cmd('newShape') },
    { label: 'Solid', kb: '⌘Y', run: () => PM.cmd('newSolid') },
    { label: 'Shader', kb: '⌘⇧G', run: () => PM.cmd('newShader') },
    { label: 'Null', run: () => PM.cmd('newNull') },
    '-', { label: 'Import media…', kb: '⌘I', run: () => PM.cmd('import') },
  ], { right: true });
}

function fxMenu(anchor) {
  const L = PM.firstSel();
  if (!L) return PM.toast('Select a layer first');
  const groups = {};
  Object.entries(PM.FX).forEach(([k, d]) => (groups[d.group] = groups[d.group] || []).push([k, d]));
  const items = [];
  Object.entries(groups).forEach(([g, list]) => {
    items.push({ header: g });
    list.forEach(([k, d]) => items.push({
      label: d.label, run: () => PM.hist.do('Add ' + d.label, () => { const fx = PM.mkEffect(k); fx.open = true; L.fx.push(fx); I.refresh(); PM.invalidate(); }),
    }));
  });
  PM.menu(anchor, items, { right: true });
}
PM.fxMenu = fxMenu;

/* ── layer options ─────────────────────────────────────── */
function layerOptions(wrap, L) {
  wrap.appendChild(PM.section('Layer'));
  wrap.appendChild(PM.row('Blend mode', PM.selectField(() => L.blend, v => { L.blend = v; PM.invalidate(); }, PM.BLENDS, { label: 'Blend' })));
  wrap.appendChild(PM.row('Motion blur', PM.toggleField(() => L.mblur, v => { L.mblur = v; PM.invalidate(); }, { label: 'Motion blur' })));
  wrap.appendChild(PM.row('Parent', PM.selectField(
    () => { const p = PM.L(L.parent); return p ? p.name : 'none'; },
    (v) => { L.parent = v; PM.invalidate(); },
    [{ v: null, label: 'none' }, ...PM.proj.layers.filter(o => o.id !== L.id).map(o => ({ v: o.id, label: o.name }))], { label: 'Parent' })));
  wrap.appendChild(PM.row('Color', PM.colorField(() => L.color, v => { L.color = v; PM.invalidate(); }, { label: 'Label color' })));
  numRow(wrap, 'Start', () => L.from, v => { L.from = Math.max(0, v); PM.invalidate(); }, { step: .05, precision: 2, unit: 's' });
  numRow(wrap, 'Duration', () => L.dur, v => { L.dur = Math.max(.02, v); PM.invalidate(); }, { step: .05, precision: 2, unit: 's' });
}

/* ── scene params (project level, agent-authored) ──────── */
function sceneParams(wrap) {
  const ps = Object.values(PM.proj.params || {});
  wrap.appendChild(PM.section('Composition'));
  numRow(wrap, 'Width', () => PM.proj.w, v => { PM.proj.w = Math.round(v); PM.bus.emit('project'); }, { step: 2, min: 16 });
  numRow(wrap, 'Height', () => PM.proj.h, v => { PM.proj.h = Math.round(v); PM.bus.emit('project'); }, { step: 2, min: 16 });
  numRow(wrap, 'Duration', () => PM.proj.dur, v => { PM.proj.dur = Math.max(.2, v); PM.proj.work = [0, PM.proj.dur]; PM.bus.emit('project'); }, { step: .5, precision: 2, unit: 's' });
  numRow(wrap, 'Frame rate', () => PM.proj.fps, v => { PM.proj.fps = Math.round(PM.clamp(v, 1, 240)); PM.bus.emit('project'); }, { step: 1 });
  wrap.appendChild(PM.row('Background', PM.colorField(() => PM.proj.bg, v => { PM.proj.bg = v; PM.invalidate(); }, { label: 'Background' })));
  if (ps.length) {
    wrap.appendChild(PM.section('Scene parameters'));
    ps.forEach(p => {
      if (p.control === 'color') wrap.appendChild(PM.row(p.label, PM.colorField(() => p.value, v => { p.value = v; PM.touch(); PM.invalidate(); }, { label: p.label })));
      else if (p.control === 'toggle') wrap.appendChild(PM.row(p.label, PM.toggleField(() => p.value, v => { p.value = v; PM.touch(); PM.invalidate(); }, { label: p.label })));
      else if (p.control === 'select') wrap.appendChild(PM.row(p.label, PM.selectField(() => p.value, v => { p.value = v; PM.touch(); PM.invalidate(); }, p.options || [], { label: p.label })));
      else numRow(wrap, p.label, () => p.value, v => { p.value = v; PM.touch(); PM.invalidate(); }, { step: (p.max - p.min) / 200 || .01, min: p.min, max: p.max, precision: 3 });
    });
  }
  wrap.appendChild(h('div.empty', 'Select a layer to edit its properties.'));
}

I.focusText = (L) => {
  PM.selectLayers(L.id);
  requestAnimationFrame(() => { if (I.textArea) { I.textArea.focus(); I.textArea.select(); } });
};
})();
