/* Powermove — workspaces. The UI is data: docks, panels, theme, features, custom controls. */
(() => {
const PM = window.PM, h = PM.h;

const dock = (id, panels, size) => ({ id, size, panels });
const p = (id, o = {}) => ({ id, ...o });

const copy = (value) => JSON.parse(JSON.stringify(value));
const finite = (value) => typeof value === 'number' && Number.isFinite(value);
const text = (value, fallback = '') => typeof value === 'string' && value.trim() ? value.trim() : fallback;

/* Agent-authored workspace JSON is persisted, so tolerate common model aliases and
   repair older malformed workspaces before they reach the layout or controls UI. */
function normalizeControl(control, index) {
  const raw = control && typeof control === 'object' ? control : {};
  const rawDefault = raw.def !== undefined ? raw.def : raw.default !== undefined ? raw.default : raw.value;
  let type = text(raw.type).toLowerCase();
  if (!['slider', 'color', 'toggle', 'select', 'button'].includes(type)) {
    if (Array.isArray(raw.options)) type = 'select';
    else if (typeof rawDefault === 'boolean') type = 'toggle';
    else if (typeof rawDefault === 'string' && /^#[0-9a-f]{3,8}$/i.test(rawDefault)) type = 'color';
    else type = 'slider';
  }
  const legacyParam = text(raw.parameter);
  const savedParam = text(raw.param);
  const generatedParam = /^Control \d+$/.test(savedParam);
  const authoredParam = generatedParam && legacyParam ? legacyParam : text(savedParam, legacyParam);
  const savedLabel = text(raw.label);
  const label = generatedParam && /^Control \d+$/.test(savedLabel) && legacyParam
    ? legacyParam
    : text(savedLabel, text(raw.name, text(authoredParam, `Control ${index + 1}`)));
  const out = { ...raw, type, label };
  if (type === 'button') return out;
  out.param = text(authoredParam, text(raw.name, label));
  if (type === 'color') out.def = typeof rawDefault === 'string' ? rawDefault : '#FF6B1A';
  else if (type === 'toggle') out.def = rawDefault === undefined ? false : !!rawDefault;
  else if (type === 'select') {
    out.options = Array.isArray(raw.options) ? raw.options.filter(v => typeof v === 'string') : [];
    out.def = rawDefault !== undefined ? rawDefault : (out.options[0] || '');
  } else {
    out.min = finite(raw.min) ? raw.min : 0;
    out.max = finite(raw.max) ? raw.max : Math.max(1, finite(rawDefault) ? Math.abs(rawDefault) * 2 : 1);
    if (out.max < out.min) [out.min, out.max] = [out.max, out.min];
    out.def = finite(rawDefault) ? rawDefault : out.min;
    out.step = finite(raw.step) && raw.step > 0 ? raw.step : Math.max((out.max - out.min) / 100, .01);
  }
  return out;
}

function normalizeWorkspace(workspace, fallback) {
  const raw = workspace && typeof workspace === 'object' ? copy(workspace) : {};
  const backup = fallback && typeof fallback === 'object' ? copy(fallback) : null;
  raw.id = text(raw.id, PM.uid('ws'));
  raw.name = text(raw.name, 'Workspace');
  raw.density = ['compact', 'normal', 'comfy'].includes(raw.density) ? raw.density : 'normal';
  raw.theme = raw.theme && typeof raw.theme === 'object' ? raw.theme : {};
  raw.features = raw.features && typeof raw.features === 'object' ? raw.features : {};
  raw.custom = (Array.isArray(raw.custom) ? raw.custom : []).map((panel, index) => {
    const cp = panel && typeof panel === 'object' ? panel : {};
    return {
      ...cp,
      id: text(cp.id, `custom-${raw.id}-${index + 1}`),
      title: text(cp.title, text(cp.name, 'Controls')),
      size: finite(cp.size) ? PM.clamp(cp.size, 72, 1200) : 220,
      controls: (Array.isArray(cp.controls) ? cp.controls : []).map(normalizeControl),
    };
  });

  let docks = raw.layout && Array.isArray(raw.layout.docks) ? raw.layout.docks : null;
  if (!docks || !docks.some(d => d && Array.isArray(d.panels) && d.panels.length)) {
    docks = backup && backup.layout && Array.isArray(backup.layout.docks)
      ? backup.layout.docks
      : [{ id: 'center', panels: [{ id: 'viewer', flex: true }] }];
  }
  const usedPanels = new Set();
  docks = docks.map((item, index) => {
    const d = item && typeof item === 'object' ? item : {};
    /* the assistant lives exclusively in its own collapsible rail */
    const panels = (Array.isArray(d.panels) ? d.panels : []).map(spec => {
      const q = typeof spec === 'string' ? { id: spec } : (spec && typeof spec === 'object' ? spec : {});
      const id = text(q.id);
      /* the assistant lives in its rail; the redundant Layers panel was removed */
      if (!id || id === 'chat' || id === 'layers' || usedPanels.has(id)) return null;
      usedPanels.add(id);
      const clean = { id };
      if (q.flex) clean.flex = true;
      if (finite(q.size)) clean.size = PM.clamp(q.size, 56, 1600);
      if (finite(q.min)) clean.min = PM.clamp(q.min, 32, 800);
      if (text(q.title)) clean.title = text(q.title);
      return clean;
    }).filter(Boolean);
    const out = { id: text(d.id, `dock-${index + 1}`), panels };
    if (finite(d.size)) out.size = PM.clamp(d.size, 200, 760);
    if (d.hidden) out.hidden = true;
    if (d.flex) out.flex = true;
    return out;
  }).filter(d => d.panels.length);
  if (!docks.length) docks = [{ id: 'center', panels: [{ id: 'viewer', flex: true }], flex: true }];

  /* The layout must always have one dock that consumes unused width. Models often
     call it "main" or "canvas" instead of "center"; the viewer/timeline dock is
     the semantic center regardless of its authored name. */
  const fluid = docks.find(d => d.id === 'center')
    || docks.find(d => d.panels.some(q => q.id === 'viewer' || q.id === 'timeline'))
    || docks.find(d => d.flex)
    || docks[0];
  fluid.flex = true;
  raw.layout = { ...(raw.layout && typeof raw.layout === 'object' ? raw.layout : {}), docks };
  return raw;
}

const PRESETS = () => ([
  {
    id: 'design', name: 'Design', builtin: true, density: 'normal',
    theme: { accent: '#FF6B1A' },
    features: { motionBlur: true, snapping: true, guides: true, autosave: true, adaptiveQuality: true },
    layout: {
      docks: [
        /* the timeline already owns layer ordering and visibility in Design;
           use the left rail for project media, the generative library, and effects */
        dock('left', [p('assets', { size: 150 }), p('library', { size: 220 }), p('fxbrowser', { flex: true })], 250),
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 300 })]),
        dock('right', [p('inspector', { flex: true })], 300),
      ],
    },
  },
  {
    id: 'gradient', name: 'Gradient', builtin: true, density: 'compact',
    theme: { accent: '#FF6B1A', radius: 10 },
    features: { motionBlur: true, snapping: true, guides: true, autosave: true, adaptiveQuality: true },
    layout: {
      docks: [
        dock('left', [p('gradient-controls', { flex: true }), p('assets', { size: 190 })], 300),
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 300 })]),
        dock('right', [p('inspector', { flex: true })], 340),
      ],
    },
    custom: [{
      id: 'gradient-controls', title: 'Gradient Editor', size: 220,
      controls: [
        { type: 'color', label: 'Gradient Start', param: 'Gradient Start', def: '#FF6B1A' },
        { type: 'color', label: 'Gradient End', param: 'Gradient End', def: '#34144F' },
        { type: 'slider', label: 'Gradient Angle', param: 'Gradient Angle', min: 0, max: 360, def: 0, step: 1 },
        { type: 'slider', label: 'Gradient Midpoint', param: 'Gradient Midpoint', min: 0, max: 100, def: 50, step: 1 },
        { type: 'slider', label: 'Gradient Softness', param: 'Gradient Softness', min: 0, max: 100, def: 50, step: 1 },
      ],
    }],
  },
  {
    id: 'animate', name: 'Animate', builtin: true, density: 'compact',
    theme: { accent: '#FF6B1A' },
    features: { motionBlur: true, snapping: true, guides: false, autosave: true, graphOnOpen: true },
    layout: {
      docks: [
        dock('left', [p('takes', { flex: true }), p('assets', { size: 170 })], 230),
        dock('center', [p('viewer', { size: 300 }), p('timeline', { flex: true })]),
        dock('right', [p('inspector', { flex: true })], 320),
      ],
    },
  },
  {
    id: 'shaderlab', name: 'Shader Lab', builtin: true, density: 'compact',
    theme: { accent: '#4C8DFF' },
    features: { motionBlur: false, snapping: true, guides: false, autosave: true },
    layout: {
      docks: [
        dock('left', [p('shader', { flex: true })], 460),
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 200 })]),
        dock('right', [p('inspector', { flex: true }), p('perf', { size: 190 })], 290),
      ],
    },
  },
  {
    id: 'edit', name: 'Edit', builtin: true, density: 'normal',
    theme: { accent: '#3FCF8E' },
    features: { motionBlur: false, snapping: true, guides: true, autosave: true },
    layout: {
      docks: [
        dock('left', [p('assets', { flex: true }), p('fxbrowser', { size: 240 })], 250),
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 360 })]),
        dock('right', [p('inspector', { flex: true })], 280),
      ],
    },
  },
  {
    id: 'review', name: 'Review', builtin: true, density: 'comfy',
    theme: { accent: '#FF6B1A' },
    features: { motionBlur: true, snapping: true, guides: false, autosave: true },
    layout: {
      docks: [
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 180 })]),
        dock('right', [p('library', { flex: true }), p('notes', { size: 170 })], 400),
      ],
    },
  },
  {
    id: 'focus', name: 'Focus', builtin: true, density: 'normal',
    theme: { accent: '#FF6B1A' },
    features: { motionBlur: true, snapping: true, guides: false, autosave: true },
    layout: { docks: [dock('center', [p('viewer', { flex: true })])] },
  },
]);

