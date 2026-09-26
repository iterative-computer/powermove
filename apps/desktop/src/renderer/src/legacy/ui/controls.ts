import { legacyColorField } from '../../controls/legacy-color.svelte';
/* Ported from js/ui/controls.js — behavior-preserving. */
import { cssFontStack } from '../../typography/font-stack';
import { horizontalScrub } from '../../controls/horizontal-scrub';
import type { PMRegistry } from '../registry';
import { consumeMenuTriggerPress, markMenuDismissal } from '../../overlays/dismissal';

export function install(PM: PMRegistry): void {
const h = PM.h;
/* Controls may receive `command(value)`. That makes the same visual control
   usable by the inspector, agent-authored panels, and future generated UI
   without giving the control a private mutation path. */
const hasCommand = (opt: any) => !!(opt && opt.command);
const sourceCommand = (opt: any, value: any) => typeof opt.command === 'function' ? opt.command(value) : { ...opt.command, value };
const begin = (opt: any, label: any) => opt.local ? true : hasCommand(opt) ? PM.Edit.begin(label, { origin: opt.origin || 'interface' }) : PM.hist.begin(label);
const write = (set: any, value: any, opt: any) => opt.local ? set(value) : hasCommand(opt) ? PM.Edit.dispatch(sourceCommand(opt, value)) : set(value);
const commit = (opt: any, label: any) => opt.local ? true : hasCommand(opt) ? PM.Edit.commit(label) : PM.hist.commit(label);
const cancel = (opt: any) => opt.local ? true : hasCommand(opt) ? PM.Edit.cancel() : PM.hist.cancel();
const once = (set: any, value: any, opt: any, label: any) => opt.local
  ? set(value)
  : hasCommand(opt)
  ? PM.Edit.apply(sourceCommand(opt, value), { label, origin: opt.origin || 'interface' })
  : PM.hist.do(label, () => set(value));

/** Scrubbable numeric field. opts: {min,max,step,unit,precision,onInput,onCommit,label} */
PM.numField = (get: any, set: any, opt: any = {}) => {
  const el = h('div.num' + (opt.link ? '.link' : ''), { title: opt.label || '' });
  const fmt = (v: any) => {
    if (typeof v !== 'number' || !isFinite(v)) return String(v);
    const p = opt.precision != null ? opt.precision : (opt.step && opt.step < 1 ? 2 : (Math.abs(v) < 10 ? 1 : 0));
    let s = v.toFixed(p);
    if (p > 0) s = s.replace(/\.?0+$/, '');
    return s + (opt.unit || '');
  };
  const sync = () => { if (!el.classList.contains('editing')) el.textContent = fmt(get()); };
  el.sync = sync; sync();
  let wheelValue = 0;
  horizontalScrub(el, {
    enabled: () => !el.classList.contains('editing'),
    begin: () => { wheelValue = Number(get()); begin(opt, opt.label || 'Adjust'); },
    move: (delta, event) => {
      const previous = PM.round(wheelValue, 3);
      wheelValue += delta * (opt.step || 1) * (opt.speed || .5) * (event.shiftKey ? 10 : event.altKey ? .1 : 1);
      wheelValue = Math.max(opt.min ?? -Infinity, Math.min(opt.max ?? Infinity, wheelValue));
      const next = PM.round(wheelValue, 3);
      if (next === previous) return false;
      write(set, next, opt); sync();
      return true;
    },
    commit: () => commit(opt, opt.label || 'Adjust'),
    cancel: () => cancel(opt)
  });

  el.addEventListener('pointerdown', (e: any) => {
    if (el.classList.contains('editing')) return;
    if (e.button !== 0) return;
    let start = get(), moved = false;
    begin(opt, opt.label || 'Adjust');
    PM.drag(e, {
      cursor: 'ew-resize',
      infinite: true,
      move: (dx: any, dy: any, ev: any) => {
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
      cancel: () => cancel(opt),
    });
  });

  function edit() {
    el.classList.add('editing');
    const inp = h('input', { value: String(PM.round(get(), 3)), style: { width: Math.max(40, el.offsetWidth) + 'px', textAlign: 'right', fontFamily: 'var(--f-mono)' } });
    el.textContent = ''; el.appendChild(inp);
    inp.focus(); inp.select();
    const done = (ok: any) => {
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
    inp.addEventListener('keydown', (e: any) => {
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
function evalSafe(s: any) {
  if (!/^[-+*/(). 0-9e]+$/i.test(s)) return NaN;
  try { return Function('"use strict";return (' + s + ')')(); } catch { return NaN; }
}

const clampChannel = (value: any, max: any = 255) => Math.round(PM.clamp(Number(value) || 0, 0, max));
const normalizeHex = (value: any) => {
  const raw = String(value || '').trim().replace(/^#/, '');
  if (/^[0-9a-f]{3}$/i.test(raw)) return `#${raw.split('').map((char: any) => char + char).join('').toUpperCase()}`;
  return /^[0-9a-f]{6}$/i.test(raw) ? `#${raw.toUpperCase()}` : null;
};
const rgbToHex = ({ r, g, b }: any) => `#${[r, g, b].map((value: any) => clampChannel(value).toString(16).padStart(2, '0')).join('').toUpperCase()}`;
const hexToRgb = (value: any) => {
  const hex = normalizeHex(value); if (!hex) return null;
  return { r: parseInt(hex.slice(1, 3), 16), g: parseInt(hex.slice(3, 5), 16), b: parseInt(hex.slice(5, 7), 16) };
};
const rgbToHsv = ({ r, g, b }: any) => {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  let h = 0;
  if (d) h = max === r ? 60 * (((g - b) / d) % 6) : max === g ? 60 * ((b - r) / d + 2) : 60 * ((r - g) / d + 4);
  if (h < 0) h += 360;
  return { h: Math.round(h), s: Math.round(max ? d / max * 100 : 0), v: Math.round(max * 100) };
};
const hsvToRgb = ({ h, s, v }: any) => {
  h = ((Number(h) || 0) % 360 + 360) % 360; s = PM.clamp(Number(s) || 0, 0, 100) / 100; v = PM.clamp(Number(v) || 0, 0, 100) / 100;
  const c = v * s, x = c * (1 - Math.abs((h / 60) % 2 - 1)), m = v - c;
  const parts: any = h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
  return { r: Math.round((parts[0] + m) * 255), g: Math.round((parts[1] + m) * 255), b: Math.round((parts[2] + m) * 255) };
};
PM.Color = { normalizeHex, rgbToHex, hexToRgb, rgbToHsv, hsvToRgb };

PM.colorField = (get: any, set: any, opt: any = {}) => legacyColorField(PM, false, get, set, opt);
PM.fillField = (get: any, set: any, opt: any = {}) => legacyColorField(PM, true, get, set, opt);

PM.toggleField = (get: any, set: any, opt: any = {}) => {
  // Off | On segments with a gliding pill; shares .onoff styles with ToggleField.svelte.
  const off = h('button.onoff-off', { type: 'button', role: 'radio' }, 'Off');
  const on = h('button.onoff-on', { type: 'button', role: 'radio' }, 'On');
  const t = h('div.onoff' + (get() ? '.on' : ''), h('span.onoff-pill', { 'aria-hidden': 'true' }), off, on);
  t.setAttribute('role', 'radiogroup');
  t.sync = () => {
    const value = !!get();
    t.classList.toggle('on', value);
    off.setAttribute('aria-checked', String(!value)); on.setAttribute('aria-checked', String(value));
  };
  t.sync();
  t.addEventListener('pointerdown', (e: any) => e.stopPropagation());
  const choose = (next: boolean) => { if (next === !!get()) return; once(set, next, opt, opt.label || 'Toggle'); t.sync(); PM.invalidate(); };
  off.addEventListener('click', () => choose(false));
  on.addEventListener('click', () => choose(true));
  return t;
};

PM.selectField = (get: any, set: any, options: any, opt: any = {}) => {
  const labelFor = (value: any) => {
    const option = options.find((item: any) => (typeof item === 'string' ? item : item.v) === value);
    return String(option && typeof option === 'object' ? option.label : option ?? value);
  };
  const b = h('button.sel', labelFor(get()));
  b.sync = () => { b.textContent = labelFor(get()); };
  b.addEventListener('pointerdown', (e: any) => {
    e.stopPropagation(); e.preventDefault();
    PM.menu(b, options.map((o: any) => {
      const v = typeof o === 'string' ? o : o.v;
      const l = typeof o === 'string' ? o : o.label;
      return { label: l, on: v === get(), run: () => { once(set, v, opt, opt.label || 'Change'); b.sync(); PM.invalidate(); opt.onChange && opt.onChange(v); } };
    }), { right: true });
  });
  return b;
};

PM.fontField = (get: any, set: any, opt: any = {}) => {
  const familyStyle = (value: any) => cssFontStack(String(value || ''));
  const b = h('button.sel.font-select', String(get()));
  b.sync = () => {
    b.textContent = String(get());
    b.style.fontFamily = familyStyle(get());
    b.title = String(get());
  };
  b.sync();

  b.addEventListener('pointerdown', (event: any) => {
    event.stopPropagation(); event.preventDefault();
    PM.closeMenus();
    const menu = h('div.drop.font-drop');
    const search = h('input.font-search', { placeholder: 'Search fonts', 'aria-label': 'Search fonts', autocomplete: 'off', spellcheck: 'false' });
    const results = h('div.font-results');
    menu.append(search, results);
    window.document.body.appendChild(menu);

    const render = () => {
      const query = search.value.trim().toLocaleLowerCase();
      const all = (PM.Fonts ? PM.Fonts.options(get()) : [get()]);
      const matches = all.filter((name: any) => !query || name.toLocaleLowerCase().includes(query));
      results.textContent = '';
      for (const name of matches.slice(0, 180)) {
        const item = h('button.di.font-item' + (name === get() ? '.on' : ''), {
          type: 'button', role: 'menuitem', title: name,
          style: { fontFamily: familyStyle(name) },
          onclick: (e: any) => {
            e.stopPropagation();
            once(set, name, opt, opt.label || 'Change font');
            b.sync(); PM.closeMenus(); PM.invalidate();
            PM.Fonts && PM.Fonts.ensure(name, opt.weight ? opt.weight() : 400);
            opt.onChange && opt.onChange(name);
          },
        }, name);
        results.appendChild(item);
      }
      if (!matches.length) results.appendChild(h('div.font-empty', 'No matching fonts'));
      else if (matches.length > 180) results.appendChild(h('div.font-empty', `${matches.length - 180} more · keep typing to narrow`));
    };
    const unsubscribeFonts = PM.bus.on('fonts', render);
    const observer = new MutationObserver(() => {
      if (!menu.isConnected) { unsubscribeFonts?.(); observer.disconnect(); }
    });
    observer.observe(window.document.body, { childList: true });
    search.addEventListener('input', render);
    search.addEventListener('keydown', (e: any) => {
      e.stopPropagation();
      if (e.key === 'Escape') { e.preventDefault(); PM.closeMenus(); b.focus(); }
      if (e.key === 'Enter') {
        const first = results.querySelector('.font-item');
        if (first) { e.preventDefault(); first.click(); }
      }
    });
    render();

    const rect = b.getBoundingClientRect();
    const width = Math.max(230, Math.min(310, rect.width + 120));
    menu.style.width = width + 'px';
    menu.style.left = PM.clamp(rect.right - width, 6, window.innerWidth - width - 6) + 'px';
    menu.style.top = PM.clamp(rect.bottom + 5, 6, window.innerHeight - menu.offsetHeight - 6) + 'px';
    PM._menuOutside = (e: any) => { if (menu.contains(e.target)) return; markMenuDismissal(e); consumeMenuTriggerPress(e, b); PM.closeMenus(); };
    window.setTimeout(() => window.document.addEventListener('pointerdown', PM._menuOutside, true), 0);
    window.requestAnimationFrame(() => search.focus());
  });
  return b;
};

PM.textField = (get: any, set: any, opt: any = {}) => {
  const inp = h('input', {
    value: get() == null ? '' : String(get()),
    style: { textAlign: opt.align || 'right', fontFamily: opt.mono === false ? 'var(--f-ui)' : 'var(--f-mono)', fontSize: 'var(--fs-md)', width: '100%', minWidth: '40px' },
  });
  inp.sync = () => { if (window.document.activeElement !== inp) inp.value = get() == null ? '' : String(get()); };
  let live = false;
  inp.addEventListener('focus', () => { begin(opt, opt.label || 'Edit text'); live = true; });
  inp.addEventListener('input', () => { write(set, inp.value, opt); PM.invalidate('render'); });
  inp.addEventListener('blur', () => { if (live) { commit(opt, opt.label || 'Edit text'); live = false; } });
  inp.addEventListener('keydown', (e: any) => { e.stopPropagation(); if (e.key === 'Enter') inp.blur(); });
  return inp;
};

/** A standard inspector row: label + value area. */
PM.row = (label: any, value: any, extras: any = {}) => {
  const k = h('div.k', label);
  const vw = h('div.vwrap', value);
  const r = h('div.row.split', extras.left || null, k, vw);
  r.valueWrap = vw; r.labelEl = k;
  if (extras.onLabel) k.addEventListener('pointerdown', extras.onLabel);
  return r;
};
PM.section = (t: any) => h('div.sec', t);
}
