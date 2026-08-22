/* Powermove — inspector controls: scrub numbers, colors, toggles, selects. */
(() => {
const PM = window.PM, h = PM.h;

/* Controls may receive `command(value)`. That makes the same visual control
   usable by the inspector, agent-authored panels, and future generated UI
   without giving the control a private mutation path. */
const hasCommand = (opt) => !!(opt && opt.command);
const sourceCommand = (opt, value) => typeof opt.command === 'function' ? opt.command(value) : { ...opt.command, value };
const begin = (opt, label) => hasCommand(opt) ? PM.Edit.begin(label, { origin: opt.origin || 'interface' }) : PM.hist.begin(label);
const write = (set, value, opt) => hasCommand(opt) ? PM.Edit.dispatch(sourceCommand(opt, value)) : set(value);
const commit = (opt, label) => hasCommand(opt) ? PM.Edit.commit(label) : PM.hist.commit(label);
const cancel = (opt) => hasCommand(opt) ? PM.Edit.cancel() : PM.hist.cancel();
const once = (set, value, opt, label) => hasCommand(opt)
  ? PM.Edit.apply(sourceCommand(opt, value), { label, origin: opt.origin || 'interface' })
  : PM.hist.do(label, () => set(value));

/** Scrubbable numeric field. opts: {min,max,step,unit,precision,onInput,onCommit,label} */
PM.numField = (get, set, opt = {}) => {
  const el = h('div.num' + (opt.link ? '.link' : ''), { title: opt.label || '' });
  const fmt = (v) => {
    if (typeof v !== 'number' || !isFinite(v)) return String(v);
    const p = opt.precision != null ? opt.precision : (opt.step && opt.step < 1 ? 2 : (Math.abs(v) < 10 ? 1 : 0));
    let s = v.toFixed(p);
    if (p > 0) s = s.replace(/\.?0+$/, '');
    return s + (opt.unit || '');
  };
  const sync = () => { if (!el.classList.contains('editing')) el.textContent = fmt(get()); };
  el.sync = sync; sync();

  el.addEventListener('pointerdown', (e) => {
    if (el.classList.contains('editing')) return;
    if (e.button !== 0) return;
    let start = get(), moved = false;
    begin(opt, opt.label || 'Adjust');
    PM.drag(e, {
      cursor: 'ew-resize',
      move: (dx, dy, ev) => {
        if (!moved && Math.abs(dx) < 3) return;
        moved = true;
        const mult = ev.shiftKey ? 10 : ev.altKey ? .1 : 1;
        const step = (opt.step || 1) * mult;
        let v = start + dx * step * (opt.speed || .5);
        if (opt.min != null) v = Math.max(opt.min, v);
        if (opt.max != null) v = Math.min(opt.max, v);
        write(set, PM.round(v, 3), opt); sync();
      },
      up: () => {
        if (!moved) { edit(); cancel(opt); }
        else commit(opt, opt.label || 'Adjust');
      },
    });
  });

  function edit() {
    el.classList.add('editing');
    const inp = h('input', { value: String(PM.round(get(), 3)), style: { width: Math.max(40, el.offsetWidth) + 'px', textAlign: 'right', fontFamily: 'var(--f-mono)' } });
    el.textContent = ''; el.appendChild(inp);
    inp.focus(); inp.select();
    const done = (ok) => {
      el.classList.remove('editing');
      if (ok) {
        let v = inp.value.trim();
        let n = /^[-+*/]/.test(v) ? evalSafe(get() + v) : evalSafe(v);
        if (isFinite(n)) {
          once(set, PM.round(n, 4), opt, opt.label || 'Set value');
        }
      }
      sync();
    };
    inp.addEventListener('blur', () => done(true));
    inp.addEventListener('keydown', (e) => {
      e.stopPropagation();
      if (e.key === 'Enter') { e.preventDefault(); inp.blur(); }
      if (e.key === 'Escape') { e.preventDefault(); el.classList.remove('editing'); sync(); }
      if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
        e.preventDefault();
        const d = (e.key === 'ArrowUp' ? 1 : -1) * (opt.step || 1) * (e.shiftKey ? 10 : 1);
        inp.value = String(PM.round(parseFloat(inp.value || 0) + d, 4));
      }
    });
  }
  return el;
};
function evalSafe(s) {
  if (!/^[-+*/(). 0-9e]+$/i.test(s)) return NaN;
  try { return Function('"use strict";return (' + s + ')')(); } catch { return NaN; }
}

PM.colorField = (get, set, opt = {}) => {
  /* tolerate undefined/non-string values instead of painting "UNDEFINED" */
  const safe = () => { const v = get(); return typeof v === 'string' && /^#[0-9a-f]{3,8}$/i.test(v) ? v : '#808080'; };
  const sw = h('div.sw', { style: { background: safe() } });
  const val = h('span', { style: { fontFamily: 'var(--f-mono)', fontSize: 'var(--fs-md)', color: 'var(--tx)' } }, safe().toUpperCase());
  const wrap = h('div', { style: { display: 'flex', alignItems: 'center', gap: '8px' } }, val, sw);
  const inp = h('input', { type: 'color', value: safe(), style: { position: 'absolute', width: 0, height: 0, opacity: 0 } });
  wrap.appendChild(inp);
  wrap.sync = () => { sw.style.background = safe(); val.textContent = safe().toUpperCase(); };
  let live = false;
  inp.addEventListener('input', () => { if (!live) { begin(opt, opt.label || 'Color'); live = true; } write(set, inp.value, opt); wrap.sync(); });
  inp.addEventListener('change', () => { commit(opt, opt.label || 'Color'); live = false; });
  wrap.addEventListener('pointerdown', (e) => { e.stopPropagation(); inp.click(); });
  return wrap;
};

PM.toggleField = (get, set, opt = {}) => {
  const t = h('div.toggle' + (get() ? '.on' : ''), h('i'));
  t.sync = () => t.classList.toggle('on', !!get());
  t.addEventListener('pointerdown', (e) => {
    e.stopPropagation();
    once(set, !get(), opt, opt.label || 'Toggle');
    t.sync(); PM.invalidate();
  });
  return t;
};

PM.selectField = (get, set, options, opt = {}) => {
  const b = h('button.sel', String(get()));
  b.sync = () => { b.textContent = String(get()); };
  b.addEventListener('pointerdown', (e) => {
    e.stopPropagation(); e.preventDefault();
    PM.menu(b, options.map(o => {
      const v = typeof o === 'string' ? o : o.v;
      const l = typeof o === 'string' ? o : o.label;
      return { label: l, on: v === get(), run: () => { once(set, v, opt, opt.label || 'Change'); b.sync(); PM.invalidate(); opt.onChange && opt.onChange(v); } };
    }), { right: true });
  });
  return b;
};

PM.textField = (get, set, opt = {}) => {
  const inp = h('input', {
    value: get() == null ? '' : String(get()),
    style: { textAlign: opt.align || 'right', fontFamily: opt.mono === false ? 'var(--f-ui)' : 'var(--f-mono)', fontSize: 'var(--fs-md)', width: '100%', minWidth: '40px' },
  });
  inp.sync = () => { if (document.activeElement !== inp) inp.value = get() == null ? '' : String(get()); };
  let live = false;
  inp.addEventListener('focus', () => { begin(opt, opt.label || 'Edit text'); live = true; });
  inp.addEventListener('input', () => { write(set, inp.value, opt); PM.invalidate('render'); });
  inp.addEventListener('blur', () => { if (live) { commit(opt, opt.label || 'Edit text'); live = false; } });
  inp.addEventListener('keydown', (e) => { e.stopPropagation(); if (e.key === 'Enter') inp.blur(); });
  return inp;
};

/** A standard inspector row: label + value area. */
PM.row = (label, value, extras = {}) => {
  const k = h('div.k', label);
  const vw = h('div.vwrap', value);
  const r = h('div.row.split', extras.left || null, k, vw);
  r.valueWrap = vw; r.labelEl = k;
  if (extras.onLabel) k.addEventListener('pointerdown', extras.onLabel);
  return r;
};
PM.section = (t) => h('div.sec', t);
})();
