import type { PowermoveAPI } from 'powermove';

/*
 * The light tool's panel, under the toolbar while the tool is on:
 * - how pointing aims the light (reflect, direct, orbit);
 * - Add, and the kind of light it adds;
 * - the selected light's brightness, size, distance and spread. A slider moves
 *   the light live; releasing it is one undoable edit. Distance keeps the light
 *   on the product as bright.
 * Plain DOM, refreshed from the light tool (like the viewer's other overlays).
 */

type Lights = PowermoveAPI['scene3d']['lights'];
type Info = NonNullable<ReturnType<Lights['info']>>;
type What = 'power' | 'size' | 'distance' | 'spread';
type Row = { what: What; label: string; tip: string; min: number; max: number; log: boolean; unit: string; show: (i: Info) => boolean; value: (i: Info) => number };

const AIMS = [
  { id: 'reflect', label: 'Reflect', tip: 'Reflect — The highlight lands where you click' },
  { id: 'direct', label: 'Direct', tip: 'Direct — The light faces where you click' },
  { id: 'orbit', label: 'Orbit', tip: 'Orbit — Drag to circle the light around its target' },
] as const;
const KINDS = [{ id: 'area', label: 'Softbox' }, { id: 'spot', label: 'Spotlight' }, { id: 'point', label: 'Bulb' }, { id: 'sun', label: 'Sun' }] as const;
const KIND: Record<string, string> = { area: 'Softbox', spot: 'Spotlight', point: 'Bulb', sun: 'Sun' };
// Sliders run 0…1000; brightness, size and distance on a log scale (equal steps feel equal).
const ROWS: Row[] = [
  { what: 'power', label: 'Brightness', tip: 'How much light it gives', min: 0.05, max: 2000, log: true, unit: '', show: () => true, value: i => i.power },
  { what: 'size', label: 'Size', tip: 'Bigger softboxes give softer light and broader highlights', min: 0.02, max: 40, log: true, unit: '', show: i => i.size !== null, value: i => i.size ?? 1 },
  { what: 'distance', label: 'Distance', tip: 'How far it sits from what it lights. Brightness on the product stays the same', min: 0.1, max: 200, log: true, unit: '', show: i => i.type !== 'sun', value: i => i.distance },
  { what: 'spread', label: 'Spread', tip: 'How wide the spotlight’s beam is', min: 1, max: 89, log: false, unit: '°', show: i => i.spread !== null, value: i => i.spread ?? 45 },
];
const toSlider = (r: Row, v: number) => r.log
  ? 1000 * Math.log(Math.max(r.min, Math.min(r.max, v)) / r.min) / Math.log(r.max / r.min)
  : 1000 * (v - r.min) / (r.max - r.min);
const fromSlider = (r: Row, s: number) => r.log ? r.min * Math.pow(r.max / r.min, s / 1000) : r.min + (r.max - r.min) * s / 1000;
const fmt = (n: number) => n >= 100 ? n.toFixed(0) : n >= 10 ? n.toFixed(1) : n.toFixed(2);

const CSS = `
#light-panel [hidden]{display:none!important}
#light-panel .lp{width:244px;display:flex;flex-direction:column;gap:6px;padding:8px;border-radius:10px;background:var(--bg-float);box-shadow:var(--shadow-float);color:var(--tx);font-size:var(--fs-sm)}
#light-panel .lp-seg{display:flex;gap:1px;padding:2px;border-radius:var(--r-md);background:var(--ink-2)}
#light-panel .lp-seg button{flex:1;height:calc(var(--ctl-h) - 4px);border:0;border-radius:calc(var(--r-md) - 2px);background:transparent;color:var(--tx-2);font:inherit;font-weight:var(--fw-medium);cursor:default}
#light-panel .lp-seg button:hover{color:var(--tx)}
#light-panel .lp-seg button.on{background:var(--ctl-raised);box-shadow:var(--ctl-edge);color:var(--tx)}
#light-panel .lp-seg select{flex:1;height:calc(var(--ctl-h) - 4px);padding:0 6px;border:0;border-radius:calc(var(--r-md) - 2px);background:transparent;color:var(--tx-2);font:inherit;font-weight:var(--fw-medium)}
#light-panel .lp-head{display:flex;align-items:baseline;justify-content:space-between;gap:8px;padding:4px 2px 0}
#light-panel .lp-head b{font-weight:var(--fw-semibold);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
#light-panel .lp-head span{color:var(--tx-3);font-size:var(--fs-xs)}
#light-panel .lp-row{display:grid;grid-template-columns:66px 1fr 38px;align-items:center;gap:8px;height:24px;padding:0 2px}
#light-panel .lp-name,#light-panel .lp-val{color:var(--tx-2)}
#light-panel .lp-val{text-align:right;font-variant-numeric:tabular-nums}
#light-panel .lp-hint{margin:2px 2px 0;color:var(--tx-3);line-height:1.4}
#light-panel input[type=range]{-webkit-appearance:none;appearance:none;width:100%;height:16px;margin:0;background:transparent}
#light-panel input[type=range]::-webkit-slider-runnable-track{height:4px;border-radius:2px;background:linear-gradient(to right,var(--accent) var(--fill),var(--ink-3) var(--fill))}
#light-panel input[type=range]::-webkit-slider-thumb{-webkit-appearance:none;width:12px;height:12px;margin-top:-4px;border-radius:50%;background:#fff;box-shadow:0 0 0 .5px rgba(0,0,0,.25),0 1px 3px rgba(0,0,0,.25)}
#light-panel input[type=range]:focus-visible{outline:none}
#light-panel input[type=range]:focus-visible::-webkit-slider-thumb{box-shadow:var(--focus-ring)}
`;

