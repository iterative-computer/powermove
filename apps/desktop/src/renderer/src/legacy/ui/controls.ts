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

PM.colorField = (get: any, set: any, opt: any = {}) => {
  /* tolerate undefined/non-string values instead of painting "UNDEFINED" */
  const safe = () => { const v = get(); return typeof v === 'string' && /^#[0-9a-f]{3,8}$/i.test(v) ? v : '#808080'; };
  const sw = h('div.sw'); sw.style.setProperty('--sw-color', safe());
  const val = h('span', { style: { fontFamily: 'var(--f-mono)', fontSize: 'var(--fs-md)', color: 'var(--tx)' } }, safe().toUpperCase());
  const wrap = h('button.color-field', { 'aria-label': `${opt.label || 'Color'} · ${safe()}` }, val, sw);
  wrap.sync = () => {
    const value = safe(); sw.style.setProperty('--sw-color', value); val.textContent = value.toUpperCase();
    wrap.setAttribute('aria-label', `${opt.label || 'Color'} · ${value}`);
  };
  wrap.addEventListener('pointerdown', (e: any) => e.stopPropagation());
  wrap.addEventListener('click', () => {
    const before = safe();
    let previewing = false;
    openColorPicker(before, {
      preview: (value: any) => {
        if (!previewing) { begin(opt, opt.label || 'Color'); previewing = true; }
        write(set, value, opt); wrap.sync(); PM.invalidate('render');
      },
      commit: (value: any) => {
        if (previewing) { write(set, value, opt); commit(opt, opt.label || 'Color'); previewing = false; }
        wrap.sync(); PM.invalidate('render');
      },
      cancel: () => {
        if (!previewing) return;
        if (opt.local) set(before);
        cancel(opt); previewing = false; wrap.sync(); PM.invalidate('render');
      },
    }, opt.label || 'Color');
  });
  return wrap;
};

function openColorPicker(initial: any, edit: any, label: any) {
  const presets = ['#09090A', '#FFFFFF', '#FF6B1A', '#FFB000', '#34C759', '#0A84FF', '#6E5AE6', '#FF375F'];
  const before = normalizeHex(initial) || '#808080';
  let chosen = before;
  let hsv = rgbToHsv(hexToRgb(chosen));

  const sv = h('div.fill-sv.color-sv', { role: 'slider', tabindex: '0', 'aria-label': 'Saturation and brightness' }, h('i'));
  const hue = h('div.fill-hue.color-hue', { role: 'slider', tabindex: '0', 'aria-label': 'Hue' }, h('i'));
  const newSwatch = h('span.color-compare-swatch', { 'aria-hidden': 'true' });
  const oldSwatch = h('button.color-compare-swatch.is-before', { 'aria-label': `Restore original color ${before}`, title: `Original ${before}` });
  oldSwatch.style.setProperty('--sw-color', before);
  const hex = h('input.color-hex.pm-control-input', { value: chosen, 'aria-label': `${label} hex value`, spellcheck: 'false' });
  const grid = h('div.color-grid', { role: 'group', 'aria-label': 'Color presets' });

  const channels: any = {};
  const channelRow = (key: any, text: any, unit: any) => {
    const input = h('input.pm-control-input', { inputmode: 'numeric', 'aria-label': text });
    channels[key] = input;
    return h('label.color-channel', h('span', text), input, h('em', unit));
  };
  const channelList = h('div.color-channels',
    channelRow('h', 'H', '°'), channelRow('s', 'S', '%'), channelRow('v', 'B', '%'), h('hr', { 'aria-hidden': 'true' }),
    channelRow('r', 'R', ''), channelRow('g', 'G', ''), channelRow('b', 'B', ''));

  const sync = (skipHex?: any) => {
    const rgb = hexToRgb(chosen) || { r: 0, g: 0, b: 0 };
    newSwatch.style.setProperty('--sw-color', chosen);
    sv.style.setProperty('--hue-color', rgbToHex(hsvToRgb({ h: hsv.h, s: 100, v: 100 })));
    sv.firstChild.style.left = `${hsv.s}%`; sv.firstChild.style.top = `${100 - hsv.v}%`;
    hue.firstChild.style.top = `${hsv.h / 359 * 100}%`;
    channels.h.value = hsv.h; channels.s.value = hsv.s; channels.v.value = hsv.v;
    channels.r.value = rgb.r; channels.g.value = rgb.g; channels.b.value = rgb.b;
    if (!skipHex) hex.value = chosen;
    grid.querySelectorAll('button').forEach((button: any) => button.classList.toggle('on', button.dataset.color === chosen));
  };

  const setHex = (value: any, skipHex?: any) => {
    const valid = normalizeHex(value); if (!valid) return false;
    chosen = valid; hsv = rgbToHsv(hexToRgb(valid)); sync(skipHex); edit.preview(chosen); return true;
  };
  const fromHsv = (next: any) => {
    hsv = { h: clampChannel(next.h, 359), s: clampChannel(next.s, 100), v: clampChannel(next.v, 100) };
    chosen = rgbToHex(hsvToRgb(hsv)); sync(); edit.preview(chosen);
  };

  const pickSv = (event: any) => {
    const rect = sv.getBoundingClientRect(); if (!rect.width || !rect.height) return;
    fromHsv({ h: hsv.h, s: (event.clientX - rect.left) / rect.width * 100, v: 100 - (event.clientY - rect.top) / rect.height * 100 });
  };
  const pickHue = (event: any) => {
    const rect = hue.getBoundingClientRect(); if (!rect.height) return;
    fromHsv({ ...hsv, h: (event.clientY - rect.top) / rect.height * 359 });
  };
  const dragColor = (element: any, picker: any) => element.addEventListener('pointerdown', (event: any) => {
    if (event.button !== 0) return;
    event.preventDefault(); picker(event);
    PM.drag(event, { move: (_dx: any, _dy: any, next: any) => picker(next), up: () => {} });
  });
  dragColor(sv, pickSv); dragColor(hue, pickHue);

  presets.forEach((color: any) => {
    const choice = h('button.color-choice', { 'aria-label': color, title: color, onclick: () => setHex(color) });
    choice.dataset.color = color; choice.style.setProperty('--sw-color', color); grid.appendChild(choice);
  });
  oldSwatch.addEventListener('click', () => setHex(before));
  hex.addEventListener('input', () => { if (/^#?[0-9a-f]{6}$/i.test(hex.value.trim())) setHex(hex.value.trim(), true); });
  hex.addEventListener('keydown', (event: any) => event.stopPropagation());
  for (const key of ['h', 's', 'v']) channels[key].addEventListener('change', () => fromHsv({ ...hsv, [key]: channels[key].value }));
  for (const key of ['r', 'g', 'b']) channels[key].addEventListener('change', () => setHex(rgbToHex({ r: channels.r.value, g: channels.g.value, b: channels.b.value })));
  sync();

  const body = h('div.color-dialog',
    h('div.color-workbench', sv, hue, h('div.color-side', h('div.color-compare', newSwatch, oldSwatch), channelList)),
    h('div.color-dialog-value', hex), grid);
  let accepted = false;
  PM.modal({ title: label, body, width: 384, onClose: () => { if (!accepted) edit.cancel(); }, actions: [
    { label: 'Cancel' },
    { label: 'OK', pri: true, run: () => {
      if (!setHex(hex.value.trim())) { PM.toast('Enter a six-digit hex color'); return false; }
      accepted = true; edit.commit(chosen);
    } },
  ] });
  window.setTimeout(() => { hex.focus(); hex.select(); }, 30);
}

PM.fillField = (get: any, set: any, opt: any = {}) => {
  const fill = () => PM.normalizeFill(get(), opt.fallback || '#000000');
  const sw = h('div.sw');
  const val = h('span', { style: { fontFamily: 'var(--f-mono)', fontSize: 'var(--fs-md)' } });
  const button = h('button.color-field', { 'aria-label': opt.label || 'Fill' }, val, sw);
  button.sync = () => {
    const current = fill(); val.textContent = current.type === 'solid' ? current.stops[0].color : current.type;
    sw.style.setProperty('--sw-fill', fillCss(current));
  };
  button.onclick = () => openFillPicker(button, fill(), (next: any) => {
    once(set, next, opt, opt.label || 'Fill'); button.sync(); PM.invalidate('render');
  }, opt.label || 'Fill');
  button.sync(); return button;
};

function fillCss(fill: any) {
  if (fill.type === 'none') return 'transparent';
  if (fill.type === 'solid') return fill.stops[0].color;
  const stops = fill.stops.map((stop: any) => `${stop.color} ${stop.position}%`).join(',');
  return fill.type === 'radial' ? `radial-gradient(circle,${stops})` : `linear-gradient(${fill.angle}deg,${stops})`;
}

function openFillPicker(anchor: any, initial: any, apply: any, label: any) {
  let draft = PM.normalizeFill(initial), selected = draft.stops[0].id;
  const pickerWidth = 400;
  const layer = h('div.fill-picker-layer');
  const pop = h('section.fill-picker', { role: 'dialog', 'aria-modal': 'true', 'aria-label': label, tabindex: '-1' });
  const head = h('header', h('b', label), h('button.iconbtn', { 'aria-label': 'Close fill picker' }, PM.icon('x')));
  const modes = [['solid', 'Solid'], ['linear', 'Linear'], ['radial', 'Radial'], ['none', 'None']];
  const type = h('div.fill-types', { role: 'group', 'aria-label': 'Fill type' });
  modes.forEach(([value, text]: any) => type.appendChild(h('button.fill-type', {
    type: 'button', 'data-fill-type': value, 'aria-pressed': 'false', onclick: () => {
      draft = PM.normalizeFill({ ...draft, type: value }); selected = draft.stops[0].id; render();
    },
  }, text)));
  const preview = h('div.fill-preview', { 'aria-label': 'Fill preview' });
  const colors = ['#FF3B30','#FF9500','#FFCC00','#34C759','#00C7BE','#0A84FF','#5E5CE6','#BF5AF2','#FF2D55','#FFFFFF','#8E8E93','#09090A'];
  const sv = h('div.fill-sv', { role: 'slider', tabindex: '0', 'aria-label': 'Saturation and brightness', 'aria-valuemin': '0', 'aria-valuemax': '100' }, h('i'));
  const hue = h('div.fill-hue', { role: 'slider', tabindex: '0', 'aria-label': 'Hue', 'aria-valuemin': '0', 'aria-valuemax': '359' }, h('i'));
  const current = h('div.fill-current', { 'aria-label': 'Current color' });
  const channels: any = {};
  const channel = (key: any, label: any, max: any) => {
    const input = h('input', { type: 'number', min: '0', max: String(max), 'aria-label': label }); channels[key] = input;
    return h('label.fill-channel', h('span', label), input);
  };
  const hex = h('input.fill-hex', { 'aria-label': 'Hex color', spellcheck: 'false' });
  const channelGrid = h('div.fill-channels', { hidden: true }, channel('h', 'H', 359), channel('s', 'S', 100), channel('v', 'B', 100),
    channel('r', 'R', 255), channel('g', 'G', 255), channel('b', 'B', 255));
  const channelsButton = h('button.fill-channels-toggle', { type: 'button', 'aria-expanded': 'false' }, 'Channels');
  channelsButton.onclick = () => {
    channelGrid.hidden = !channelGrid.hidden;
    channelsButton.setAttribute('aria-expanded', String(!channelGrid.hidden));
    channelsButton.classList.toggle('on', !channelGrid.hidden);
  };
  const palette = h('div.fill-palette', { role: 'group', 'aria-label': 'Color swatches' });
  const colorWorkbench = h('div.fill-color-workbench', h('div.fill-color-main', sv, hue),
    h('div.fill-color-values', current, h('label.fill-hex-field', h('span', 'Hex'), hex), channelsButton), channelGrid,
    h('div.fill-palette-row', palette));
  const stops = h('div.fill-stops');
  const angle = h('input', { type: 'range', min: '-180', max: '180', value: draft.angle, 'aria-label': 'Gradient angle' });
  const angleValue = h('span.mono', `${draft.angle}°`);
  const angleRow = h('label.fill-angle', h('span', 'Angle'), angle, angleValue);
  const add = h('button.btn', 'Add stop');
  const cancelButton = h('button.btn', 'Cancel'), applyButton = h('button.btn.pri', 'Apply');
  const close = () => { layer.remove(); anchor.focus(); };
  head.lastChild.onclick = close; cancelButton.onclick = close;
  layer.addEventListener('pointerdown', (event: any) => { if (event.target === layer) close(); });
  layer.addEventListener('keydown', (event: any) => {
    if (event.key === 'Escape') { event.preventDefault(); close(); return; }
    if (event.key !== 'Tab') return;
    const focusable = [...pop.querySelectorAll('button:not([disabled]):not([hidden]),input:not([disabled]):not([hidden]),select:not([disabled]):not([hidden])')]
      .filter((node: any) => node.offsetParent !== null);
    if (!focusable.length) return;
    const first = focusable[0], last = focusable.at(-1);
    if (event.shiftKey && window.document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && window.document.activeElement === last) { event.preventDefault(); first.focus(); }
  });
  const selectedStop = () => draft.stops.find((stop: any) => stop.id === selected) || draft.stops[0];
  let hsv = rgbToHsv(hexToRgb(selectedStop().color));
  function setSelectedColor(value: any, redraw: any = true) {
    const valid = normalizeHex(value); if (!valid) return false;
    selectedStop().color = valid; hsv = rgbToHsv(hexToRgb(valid));
    if (redraw) render(); else syncColorEditor();
    return true;
  }
  function syncColorEditor() {
    const rgb = hexToRgb(selectedStop().color) || { r: 0, g: 0, b: 0 };
    hsv = rgbToHsv(rgb);
    channels.h.value = hsv.h; channels.s.value = hsv.s; channels.v.value = hsv.v;
    channels.r.value = rgb.r; channels.g.value = rgb.g; channels.b.value = rgb.b; hex.value = rgbToHex(rgb);
    const pureHue = rgbToHex(hsvToRgb({ h: hsv.h, s: 100, v: 100 }));
    sv.style.setProperty('--hue-color', pureHue); sv.firstChild.style.left = `${hsv.s}%`; sv.firstChild.style.top = `${100 - hsv.v}%`;
    hue.firstChild.style.top = `${hsv.h / 359 * 100}%`; current.style.setProperty('--sw-color', selectedStop().color);
    sv.setAttribute('aria-valuetext', `${hsv.s}% saturation, ${hsv.v}% brightness`); hue.setAttribute('aria-valuenow', hsv.h);
  }
  function fromHsv(next: any) { hsv = { h: clampChannel(next.h, 359), s: clampChannel(next.s, 100), v: clampChannel(next.v, 100) }; setSelectedColor(rgbToHex(hsvToRgb(hsv))); }
  function pickSv(event: any) {
    const rect = sv.getBoundingClientRect();
    fromHsv({ h: hsv.h, s: (event.clientX - rect.left) / rect.width * 100, v: 100 - (event.clientY - rect.top) / rect.height * 100 });
  }
  function pickHue(event: any) {
    const rect = hue.getBoundingClientRect(); fromHsv({ ...hsv, h: (event.clientY - rect.top) / rect.height * 359 });
  }
  const dragColor = (element: any, picker: any) => element.addEventListener('pointerdown', (event: any) => {
    if (event.button !== 0) return; event.preventDefault(); picker(event);
    PM.drag(event, { move: (dx: any, dy: any, next: any) => picker(next), up: () => {} });
  });
  dragColor(sv, pickSv); dragColor(hue, pickHue);
  sv.addEventListener('keydown', (event: any) => {
    if (!['ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(event.key)) return; event.preventDefault();
    fromHsv({ h: hsv.h, s: hsv.s + (event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0), v: hsv.v + (event.key === 'ArrowUp' ? 1 : event.key === 'ArrowDown' ? -1 : 0) });
  });
  hue.addEventListener('keydown', (event: any) => {
    if (!['ArrowUp','ArrowDown'].includes(event.key)) return; event.preventDefault(); fromHsv({ ...hsv, h: hsv.h + (event.key === 'ArrowDown' ? 1 : -1) });
  });
  colors.forEach((color: any) => {
    const choice = h('button.fill-spectrum-color', { 'aria-label': color, title: color, onclick: () => setSelectedColor(color) });
    choice.style.setProperty('--sw-color', color); palette.appendChild(choice);
  });
  for (const key of ['h','s','v']) channels[key].addEventListener('change', () => fromHsv({ h: channels.h.value, s: channels.s.value, v: channels.v.value }));
  for (const key of ['r','g','b']) channels[key].addEventListener('change', () => setSelectedColor(rgbToHex({ r: channels.r.value, g: channels.g.value, b: channels.b.value })));
  hex.addEventListener('change', () => { if (!setSelectedColor(hex.value)) { PM.toast('Enter a three- or six-digit hex color'); syncColorEditor(); } });
  function render() {
    type.querySelectorAll('.fill-type').forEach((button: any) => {
      const on = button.dataset.fillType === draft.type; button.classList.toggle('on', on); button.setAttribute('aria-pressed', String(on));
    });
    preview.style.background = fillCss(draft);
    const noColor = draft.type === 'none'; colorWorkbench.hidden = noColor; preview.hidden = noColor; stops.hidden = draft.type === 'solid' || noColor;
    angleRow.hidden = draft.type !== 'linear'; add.hidden = draft.type === 'solid' || noColor; angle.value = draft.angle; angleValue.textContent = `${draft.angle}°`;
    stops.textContent = '';
    draft.stops.forEach((stop: any, index: any) => {
      const color = h('input.fill-stop-color', { value: stop.color, 'aria-label': `Stop ${index + 1} color` });
      const position = h('input', { type: 'range', min: '0', max: '100', value: stop.position, 'aria-label': `Stop ${index + 1} position` });
      const row = h('div.fill-stop' + (stop.id === selected ? '.on' : ''),
        h('button.fill-stop-swatch', { 'aria-label': `Select stop ${index + 1}`, onclick: () => { selected = stop.id; render(); } }),
        color, position, h('span.mono', `${stop.position}%`),
        h('button.iconbtn.fill-stop-remove', { title: 'Remove stop', 'aria-label': `Remove stop ${index + 1}`, disabled: draft.stops.length <= 2, onclick: () => { draft.stops.splice(index, 1); selected = draft.stops[Math.max(0, index - 1)].id; render(); } }, PM.icon('x')));
      row.firstChild.style.setProperty('--sw-color', stop.color);
      color.oninput = () => { if (/^#[0-9a-f]{6}$/i.test(color.value)) { stop.color = color.value.toUpperCase(); render(); } };
      position.oninput = () => { stop.position = +position.value; row.querySelector('.mono').textContent = `${stop.position}%`; preview.style.background = fillCss(draft); };
      position.onchange = () => { draft.stops.sort((a: any, b: any) => a.position - b.position); render(); };
      stops.appendChild(row);
    });
    if (!noColor) syncColorEditor();
  }
  angle.oninput = () => { draft.angle = +angle.value; render(); };
  add.onclick = () => {
    if (draft.stops.length >= 8) return;
    const prior = selectedStop(); draft.stops.push({ id: PM.uid('stop'), color: prior.color, position: Math.min(100, prior.position + 10) });
    selected = draft.stops.at(-1).id; render();
  };
  applyButton.onclick = () => { apply(PM.normalizeFill(draft)); close(); };
  pop.append(head, h('div.fill-picker-body', type, preview, colorWorkbench, stops, h('div.fill-picker-tools', angleRow, add)),
    h('footer', cancelButton, applyButton)); layer.appendChild(pop); window.document.body.appendChild(layer);
  const rect = anchor.getBoundingClientRect();
  const pickerHeight = Math.min(560, window.innerHeight - 64);
  Object.assign(pop.style, { left: `${PM.clamp(rect.right - pickerWidth, 12, window.innerWidth - pickerWidth - 12)}px`, top: `${PM.clamp(rect.bottom + 6, 52, window.innerHeight - pickerHeight - 12)}px` });
  render(); window.requestAnimationFrame(() => type.querySelector('.fill-type.on')?.focus());
}

PM.toggleField = (get: any, set: any, opt: any = {}) => {
  const t = h('div.toggle' + (get() ? '.on' : ''), h('i'));
  t.sync = () => t.classList.toggle('on', !!get());
  t.addEventListener('pointerdown', (e: any) => {
    e.stopPropagation();
    once(set, !get(), opt, opt.label || 'Toggle');
    t.sync(); PM.invalidate();
  });
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