const WS = {
  current: null,
  all: [],
  list: () => WS.all,
  get: (id) => WS.all.find(w => w.id === id),
};
PM.WS = WS;

/* The first agent-authored Gradient workspace used a two-dock layout that put
   the canvas, timeline, Layers, and Inspector into one vertical stack. Replace
   only that recognizable legacy shape with the shared repaired preset. */
WS.isLegacyGradient = (w) => {
  if (!w || w.builtin || w.name !== 'Gradient') return false;
  const docks = w.layout && Array.isArray(w.layout.docks) ? w.layout.docks : [];
  const panelIds = docks.flatMap(d => Array.isArray(d.panels) ? d.panels.map(q => q && q.id) : []);
  return docks.length === 2
    && panelIds.includes('gradient-controls')
    && panelIds.includes('viewer')
    && panelIds.includes('timeline')
    && panelIds.includes('layers');
};

WS.init = () => {
  const saved = PM.store.get('workspaces', null);
  WS.all = saved && saved.length ? saved.map(w => normalizeWorkspace(w)) : PRESETS();
  const lastSavedId = PM.store.get('workspace', 'design');
  const legacyGradientIds = new Set(WS.all.filter(WS.isLegacyGradient).map(w => w.id));
  if (legacyGradientIds.size) WS.all = WS.all.filter(w => !legacyGradientIds.has(w.id));
  /* always keep builtins available even if the user saved before they existed */
  PRESETS().forEach(preset => { if (!WS.all.some(w => w.id === preset.id)) WS.all.push(preset); });
  if (legacyGradientIds.size) WS.save();
  const lastId = legacyGradientIds.has(lastSavedId) ? 'gradient' : lastSavedId;
  WS.activate(WS.get(lastId) ? lastId : 'design', true);
};

