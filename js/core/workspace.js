/* Powermove — workspaces. The UI is data: docks, panels, theme, features, custom controls. */
(() => {
const PM = window.PM, h = PM.h;

const dock = (id, panels, size) => ({ id, size, panels });
const p = (id, o = {}) => ({ id, ...o });

const PRESETS = () => ([
  {
    id: 'design', name: 'Design', builtin: true, density: 'normal',
    theme: { accent: '#FF6B1A' },
    features: { motionBlur: true, snapping: true, guides: true, autosave: true, adaptiveQuality: true },
    layout: {
      docks: [
        dock('left', [p('assets', { size: 150 }), p('layers', { flex: true }), p('chat', { size: 280 })], 250),
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 300 })]),
        dock('right', [p('inspector', { flex: true })], 300),
      ],
    },
  },
  {
    id: 'animate', name: 'Animate', builtin: true, density: 'compact',
    theme: { accent: '#FF6B1A' },
    features: { motionBlur: true, snapping: true, guides: false, autosave: true, graphOnOpen: true },
    layout: {
      docks: [
        dock('left', [p('layers', { flex: true }), p('takes', { size: 160 })], 230),
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
        dock('left', [p('assets', { flex: true }), p('layers', { size: 240 })], 250),
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
        dock('right', [p('chat', { flex: true }), p('notes', { size: 170 })], 400),
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

WS.init = () => {
  const saved = PM.store.get('workspaces', null);
  WS.all = saved && saved.length ? saved : PRESETS();
  /* always keep builtins available even if the user saved before they existed */
  PRESETS().forEach(preset => { if (!WS.all.some(w => w.id === preset.id)) WS.all.push(preset); });
  const lastId = PM.store.get('workspace', 'design');
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
  fn(WS.current);
  WS.save();
  WS.activate(WS.current.id, true);
  return WS.current;
};

WS.create = (spec) => {
  const base = JSON.parse(JSON.stringify(WS.get(spec.base) || WS.get('design')));
  const w = Object.assign(base, spec, { id: PM.uid('ws'), builtin: false });
  w.name = spec.name || 'Workspace';
  WS.all.push(w);
  WS.save();
  WS.activate(w.id);
  return w;
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
        body.appendChild(wrap);
        if (cp.note) wrap.appendChild(h('div', { style: { color: 'var(--tx-3)', fontSize: '11.5px', lineHeight: 1.6, padding: '2px 4px 8px' } }, cp.note));
        (cp.controls || []).forEach(ct => {
          if (ct.type === 'button') {
            wrap.appendChild(h('button.chip', {
              style: { width: '100%', justifyContent: 'center', height: '28px', marginBottom: '4px' },
              onclick: () => { if (ct.cmd) PM.cmd(ct.cmd); else if (ct.prompt) PM.Agent.send(ct.prompt); },
            }, ct.label));
            return;
          }
          const param = ensureParam(ct);
          if (ct.type === 'color') wrap.appendChild(PM.row(ct.label, PM.colorField(() => param.value, v => { param.value = v; applyParam(param); }, { label: ct.label })));
          else if (ct.type === 'toggle') wrap.appendChild(PM.row(ct.label, PM.toggleField(() => param.value, v => { param.value = v; applyParam(param); }, { label: ct.label })));
          else if (ct.type === 'select') wrap.appendChild(PM.row(ct.label, PM.selectField(() => param.value, v => { param.value = v; applyParam(param); }, ct.options || [], { label: ct.label })));
          else {
            const f = PM.numField(() => param.value, v => { param.value = v; applyParam(param); },
              { label: ct.label, min: ct.min, max: ct.max, step: ct.step || ((ct.max - ct.min) / 100) || .01, precision: 3, unit: ct.unit });
            PM.Inspector.syncs.push(f.sync);
            wrap.appendChild(PM.row(ct.label, f));
          }
        });
      },
    });
  });
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
})();