const h = <K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> & Record<string, unknown> = {}, ...children: (Node | string)[]) => {
  const el = document.createElement(tag);
  Object.assign(el, props);
  el.append(...children);
  return el;
};

/** The panel in `host`; returns its disposer. */
export function mountLightPanel(api: PowermoveAPI, host: HTMLElement): () => void {
  const lights = api.scene3d.lights;
  host.append(h('style', { textContent: CSS }));
  const panel = h('div', { className: 'lp', role: 'group' } as any);
  panel.setAttribute('aria-label', 'Light tool');
  host.append(panel);

  // Aim: three buttons.
  const seg = h('div', { className: 'lp-seg' });
  seg.setAttribute('role', 'radiogroup'); seg.setAttribute('aria-label', 'Aim');
  const aimButtons = AIMS.map(aim => {
    const b = h('button', { textContent: aim.label, title: aim.tip, onclick: () => lights.setTool({ aim: aim.id }) });
    b.setAttribute('role', 'radio');
    seg.append(b);
    return { id: aim.id, b };
  });
  // Add, and what it adds.
  const add = h('div', { className: 'lp-seg' });
  const addButton = h('button', { textContent: '+ Add', title: 'Add light — Then click a model to put it there', onclick: () => lights.setTool({ adding: !lights.getTool().adding }) });
  const kind = h('select', { title: 'The kind of light Add puts in' });
  kind.setAttribute('aria-label', 'Kind of light to add');
  for (const k of KINDS) kind.append(h('option', { value: k.id, textContent: k.label }));
  kind.onchange = () => lights.setTool({ kind: kind.value as any, adding: true });
  add.append(addButton, kind);
  // The selected light, or a hint.
  const headName = h('b'), headKind = h('span');
  const head = h('div', { className: 'lp-head' }, headName, headKind);
  const hint = h('p', { className: 'lp-hint' });
  let dragging = false;
  const rows = ROWS.map(r => {
    const input = h('input', { type: 'range', min: '0', max: '1000', step: '1' });
    const val = h('span', { className: 'lp-val' });
    const row = h('label', { className: 'lp-row', title: r.tip }, h('span', { className: 'lp-name', textContent: r.label }), input, val);
    input.oninput = () => { dragging = true; const next = lights.set(r.what, fromSlider(r, Number(input.value))); if (next) paint(next, false); };
    const release = () => { if (!dragging) return; dragging = false; lights.commit(); refresh(); };
    input.onchange = release; input.onpointerup = release; input.onkeyup = release;
    return { r, row, input, val };
  });
  panel.append(seg, add, head, ...rows.map(x => x.row), hint);

  /** Show `info`'s values; `sliders` false while one is being dragged (leave it where the pointer is). */
  function paint(info: Info | null, sliders = true) {
    const tool = lights.getTool();
    for (const { id, b } of aimButtons) { b.classList.toggle('on', tool.aim === id); b.setAttribute('aria-checked', String(tool.aim === id)); }
    addButton.classList.toggle('on', tool.adding); addButton.setAttribute('aria-pressed', String(tool.adding));
    if (kind.value !== tool.kind) kind.value = tool.kind;
    const showLight = !!info && !tool.adding;
    head.hidden = !showLight; hint.hidden = showLight;
    hint.textContent = tool.adding ? `Click a model to add a ${KIND[tool.kind]!.toLowerCase()} there.` : 'Select a light to aim it, or Add one.';
    if (info) { headName.textContent = info.name; headKind.textContent = KIND[info.type] ?? ''; }
    for (const { r, row, input, val } of rows) {
      const shown = showLight && r.show(info!);
      row.hidden = !shown;
      if (!shown) continue;
      const v = r.value(info!), s = toSlider(r, v);
      if (sliders || document.activeElement !== input) input.value = String(Math.round(s));
      input.style.setProperty('--fill', `${Number(input.value) / 10}%`);
      val.textContent = `${fmt(v)}${r.unit}`;
    }
  }
  function refresh() {
    const tool = lights.getTool();
    host.hidden = !tool.on;
    if (tool.on) paint(lights.info(), !dragging);
  }
  refresh();
  const watch = lights.onToolChange(refresh);
  // Selection, undo and the timeline change lights without telling the tool: follow them.
  const timer = window.setInterval(() => { if (!dragging && lights.getTool().on) refresh(); }, 200);
  return () => { watch.dispose(); window.clearInterval(timer); host.replaceChildren(); };
}