WS.activate = (id, silent) => {
  const w = WS.get(id); if (!w) return;
  WS.current = w;
  registerCustom(w);
  applyFeatures(w);
  PM.Layout.apply(w);
  PM.store.set('workspace', id);
  PM.bus.emit('workspaces');
  if (!silent) PM.toast('Workspace · ' + w.name);
};

function applyFeatures(w) {
  const f = w.features || {};
  if (f.motionBlur !== undefined) PM.mblurOn = !!f.motionBlur;
  if (f.snapping !== undefined) PM.snap = !!f.snapping;
  if (f.guides !== undefined) PM.guides = !!f.guides;
  if (f.adaptiveQuality !== undefined) PM.perf.auto = !!f.adaptiveQuality;
  if (f.graphOnOpen !== undefined) PM.TL.graph = !!f.graphOnOpen;
  PM.invalidate();
}

WS.save = () => {
  PM.store.set('workspaces', WS.all);
  PM.bus.emit('workspaces');
};

/** Mutate the active workspace and re-apply. All agent UI edits funnel through here. */
WS.mutate = (fn, opts = {}) => {
  const w = WS.current;
  if (w.builtin && !opts.inPlace) {
    const copy = JSON.parse(JSON.stringify(w));
    copy.id = PM.uid('ws');
    copy.name = w.name + ' (edited)';
    copy.builtin = false;
    WS.all.push(copy);
    WS.current = copy;
  }
  const fallback = copy(WS.current);
  fn(WS.current);
  const normalized = normalizeWorkspace(WS.current, fallback);
  const index = WS.all.findIndex(item => item.id === WS.current.id);
  if (index >= 0) WS.all[index] = normalized;
  WS.current = normalized;
  WS.save();
  WS.activate(WS.current.id, true);
  return WS.current;
};

WS.create = (spec) => {
  const base = copy(WS.get(spec.base) || WS.get('design'));
  const w = Object.assign(base, spec, { id: PM.uid('ws'), builtin: false });
  w.name = spec.name || 'Workspace';
  const normalized = normalizeWorkspace(w, base);
  WS.all.push(normalized);
  WS.save();
  WS.activate(normalized.id);
  return normalized;
};

WS.remove = (id) => {
  const w = WS.get(id);
  if (!w || w.builtin) return PM.toast('Built-in workspaces can’t be deleted');
  WS.all = WS.all.filter(x => x.id !== id);
  WS.save();
  if (WS.current.id === id) WS.activate('design');
  PM.bus.emit('workspaces');
};

WS.saveAsNew = () => {
  const inp = h('input', { value: WS.current.name.replace(' (edited)', '') + ' copy' });
  PM.modal({
    title: 'Save workspace', body: h('div.field', inp), width: 400,
    actions: [{ label: 'Cancel' }, {
      label: 'Save', pri: true, run: () => {
        const w = JSON.parse(JSON.stringify(WS.current));
        w.id = PM.uid('ws'); w.name = inp.value.trim() || 'Workspace'; w.builtin = false;
        WS.all.push(w); WS.save(); WS.activate(w.id);
      },
    }],
  });
  setTimeout(() => inp.focus(), 30);
};

WS.editJSON = () => {
  const ta = h('textarea.code', { style: { height: '420px', borderRadius: '8px' }, spellcheck: 'false' }, JSON.stringify(WS.current, null, 2));
  ta.addEventListener('keydown', e => e.stopPropagation());
  PM.modal({
    title: 'Workspace definition', body: ta, width: 680,
    actions: [{ label: 'Cancel' }, {
      label: 'Apply', pri: true, run: () => {
        try {
          const o = JSON.parse(ta.value);
          const i = WS.all.findIndex(w => w.id === WS.current.id);
          o.id = WS.current.id;
          WS.all[i] = o; WS.save(); WS.activate(o.id, true);
          PM.toast('Workspace updated');
        } catch (e) { PM.toast('Invalid JSON: ' + e.message, 3200); return false; }
      },
    }],
  });
};

/* ── custom panels authored by prompt ──────────────────── */
function registerCustom(w) {
  (w.custom || []).forEach(cp => {
    PM.registerPanel(cp.id, {
      title: cp.title || 'Panel',
      size: cp.size || 200,
      build(body) {
        const wrap = h('div.insp');
        const syncs = [];
        let offs = [];
        const sync = () => {
          if (!body.isConnected) { offs.forEach(off => off()); offs = []; return; }
          syncs.forEach(sync => { try { sync(); } catch (e) { } });
        };
        offs = ['draw:ui', 'project', 'history'].map(event => PM.bus.on(event, sync));
        body.appendChild(wrap);
        if (cp.note) wrap.appendChild(h('div', { style: { color: 'var(--tx-3)', fontSize: '11.5px', lineHeight: 1.6, padding: '2px 4px 8px' } }, cp.note));
        (cp.controls || []).forEach(ct => {
          if (ct.type === 'button') {
            wrap.appendChild(h('button.chip', {
              style: { width: '100%', justifyContent: 'center', height: '28px', marginBottom: '4px' },
              onclick: () => {
                if (Array.isArray(ct.commands)) PM.Edit.apply(ct.commands, { label: ct.label, origin: 'generated-ui' });
                else if (ct.cmd) PM.cmd(ct.cmd);
                else if (ct.prompt) PM.Agent.send(ct.prompt);
              },
            }, ct.label));
            return;
          }
          const binding = sourceBinding(ct);
          const param = binding ? null : ensureParam(ct);
          /* History may restore the project object. Resolve generated scene
             parameters by stable name so the mounted control follows undo,
             redo, agent edits, and direct edits to the current source. */
          const currentParam = () => PM.proj.params[param.name] || param;
          const get = binding ? binding.get : () => currentParam().value;
          const set = binding ? () => {} : v => { const active = currentParam(); active.value = v; applyParam(active); };
          const edit = binding ? {
            label: ct.label, origin: 'generated-ui', command: binding.command,
          } : {
            label: ct.label, origin: 'generated-ui',
            command: (value) => ({ type: 'set_scene_parameter', name: param.name, value }),
          };
          let field;
          if (ct.type === 'color') field = PM.colorField(get, set, edit);
          else if (ct.type === 'toggle') field = PM.toggleField(get, set, edit);
          else if (ct.type === 'select') field = PM.selectField(get, set, ct.options || [], edit);
          else {
            field = PM.numField(get, set,
              { ...edit, min: ct.min, max: ct.max, step: ct.step || ((ct.max - ct.min) / 100) || .01, precision: 3, unit: ct.unit });
          }
          if (field.sync) syncs.push(field.sync);
          wrap.appendChild(PM.row(ct.label, field));
        });
      },
    });
  });
}
function sourceBinding(ct) {
  const target = ct.target || (ct.binding && ct.binding.target);
  const path = ct.path || (ct.binding && ct.binding.path);
  if (!target || !path) return null;
  const layer = () => target === '$selection' || target === 'selection' ? PM.firstSel() : (PM.L(target) || PM.byName(target));
  const fallback = ct.def;
  if (path.startsWith('content.')) {
    const key = path.slice('content.'.length);
    return {
      get: () => { const L = layer(); return L && L.d[key] !== undefined ? L.d[key] : fallback; },
      command: (value) => ({ type: 'set_content', target, patch: { [key]: value } }),
    };
  }
  if (path.startsWith('properties.') || path.startsWith('transform.')) {
    const channel = path.replace(/^properties\./, '').replace(/^transform\./, '');
    return {
      get: () => { const L = layer(); const p = L && PM.findProp(L, channel); return L && p ? PM.evP(L, p, PM.time, channel) : fallback; },
      command: (value) => ({ type: 'set_property', target, path: channel, value, time: PM.time, mode: 'auto', preserveHandEdits: false, markIntent: 'human' }),
    };
  }
  if (path.startsWith('layer.')) {
    const key = path.slice('layer.'.length);
    const sourceKey = ({ visible: 'on', locked: 'lock', duration: 'dur', motionBlur: 'mblur' })[key] || key;
    return {
      get: () => { const L = layer(); return L && L[sourceKey] !== undefined ? L[sourceKey] : fallback; },
      command: (value) => ({ type: 'set_layer', target, patch: { [key]: value } }),
    };
  }
  return null;
}
function ensureParam(ct) {
  const name = ct.param || ct.label;
  const ps = PM.proj.params;
  if (!ps[name]) ps[name] = { name, label: ct.label, control: ct.type === 'slider' ? 'num' : ct.type, value: ct.def, min: ct.min, max: ct.max, options: ct.options };
  return ps[name];
}
function applyParam(param) {
  /* bound layers pick this up through param("name") in expressions */
  PM.touch(); PM.invalidate();
}
WS.registerCustom = registerCustom;
WS.normalize = normalizeWorkspace;
})();
