/* Ported from js/core/util.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
/* Powermove — util: dom, bus, math, time, persistence. */

PM.version = '1.0.0';
PM.bootVersion = 20260816;

/* ── dom ───────────────────────────────────────────────── */
const h: any = (tag: any, attrs: any, ...kids: any[]) => {
  const parts: any = tag.split(/([.#])/);
  const el = window.document.createElement(parts[0] || 'div');
  for (let i = 1; i < parts.length; i += 2) {
    if (parts[i] === '.') el.classList.add(parts[i + 1]);
    else el.id = parts[i + 1];
  }
  if (attrs && (attrs.nodeType || typeof attrs === 'string')) { kids.unshift(attrs); attrs = null; }
  if (attrs) for (const k in attrs) {
    const v = attrs[k];
    if (v == null || v === false) continue;
    if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
    else if (k === 'class') el.className += (el.className ? ' ' : '') + v;
    else if (k === 'html') el.innerHTML = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (k === 'data') for (const d in v) el.dataset[d] = v[d];
    else el.setAttribute(k, v === true ? '' : v);
  }
  const add = (k: any) => {
    if (k == null || k === false) return;
    if (Array.isArray(k)) return k.forEach(add);
    el.appendChild(k.nodeType ? k : window.document.createTextNode(k));
  };
  kids.forEach(add);
  return el;
};
const $: any = (s: any, r: any = window.document) => r.querySelector(s);
const $$: any = (s: any, r: any = window.document) => [...r.querySelectorAll(s)];
PM.h = h; PM.$ = $; PM.$$ = $$;

/* Native log bridge: surface webview errors on the app's stderr so headless
   debugging works without Safari Web Inspector. */
const __pmLog = (tag: any, args: any) => {
  try {
    const mh = (window as any).webkit && (window as any).webkit.messageHandlers && (window as any).webkit.messageHandlers.pmLog;
    if (mh) mh.postMessage(tag + ' ' + args.map((a: any) => {
      try { return a instanceof Error ? (a.message + '\n' + (a.stack || '')) : (typeof a === 'object' ? JSON.stringify(a) : String(a)); } catch { return String(a); }
    }).join(' '));
  } catch (e) { }
};
const __cerr = window.console.error.bind(window.console), __cwarn = window.console.warn.bind(window.console);
window.console.error = (...a) => { __pmLog('[error]', a); __cerr(...a); };
window.console.warn = (...a) => { __pmLog('[warn]', a); __cwarn(...a); };
window.addEventListener('error', (e) => __pmLog('[uncaught]', [e.message + ' @ ' + (e.filename || '') + ':' + (e.lineno || 0)]));
window.addEventListener('unhandledrejection', (e) => __pmLog('[rejection]', [String(e.reason)]));

PM.svg = (d: any, box = 256) => {
  const s = window.document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('viewBox', `0 0 ${box} ${box}`);
  s.classList.add('pm-icon');
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('focusable', 'false');
  s.style.fill = 'currentColor';
  s.style.stroke = 'none';
  s.innerHTML = d;
  return s;
};
PM.icon = (name: any) => {
  const found = PM.ICONS[name];
  if (!found) window.console.warn('Unknown Powermove icon:', name);
  const s = PM.svg(found || PM.ICONS.missing);
  s.dataset.icon = found ? name : 'missing';
  s.dataset.iconSet = 'phosphor';
  return s;
};
/* Phosphor Icons 2.1.1, regular weight. MIT licensed. */
PM.ICONS = {
  music: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M56 128v0m0-16v32m36-64v96m36-128v160m36-128v96m36-64v32"/></g>',
  film: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><rect x="32" y="40" width="192" height="176" rx="16"/><path d="M72 40v176m112-176v176M32 88h40m-40 80h40m112-80h40m-40 80h40m-116-68 48 28-48 28z"/></g>',
  image: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><rect x="32" y="40" width="192" height="176" rx="16"/><circle cx="92" cy="92" r="16"/><path d="m32 176 56-48 40 32 40-48 56 64"/></g>',
  puzzle: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M48 48h56a24 24 0 1 1 48 0h56v56a24 24 0 1 1 0 48v56h-56a24 24 0 1 0-48 0H48v-56a24 24 0 1 0 0-48z"/></g>',
  note: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M160 32H48v192h160V80zM160 32v48h48M80 120h96M80 152h96M80 184h64"/></g>',
  speedometer: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M48 200a104 104 0 1 1 160 0zM128 136l48-64M56 144h16m0-64 16 16m40-56v24m56 80h16"/><circle cx="128" cy="144" r="12"/></g>',
  sliders: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M48 40v56m0 32v88m80-176v88m0 32v56m80-176v24m0 32v120M24 96h48v32H24zm80 32h48v32h-48zm80-64h48v32h-48z"/></g>',
  timeline: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M40 32v192m-16-56h16m-16-80h16"/><rect x="72" y="48" width="144" height="32" rx="8"/><rect x="72" y="112" width="88" height="32" rx="8"/><rect x="112" y="176" width="104" height="32" rx="8"/></g>',
  tools: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="m152 48 32 32 40-40a64 64 0 0 1-80 80L64 200a24 24 0 0 1-34-34l80-80a64 64 0 0 1 80-80z"/></g>',
  dot: '<path d="M140,128a12,12,0,1,1-12-12A12,12,0,0,1,140,128Z"/>',
  missing: '<path d="M140,180a12,12,0,1,1-12-12A12,12,0,0,1,140,180ZM128,72c-22.06,0-40,16.15-40,36v4a8,8,0,0,0,16,0v-4c0-11,10.77-20,24-20s24,9,24,20-10.77,20-24,20a8,8,0,0,0-8,8v8a8,8,0,0,0,16,0v-.72c18.24-3.35,32-17.9,32-35.28C168,88.15,150.06,72,128,72Zm104,56A104,104,0,1,1,128,24,104.11,104.11,0,0,1,232,128Zm-16,0a88,88,0,1,0-88,88A88.1,88.1,0,0,0,216,128Z"/>',
  play: '<path d="M232.4,114.49,88.32,26.35a16,16,0,0,0-16.2-.3A15.86,15.86,0,0,0,64,39.87V216.13A15.94,15.94,0,0,0,80,232a16.07,16.07,0,0,0,8.36-2.35L232.4,141.51a15.81,15.81,0,0,0,0-27ZM80,215.94V40l143.83,88Z"/>',
  pause: '<path d="M200,32H160a16,16,0,0,0-16,16V208a16,16,0,0,0,16,16h40a16,16,0,0,0,16-16V48A16,16,0,0,0,200,32Zm0,176H160V48h40ZM96,32H56A16,16,0,0,0,40,48V208a16,16,0,0,0,16,16H96a16,16,0,0,0,16-16V48A16,16,0,0,0,96,32Zm0,176H56V48H96Z"/>',
  prev: '<path d="M199.81,34a16,16,0,0,0-16.24.43L64,109.23V40a8,8,0,0,0-16,0V216a8,8,0,0,0,16,0V146.77l119.57,74.78A15.95,15.95,0,0,0,208,208.12V47.88A15.86,15.86,0,0,0,199.81,34ZM192,208,64.16,128,192,48.07Z"/>',
  next: '<path d="M200,32a8,8,0,0,0-8,8v69.23L72.43,34.45A15.95,15.95,0,0,0,48,47.88V208.12a16,16,0,0,0,24.43,13.43L192,146.77V216a8,8,0,0,0,16,0V40A8,8,0,0,0,200,32ZM64,207.93V48.05l127.84,80Z"/>',
  home: '<path d="M219.31,108.68l-80-80a16,16,0,0,0-22.62,0l-80,80A15.87,15.87,0,0,0,32,120v96a8,8,0,0,0,8,8h64a8,8,0,0,0,8-8V160h32v56a8,8,0,0,0,8,8h64a8,8,0,0,0,8-8V120A15.87,15.87,0,0,0,219.31,108.68ZM208,208H160V152a8,8,0,0,0-8-8H104a8,8,0,0,0-8,8v56H48V120l80-80,80,80Z"/>',
  project: '<path d="M216,72H131.31L104,44.69A15.86,15.86,0,0,0,92.69,40H40A16,16,0,0,0,24,56V200.62A15.4,15.4,0,0,0,39.38,216H216.89A15.13,15.13,0,0,0,232,200.89V88A16,16,0,0,0,216,72ZM40,56H92.69l16,16H40ZM216,200H40V88H216Z"/>',
  plus: '<path d="M224,128a8,8,0,0,1-8,8H136v80a8,8,0,0,1-16,0V136H40a8,8,0,0,1,0-16h80V40a8,8,0,0,1,16,0v80h80A8,8,0,0,1,224,128Z"/>',
  x: '<path d="M205.66,194.34a8,8,0,0,1-11.32,11.32L128,139.31,61.66,205.66a8,8,0,0,1-11.32-11.32L116.69,128,50.34,61.66A8,8,0,0,1,61.66,50.34L128,116.69l66.34-66.35a8,8,0,0,1,11.32,11.32L139.31,128Z"/>',
  undo: '<path d="M224,128a96,96,0,0,1-94.71,96H128A95.38,95.38,0,0,1,62.1,197.8a8,8,0,0,1,11-11.63A80,80,0,1,0,71.43,71.39a3.07,3.07,0,0,1-.26.25L44.59,96H72a8,8,0,0,1,0,16H24a8,8,0,0,1-8-8V56a8,8,0,0,1,16,0V85.8L60.25,60A96,96,0,0,1,224,128Z"/>',
  redo: '<path d="M240,56v48a8,8,0,0,1-8,8H184a8,8,0,0,1,0-16H211.4L184.81,71.64l-.25-.24a80,80,0,1,0-1.67,114.78,8,8,0,0,1,11,11.63A95.44,95.44,0,0,1,128,224h-1.32A96,96,0,1,1,195.75,60L224,85.8V56a8,8,0,1,1,16,0Z"/>',
  panel: '<path d="M216,40H40A16,16,0,0,0,24,56V200a16,16,0,0,0,16,16H216a16,16,0,0,0,16-16V56A16,16,0,0,0,216,40ZM40,56H80V200H40ZM216,200H96V56H216V200Z"/>',
  panelL: '<path d="M216,40H40A16,16,0,0,0,24,56V200a16,16,0,0,0,16,16H216a16,16,0,0,0,16-16V56A16,16,0,0,0,216,40ZM40,152H56a8,8,0,0,0,0-16H40V120H56a8,8,0,0,0,0-16H40V88H56a8,8,0,0,0,0-16H40V56H80V200H40Zm176,48H96V56H216V200Z"/>',
  grip: '<path d="M104,60A12,12,0,1,1,92,48,12,12,0,0,1,104,60Zm60,12a12,12,0,1,0-12-12A12,12,0,0,0,164,72ZM92,116a12,12,0,1,0,12,12A12,12,0,0,0,92,116Zm72,0a12,12,0,1,0,12,12A12,12,0,0,0,164,116ZM92,184a12,12,0,1,0,12,12A12,12,0,0,0,92,184Zm72,0a12,12,0,1,0,12,12A12,12,0,0,0,164,184Z"/>',
  up: '<path d="M205.66,117.66a8,8,0,0,1-11.32,0L136,59.31V216a8,8,0,0,1-16,0V59.31L61.66,117.66a8,8,0,0,1-11.32-11.32l72-72a8,8,0,0,1,11.32,0l72,72A8,8,0,0,1,205.66,117.66Z"/>',
  export: '<path d="M216,112v96a16,16,0,0,1-16,16H56a16,16,0,0,1-16-16V112A16,16,0,0,1,56,96H80a8,8,0,0,1,0,16H56v96H200V112H176a8,8,0,0,1,0-16h24A16,16,0,0,1,216,112ZM93.66,69.66,120,43.31V136a8,8,0,0,0,16,0V43.31l26.34,26.35a8,8,0,0,0,11.32-11.32l-40-40a8,8,0,0,0-11.32,0l-40,40A8,8,0,0,0,93.66,69.66Z"/>',
  clock: '<path d="M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm0,192a88,88,0,1,1,88-88A88.1,88.1,0,0,1,128,216Zm64-88a8,8,0,0,1-8,8H128a8,8,0,0,1-8-8V72a8,8,0,0,1,16,0v48h48A8,8,0,0,1,192,128Z"/>',
  search: '<path d="M229.66,218.34l-50.07-50.06a88.11,88.11,0,1,0-11.31,11.31l50.06,50.07a8,8,0,0,0,11.32-11.32ZM40,112a72,72,0,1,1,72,72A72.08,72.08,0,0,1,40,112Z"/>',
  list: '<path d="M80,64a8,8,0,0,1,8-8H216a8,8,0,0,1,0,16H88A8,8,0,0,1,80,64Zm136,56H88a8,8,0,0,0,0,16H216a8,8,0,0,0,0-16Zm0,64H88a8,8,0,0,0,0,16H216a8,8,0,0,0,0-16ZM44,52A12,12,0,1,0,56,64,12,12,0,0,0,44,52Zm0,64a12,12,0,1,0,12,12A12,12,0,0,0,44,116Zm0,64a12,12,0,1,0,12,12A12,12,0,0,0,44,180Z"/>',
  trash: '<path d="M216,48H176V40a24,24,0,0,0-24-24H104A24,24,0,0,0,80,40v8H40a8,8,0,0,0,0,16h8V208a16,16,0,0,0,16,16H192a16,16,0,0,0,16-16V64h8a8,8,0,0,0,0-16ZM96,40a8,8,0,0,1,8-8h48a8,8,0,0,1,8,8v8H96Zm96,168H64V64H192ZM112,104v64a8,8,0,0,1-16,0V104a8,8,0,0,1,16,0Zm48,0v64a8,8,0,0,1-16,0V104a8,8,0,0,1,16,0Z"/>',
  more: '<path d="M140,128a12,12,0,1,1-12-12A12,12,0,0,1,140,128Zm56-12a12,12,0,1,0,12,12A12,12,0,0,0,196,116ZM60,116a12,12,0,1,0,12,12A12,12,0,0,0,60,116Z"/>',
  chev: '<path d="M181.66,133.66l-80,80a8,8,0,0,1-11.32-11.32L164.69,128,90.34,53.66a8,8,0,0,1,11.32-11.32l80,80A8,8,0,0,1,181.66,133.66Z"/>',
  chevD: '<path d="M213.66,101.66l-80,80a8,8,0,0,1-11.32,0l-80-80A8,8,0,0,1,53.66,90.34L128,164.69l74.34-74.35a8,8,0,0,1,11.32,11.32Z"/>',
  eye: '<path d="M247.31,124.76c-.35-.79-8.82-19.58-27.65-38.41C194.57,61.26,162.88,48,128,48S61.43,61.26,36.34,86.35C17.51,105.18,9,124,8.69,124.76a8,8,0,0,0,0,6.5c.35.79,8.82,19.57,27.65,38.4C61.43,194.74,93.12,208,128,208s66.57-13.26,91.66-38.34c18.83-18.83,27.3-37.61,27.65-38.4A8,8,0,0,0,247.31,124.76ZM128,192c-30.78,0-57.67-11.19-79.93-33.25A133.47,133.47,0,0,1,25,128,133.33,133.33,0,0,1,48.07,97.25C70.33,75.19,97.22,64,128,64s57.67,11.19,79.93,33.25A133.46,133.46,0,0,1,231.05,128C223.84,141.46,192.43,192,128,192Zm0-112a48,48,0,1,0,48,48A48.05,48.05,0,0,0,128,80Zm0,80a32,32,0,1,1,32-32A32,32,0,0,1,128,160Z"/>',
  lock: '<path d="M208,80H176V56a48,48,0,0,0-96,0V80H48A16,16,0,0,0,32,96V208a16,16,0,0,0,16,16H208a16,16,0,0,0,16-16V96A16,16,0,0,0,208,80ZM96,56a32,32,0,0,1,64,0V80H96ZM208,208H48V96H208V208Zm-68-56a12,12,0,1,1-12-12A12,12,0,0,1,140,152Z"/>',
  link: '<path d="M240,88.23a54.43,54.43,0,0,1-16,37L189.25,160a54.27,54.27,0,0,1-38.63,16h-.05A54.63,54.63,0,0,1,96,119.84a8,8,0,0,1,16,.45A38.62,38.62,0,0,0,150.58,160h0a38.39,38.39,0,0,0,27.31-11.31l34.75-34.75a38.63,38.63,0,0,0-54.63-54.63l-11,11A8,8,0,0,1,135.7,59l11-11A54.65,54.65,0,0,1,224,48,54.86,54.86,0,0,1,240,88.23ZM109,185.66l-11,11A38.41,38.41,0,0,1,70.6,208h0a38.63,38.63,0,0,1-27.29-65.94L78,107.31A38.63,38.63,0,0,1,144,135.71a8,8,0,0,0,16,.45A54.86,54.86,0,0,0,144,96a54.65,54.65,0,0,0-77.27,0L32,130.75A54.62,54.62,0,0,0,70.56,224h0a54.28,54.28,0,0,0,38.64-16l11-11A8,8,0,0,0,109,185.66Z"/>',
  magnet: '<path d="M207,50.25A87.46,87.46,0,0,0,144.6,24h-.33A87.48,87.48,0,0,0,82,49.81L20.61,112a16,16,0,0,0,.06,22.56l28.66,28.66a15.92,15.92,0,0,0,11.32,4.69h.09a16,16,0,0,0,11.36-4.82L133,100.69a16.08,16.08,0,0,1,22.41-.21,15.6,15.6,0,0,1,4.73,11.19,16.89,16.89,0,0,1-4.85,12L93,183.88a16,16,0,0,0-.17,22.79l28.66,28.66a16.06,16.06,0,0,0,22.52.12L205.81,175C240.26,140.5,240.79,84.56,207,50.25ZM60.65,151.89,32,123.24,55.8,99.12l28.52,28.52ZM132.79,224l-28.68-28.65,24.38-23.57L157,200.32Zm61.76-60.44-26.11,25.54L140,160.68l26.44-25.57.1-.09a33,33,0,0,0,9.57-23.5A31.44,31.44,0,0,0,166.47,89a32.2,32.2,0,0,0-44.9.5L95.49,116.18,67,87.74,93.35,61.09A71.51,71.51,0,0,1,144.27,40h.27a71.55,71.55,0,0,1,51.05,21.48C223.25,89.55,222.75,135.38,194.55,163.58Z"/>',
  layers: '<path d="M230.91,172A8,8,0,0,1,228,182.91l-96,56a8,8,0,0,1-8.06,0l-96-56A8,8,0,0,1,36,169.09l92,53.65,92-53.65A8,8,0,0,1,230.91,172ZM220,121.09l-92,53.65L36,121.09A8,8,0,0,0,28,134.91l96,56a8,8,0,0,0,8.06,0l96-56A8,8,0,1,0,220,121.09ZM24,80a8,8,0,0,1,4-6.91l96-56a8,8,0,0,1,8.06,0l96,56a8,8,0,0,1,0,13.82l-96,56a8,8,0,0,1-8.06,0l-96-56A8,8,0,0,1,24,80Zm23.88,0L128,126.74,208.12,80,128,33.26Z"/>',
  wand: '<path d="M48,64a8,8,0,0,1,8-8H72V40a8,8,0,0,1,16,0V56h16a8,8,0,0,1,0,16H88V88a8,8,0,0,1-16,0V72H56A8,8,0,0,1,48,64ZM184,192h-8v-8a8,8,0,0,0-16,0v8h-8a8,8,0,0,0,0,16h8v8a8,8,0,0,0,16,0v-8h8a8,8,0,0,0,0-16Zm56-48H224V128a8,8,0,0,0-16,0v16H192a8,8,0,0,0,0,16h16v16a8,8,0,0,0,16,0V160h16a8,8,0,0,0,0-16ZM219.31,80,80,219.31a16,16,0,0,1-22.62,0L36.68,198.63a16,16,0,0,1,0-22.63L176,36.69a16,16,0,0,1,22.63,0l20.68,20.68A16,16,0,0,1,219.31,80Zm-54.63,32L144,91.31l-96,96L68.68,208ZM208,68.69,187.31,48l-32,32L176,100.69Z"/>',
  grid: '<path d="M104,40H56A16,16,0,0,0,40,56v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,104,40Zm0,64H56V56h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,200,40Zm0,64H152V56h48v48Zm-96,32H56a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,104,136Zm0,64H56V152h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,200,136Zm0,64H152V152h48v48Z"/>',
  cursor: '<path d="M168,132.69,214.08,115l.33-.13A16,16,0,0,0,213,85.07L52.92,32.8A15.95,15.95,0,0,0,32.8,52.92L85.07,213a15.82,15.82,0,0,0,14.41,11l.78,0a15.84,15.84,0,0,0,14.61-9.59l.13-.33L132.69,168,184,219.31a16,16,0,0,0,22.63,0l12.68-12.68a16,16,0,0,0,0-22.63ZM195.31,208,144,156.69a16,16,0,0,0-26,4.93c0,.11-.09.22-.13.32l-17.65,46L48,48l159.85,52.2-45.95,17.64-.32.13a16,16,0,0,0-4.93,26h0L208,195.31Z"/>',
  hand: '<path d="M188,88a27.75,27.75,0,0,0-12,2.71V60a28,28,0,0,0-41.36-24.6A28,28,0,0,0,80,44v6.71A27.75,27.75,0,0,0,68,48,28,28,0,0,0,40,76v76a88,88,0,0,0,176,0V116A28,28,0,0,0,188,88Zm12,64a72,72,0,0,1-144,0V76a12,12,0,0,1,24,0v44a8,8,0,0,0,16,0V44a12,12,0,0,1,24,0v68a8,8,0,0,0,16,0V60a12,12,0,0,1,24,0v68.67A48.08,48.08,0,0,0,120,176a8,8,0,0,0,16,0,32,32,0,0,1,32-32,8,8,0,0,0,8-8V116a12,12,0,0,1,24,0Z"/>',
  zoom: '<path d="M152,112a8,8,0,0,1-8,8H120v24a8,8,0,0,1-16,0V120H80a8,8,0,0,1,0-16h24V80a8,8,0,0,1,16,0v24h24A8,8,0,0,1,152,112Zm77.66,117.66a8,8,0,0,1-11.32,0l-50.06-50.07a88.11,88.11,0,1,1,11.31-11.31l50.07,50.06A8,8,0,0,1,229.66,229.66ZM112,184a72,72,0,1,0-72-72A72.08,72.08,0,0,0,112,184Z"/>',
  rotate: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M208 112a80 80 0 1 0-19 64"/><path d="M176 32h40v40"/><circle cx="128" cy="128" r="10" fill="currentColor" stroke="none"/></g>',
  anchor: '<g fill="none" stroke="currentColor" stroke-width="14" stroke-linecap="round"><circle cx="128" cy="128" r="52"/><path d="M128 32v48m0 96v48M32 128h48m96 0h48"/><circle cx="128" cy="128" r="8" fill="currentColor" stroke="none"/></g>',
  type: '<path d="M208,56V88a8,8,0,0,1-16,0V64H136V192h24a8,8,0,0,1,0,16H96a8,8,0,0,1,0-16h24V64H64V88a8,8,0,0,1-16,0V56a8,8,0,0,1,8-8H200A8,8,0,0,1,208,56Z"/>',
  shape: '<path d="M71.59,61.47a8,8,0,0,0-15.18,0l-40,120A8,8,0,0,0,24,192h80a8,8,0,0,0,7.59-10.53ZM35.1,176,64,89.3,92.9,176ZM208,76a52,52,0,1,0-52,52A52.06,52.06,0,0,0,208,76Zm-88,0a36,36,0,1,1,36,36A36,36,0,0,1,120,76Zm104,68H136a8,8,0,0,0-8,8v56a8,8,0,0,0,8,8h88a8,8,0,0,0,8-8V152A8,8,0,0,0,224,144Zm-8,56H144V160h72Z"/>',
  solid: '<path d="M208,32H48A16,16,0,0,0,32,48V208a16,16,0,0,0,16,16H208a16,16,0,0,0,16-16V48A16,16,0,0,0,208,32Zm0,176H48V48H208V208Z"/>',
  frame: '<path d="M216,48V88a8,8,0,0,1-16,0V56H168a8,8,0,0,1,0-16h40A8,8,0,0,1,216,48ZM88,200H56V168a8,8,0,0,0-16,0v40a8,8,0,0,0,8,8H88a8,8,0,0,0,0-16Zm120-40a8,8,0,0,0-8,8v32H168a8,8,0,0,0,0,16h40a8,8,0,0,0,8-8V168A8,8,0,0,0,208,160ZM88,40H48a8,8,0,0,0-8,8V88a8,8,0,0,0,16,0V56H88a8,8,0,0,0,0-16Z"/>',
  code: '<path d="M69.12,94.15,28.5,128l40.62,33.85a8,8,0,1,1-10.24,12.29l-48-40a8,8,0,0,1,0-12.29l48-40a8,8,0,0,1,10.24,12.3Zm176,27.7-48-40a8,8,0,1,0-10.24,12.3L227.5,128l-40.62,33.85a8,8,0,1,0,10.24,12.29l48-40a8,8,0,0,0,0-12.29ZM162.73,32.48a8,8,0,0,0-10.25,4.79l-64,176a8,8,0,0,0,4.79,10.26A8.14,8.14,0,0,0,96,224a8,8,0,0,0,7.52-5.27l64-176A8,8,0,0,0,162.73,32.48Z"/>',
  cam: '<path d="M251.77,73a8,8,0,0,0-8.21.39L208,97.05V72a16,16,0,0,0-16-16H32A16,16,0,0,0,16,72V184a16,16,0,0,0,16,16H192a16,16,0,0,0,16-16V159l35.56,23.71A8,8,0,0,0,248,184a8,8,0,0,0,8-8V80A8,8,0,0,0,251.77,73ZM192,184H32V72H192V184Zm48-22.95-32-21.33V116.28L240,95Z"/>',
  graph: '<path d="M200,152a31.84,31.84,0,0,0-19.53,6.68l-23.11-18A31.65,31.65,0,0,0,160,128c0-.74,0-1.48-.08-2.21l13.23-4.41A32,32,0,1,0,168,104c0,.74,0,1.48.08,2.21l-13.23,4.41A32,32,0,0,0,128,96a32.59,32.59,0,0,0-5.27.44L115.89,81A32,32,0,1,0,96,88a32.59,32.59,0,0,0,5.27-.44l6.84,15.4a31.92,31.92,0,0,0-8.57,39.64L73.83,165.44a32.06,32.06,0,1,0,10.63,12l25.71-22.84a31.91,31.91,0,0,0,37.36-1.24l23.11,18A31.65,31.65,0,0,0,168,184a32,32,0,1,0,32-32Zm0-64a16,16,0,1,1-16,16A16,16,0,0,1,200,88ZM80,56A16,16,0,1,1,96,72,16,16,0,0,1,80,56ZM56,208a16,16,0,1,1,16-16A16,16,0,0,1,56,208Zm56-80a16,16,0,1,1,16,16A16,16,0,0,1,112,128Zm88,72a16,16,0,1,1,16-16A16,16,0,0,1,200,200Z"/>',
  pen: '<g fill="none" stroke="currentColor" stroke-width="14" stroke-linecap="round" stroke-linejoin="round"><path d="m128 24-72 136 40 40h64l40-40Z"/><path d="M128 24v100m-40 100h80"/><circle cx="128" cy="140" r="16"/></g>',
  bezier: '<g fill="none" stroke="currentColor" stroke-width="14" stroke-linecap="round" stroke-linejoin="round"><path d="M40 200C40 120 136 56 216 56"/><path opacity=".58" d="M40 200V120M216 56H136"/><circle cx="40" cy="120" r="9" fill="var(--bg-panel, #111)"/><circle cx="136" cy="56" r="9" fill="var(--bg-panel, #111)"/><path fill="currentColor" stroke="none" d="m40 184 16 16-16 16-16-16Zm176-144 16 16-16 16-16-16Z"/></g>',
  gear: '<path d="M128,80a48,48,0,1,0,48,48A48.05,48.05,0,0,0,128,80Zm0,80a32,32,0,1,1,32-32A32,32,0,0,1,128,160Zm88-29.84q.06-2.16,0-4.32l14.92-18.64a8,8,0,0,0,1.48-7.06,107.21,107.21,0,0,0-10.88-26.25,8,8,0,0,0-6-3.93l-23.72-2.64q-1.48-1.56-3-3L186,40.54a8,8,0,0,0-3.94-6,107.71,107.71,0,0,0-26.25-10.87,8,8,0,0,0-7.06,1.49L130.16,40Q128,40,125.84,40L107.2,25.11a8,8,0,0,0-7.06-1.48A107.6,107.6,0,0,0,73.89,34.51a8,8,0,0,0-3.93,6L67.32,64.27q-1.56,1.49-3,3L40.54,70a8,8,0,0,0-6,3.94,107.71,107.71,0,0,0-10.87,26.25,8,8,0,0,0,1.49,7.06L40,125.84Q40,128,40,130.16L25.11,148.8a8,8,0,0,0-1.48,7.06,107.21,107.21,0,0,0,10.88,26.25,8,8,0,0,0,6,3.93l23.72,2.64q1.49,1.56,3,3L70,215.46a8,8,0,0,0,3.94,6,107.71,107.71,0,0,0,26.25,10.87,8,8,0,0,0,7.06-1.49L125.84,216q2.16.06,4.32,0l18.64,14.92a8,8,0,0,0,7.06,1.48,107.21,107.21,0,0,0,26.25-10.88,8,8,0,0,0,3.93-6l2.64-23.72q1.56-1.48,3-3L215.46,186a8,8,0,0,0,6-3.94,107.71,107.71,0,0,0,10.87-26.25,8,8,0,0,0-1.49-7.06Zm-16.1-6.5a73.93,73.93,0,0,1,0,8.68,8,8,0,0,0,1.74,5.48l14.19,17.73a91.57,91.57,0,0,1-6.23,15L187,173.11a8,8,0,0,0-5.1,2.64,74.11,74.11,0,0,1-6.14,6.14,8,8,0,0,0-2.64,5.1l-2.51,22.58a91.32,91.32,0,0,1-15,6.23l-17.74-14.19a8,8,0,0,0-5-1.75h-.48a73.93,73.93,0,0,1-8.68,0,8,8,0,0,0-5.48,1.74L100.45,215.8a91.57,91.57,0,0,1-15-6.23L82.89,187a8,8,0,0,0-2.64-5.1,74.11,74.11,0,0,1-6.14-6.14,8,8,0,0,0-5.1-2.64L46.43,170.6a91.32,91.32,0,0,1-6.23-15l14.19-17.74a8,8,0,0,0,1.74-5.48,73.93,73.93,0,0,1,0-8.68,8,8,0,0,0-1.74-5.48L40.2,100.45a91.57,91.57,0,0,1,6.23-15L69,82.89a8,8,0,0,0,5.1-2.64,74.11,74.11,0,0,1,6.14-6.14A8,8,0,0,0,82.89,69L85.4,46.43a91.32,91.32,0,0,1,15-6.23l17.74,14.19a8,8,0,0,0,5.48,1.74,73.93,73.93,0,0,1,8.68,0,8,8,0,0,0,5.48-1.74L155.55,40.2a91.57,91.57,0,0,1,15,6.23L173.11,69a8,8,0,0,0,2.64,5.1,74.11,74.11,0,0,1,6.14,6.14,8,8,0,0,0,5.1,2.64l22.58,2.51a91.32,91.32,0,0,1,6.23,15l-14.19,17.74A8,8,0,0,0,199.87,123.66Z"/>',
  sparkle: '<path d="M128,16a6,6,0,0,1,5.9,4.9C143.6,72.4,159,96,183.5,105.5c14.3,5.6,32.1,9.9,51.6,16.6a6,6,0,0,1,0,11.8c-19.5,6.7-37.3,11-51.6,16.6C159,160,143.6,183.6,133.9,235.1a6,6,0,0,1-11.8,0C112.4,183.6,97,160,72.5,150.5c-14.3-5.6-32.1-9.9-51.6-16.6a6,6,0,0,1,0-11.8c19.5-6.7,37.3-11,51.6-16.6C97,96,112.4,72.4,122.1,20.9A6,6,0,0,1,128,16Z"/>',
  sun: '<path d="M120,40V16a8,8,0,0,1,16,0V40a8,8,0,0,1-16,0Zm72,88a64,64,0,1,1-64-64A64.07,64.07,0,0,1,192,128Zm-16,0a48,48,0,1,0-48,48A48.05,48.05,0,0,0,176,128ZM58.34,69.66A8,8,0,0,0,69.66,58.34l-16-16A8,8,0,0,0,42.34,53.66Zm0,116.68-16,16a8,8,0,0,0,11.32,11.32l16-16a8,8,0,0,0-11.32-11.32ZM192,72a8,8,0,0,0,5.66-2.34l16-16a8,8,0,0,0-11.32-11.32l-16,16A8,8,0,0,0,192,72Zm5.66,114.34a8,8,0,0,0-11.32,11.32l16,16a8,8,0,0,0,11.32-11.32ZM48,128a8,8,0,0,0-8-8H16a8,8,0,0,0,0,16H40A8,8,0,0,0,48,128Zm80,80a8,8,0,0,0-8,8v24a8,8,0,0,0,16,0V216A8,8,0,0,0,128,208Zm112-88H216a8,8,0,0,0,0,16h24a8,8,0,0,0,0-16Z"/>',
  moon: '<path d="M233.54,142.23a8,8,0,0,0-8-2,88.08,88.08,0,0,1-109.8-109.8,8,8,0,0,0-10-10,104.84,104.84,0,0,0-52.91,37A104,104,0,0,0,136,224a103.09,103.09,0,0,0,62.52-20.88,104.84,104.84,0,0,0,37-52.91A8,8,0,0,0,233.54,142.23ZM188.9,190.34A88,88,0,0,1,65.66,67.11a89,89,0,0,1,31.4-26A106,106,0,0,0,96,56,104.11,104.11,0,0,0,200,160a106,106,0,0,0,14.92-1.06A89,89,0,0,1,188.9,190.34Z"/>',
  eyeoff: '<path d="M53.92,34.62A8,8,0,1,0,42.08,45.38L61.32,66.55C25,88.84,9.38,123.2,8.69,124.76a8,8,0,0,0,0,6.5c.35.79,8.82,19.57,27.65,38.4C61.43,194.74,93.12,208,128,208a127.11,127.11,0,0,0,52.07-10.83l22,24.21a8,8,0,1,0,11.84-10.76Zm47.33,75.84,41.67,45.85a32,32,0,0,1-41.67-45.85ZM128,192c-30.78,0-57.67-11.19-79.93-33.25A133.16,133.16,0,0,1,25,128c4.69-8.79,19.66-33.39,47.35-49.38l18,19.75a48,48,0,0,0,63.66,70l14.73,16.2A112,112,0,0,1,128,192Zm6-95.43a8,8,0,0,1,3-15.72,48.16,48.16,0,0,1,38.77,42.64,8,8,0,0,1-7.22,8.71,6.39,6.39,0,0,1-.75,0,8,8,0,0,1-8-7.26A32.09,32.09,0,0,0,134,96.57Zm113.28,34.69c-.42.94-10.55,23.37-33.36,43.8a8,8,0,1,1-10.67-11.92A132.77,132.77,0,0,0,231.05,128a133.15,133.15,0,0,0-23.12-30.77C185.67,75.19,158.78,64,128,64a118.37,118.37,0,0,0-19.36,1.57A8,8,0,1,1,106,49.79,134,134,0,0,1,128,48c34.88,0,66.57,13.26,91.66,38.35,18.83,18.83,27.3,37.62,27.65,38.41A8,8,0,0,1,247.31,131.26Z"/>',
  diamond: '<path d="M235.33,116.72,139.28,20.66a16,16,0,0,0-22.56,0l-96,96.06a16,16,0,0,0,0,22.56l96.05,96.06h0a16,16,0,0,0,22.56,0l96.05-96.06a16,16,0,0,0,0-22.56ZM128,224h0L32,128,128,32,224,128Z"/>',
  enter: '<path d="M184,104v32a8,8,0,0,1-8,8H99.31l10.35,10.34a8,8,0,0,1-11.32,11.32l-24-24a8,8,0,0,1,0-11.32l24-24a8,8,0,0,1,11.32,11.32L99.31,128H168V104a8,8,0,0,1,16,0Zm48-48V200a16,16,0,0,1-16,16H40a16,16,0,0,1-16-16V56A16,16,0,0,1,40,40H216A16,16,0,0,1,232,56ZM216,200V56H40V200H216Z"/>',
  return: '<path d="M216,48a8,8,0,0,0-8,8v56a24,24,0,0,1-24,24H67.31l34.35-34.34a8,8,0,0,0-11.32-11.32l-48,48a8,8,0,0,0,0,11.32l48,48a8,8,0,0,0,11.32-11.32L67.31,152H184a40,40,0,0,0,40-40V56A8,8,0,0,0,216,48Z"/>',
  /* Menu glyphs, stroked in the same weight as music/film. */
  copy: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><rect x="88" y="88" width="136" height="136" rx="12"/><path d="M168 88V44a12 12 0 0 0-12-12H44a12 12 0 0 0-12 12v112a12 12 0 0 0 12 12h44"/></g>',
  cube: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M128 24 40 72v112l88 48 88-48V72z"/><path d="M40 72l88 48 88-48M128 120v112"/></g>',
  stack: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M128 40 32 88l96 48 96-48z"/><path d="M32 136l96 48 96-48M32 184l96 48 96-48"/></g>',
  scissors: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><circle cx="64" cy="72" r="32"/><circle cx="64" cy="184" r="32"/><path d="M92 90l132 78M92 166l132-78"/></g>',
  speaker: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M80 96H40a8 8 0 0 0-8 8v48a8 8 0 0 0 8 8h40l64 48V48zM184 104a34 34 0 0 1 0 48M208 80a68 68 0 0 1 0 96"/></g>',
  headphones: '<g fill="none" stroke="currentColor" stroke-width="16" stroke-linecap="round" stroke-linejoin="round"><path d="M40 176v-40a88 88 0 0 1 176 0v40"/><rect x="40" y="144" width="48" height="72" rx="12"/><rect x="168" y="144" width="48" height="72" rx="12"/></g>',
};

/* ── math ──────────────────────────────────────────────── */
const clamp = (v: any, a: any, b: any) => (v < a ? a : v > b ? b : v);
const lerp = (a: any, b: any, t: any) => a + (b - a) * t;
const round = (v: any, n = 2) => { const p = 10 ** n; return Math.round(v * p) / p; };
PM.clamp = clamp; PM.lerp = lerp; PM.round = round;
PM.uid = (p = 'l') => p + Math.random().toString(36).slice(2, 9);

PM.hex2rgb = (hex: any) => {
  const s = hex.replace('#', '');
  const n = parseInt(s.length === 3 ? s.split('').map((c: any) => c + c).join('') : s, 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
};
PM.rgb2hex = (r: any, g: any, b: any) => '#' + [r, g, b].map(v => clamp(Math.round(v * 255), 0, 255).toString(16).padStart(2, '0')).join('');

/* ── time ──────────────────────────────────────────────── */
PM.tc = (sec: any, fps = 30, showFrames = true) => {
  const neg = sec < 0; sec = Math.abs(sec);
  let f = Math.round(sec * fps);
  const ff = f % fps; f = Math.floor(f / fps);
  const ss = f % 60; const mm = Math.floor(f / 60) % 60; const hh = Math.floor(f / 3600);
  const p = (n: any, w = 2) => String(n).padStart(w, '0');
  const base = (hh ? p(hh) + ':' : '') + p(mm) + ':' + p(ss);
  return (neg ? '-' : '') + base + (showFrames ? ':' + p(ff) : '');
};
PM.parseTc = (str: any, fps = 30) => {
  const parts: any = String(str).trim().split(':').map(Number);
  if (parts.some(isNaN)) return null;
  let s = 0;
  if (parts.length === 4) s = parts[0] * 3600 + parts[1] * 60 + parts[2] + parts[3] / fps;
  else if (parts.length === 3) s = parts[0] * 60 + parts[1] + parts[2] / fps;
  else if (parts.length === 2) s = parts[0] + parts[1] / fps;
  else s = parts[0];
  return s;
};
PM.snapF = (t: any, fps: any) => Math.round(t * fps) / fps;

/* ── event bus ─────────────────────────────────────────── */
const bus: any = { m: new Map() };
bus.on = (ev: any, fn: any) => { (bus.m.get(ev) || bus.m.set(ev, new Set()).get(ev)).add(fn); return () => bus.off(ev, fn); };
bus.off = (ev: any, fn: any) => { const s = bus.m.get(ev); if (s) s.delete(fn); };
bus.emit = (ev: any, a: any, b: any) => { const s = bus.m.get(ev); if (s) for (const fn of [...s]) { try { fn(a, b); } catch (e) { window.console.error('[bus]', ev, e); } } };
PM.bus = bus;

/* Rendering demand is visible to the frame loop immediately, while the more
   expensive UI/timeline notifications remain coalesced on one rAF. The frame
   loop itself still coalesces repeated draw requests with a boolean. */
const dirty = new Set();
let rafId = 0;
PM.invalidate = (what = 'all') => {
  if (what === 'all' || what === 'render') bus.emit('draw');
  if (what === 'all') { dirty.add('ui'); dirty.add('timeline'); }
  else if (what !== 'render') dirty.add(what);
  if (!rafId) rafId = window.requestAnimationFrame(flush);
};
function flush() {
  rafId = 0;
  const d = new Set(dirty); dirty.clear();
  if (d.has('timeline')) bus.emit('draw:timeline');
  if (d.has('ui')) bus.emit('draw:ui');
  if (d.has('status')) bus.emit('draw:status');
}

/* ── persistence ───────────────────────────────────────── */
PM.store = {
  get(k: any, d: any) { try { const v = window.localStorage.getItem('pm.' + k); return v == null ? d : JSON.parse(v); } catch { return d; } },
  set(k: any, v: any) { try { window.localStorage.setItem('pm.' + k, JSON.stringify(v)); return true; } catch (e) { window.console.warn('store', e); return false; } },
  del(k: any) { window.localStorage.removeItem('pm.' + k); },
};

/* ── drag helper ───────────────────────────────────────── */
/* Returns { cancel }. cancel() and a native pointercancel both end the drag
   without calling up(), so interrupted gestures can never wedge the cursor
   or leave ghost UI behind. */
PM.drag = (e: any, { move, up, cancel, cursor, infinite = false }: any) => {
  e.preventDefault();
  const sx = e.clientX, sy = e.clientY;
  const pointerId = e.pointerId;
  const captureEl = e.currentTarget && typeof e.currentTarget.setPointerCapture === 'function'
    ? e.currentTarget : null;
  const prevCur = window.document.body.style.cursor;
  if (cursor) window.document.body.style.cursor = cursor;
  let done = false;
  let dx = 0, dy = 0, requested = false, locked = false;
  const lockEl = infinite && e.pointerType !== 'touch' ? window.document.body : null;
  const doc = window.document;
  const lockChange = () => {
    if (doc.pointerLockElement === lockEl && lockEl) {
      if (done) { doc.exitPointerLock?.(); return; }
      locked = true;
    } else if (locked) pc();
  };
  const lockMove = (ev: any) => {
    if (done || !locked) return;
    dx += ev.movementX || 0; dy += ev.movementY || 0;
    move?.(dx, dy, ev);
  };
  const stop = () => {
    if (done) return false;
    done = true;
    window.removeEventListener('pointermove', mv, true);
    window.removeEventListener('pointerup', fin, true);
    window.removeEventListener('pointercancel', pc, true);
    captureEl?.removeEventListener?.('lostpointercapture', lostCapture);
    window.removeEventListener('mousemove', lockMove, true);
    window.removeEventListener('mouseup', fin, true);
    window.removeEventListener('blur', pc);
    doc.removeEventListener?.('pointerlockchange', lockChange);
    if (locked && doc.pointerLockElement === lockEl) doc.exitPointerLock?.();
    try {
      if (captureEl?.hasPointerCapture?.(pointerId)) captureEl.releasePointerCapture(pointerId);
    } catch { }
    window.document.body.style.cursor = prevCur;
    return true;
  };
  const mv = (ev: any) => {
    if (done || locked) return;
    dx = ev.clientX - sx; dy = ev.clientY - sy;
    move?.(dx, dy, ev);
    // Wait for a real drag so clicking still enters the numeric editor.
    if (!done && !requested && lockEl?.requestPointerLock && Math.abs(dx) >= 3) {
      requested = true;
      try {
        Promise.resolve(lockEl.requestPointerLock()).then(() => {
          if (done && doc.pointerLockElement === lockEl) doc.exitPointerLock?.();
        }).catch(() => {}); // Keep ordinary dragging if lock is unavailable.
      } catch { }
    }
  };
  const fin = (ev: any) => {
    const finalX = locked ? dx : ev.clientX - sx;
    const finalY = locked ? dy : ev.clientY - sy;
    if (stop() && up) up(finalX, finalY, ev);
  };
  const pc = () => { if (stop() && cancel) cancel(); };
  const lostCapture = () => { if (!requested) pc(); };
  try { captureEl?.setPointerCapture(pointerId); } catch { }
  captureEl?.addEventListener?.('lostpointercapture', lostCapture);
  /* WKWebView can stop bubbling pointer movement while a canvas owns the
     gesture. Capture-phase listeners plus explicit pointer capture keep direct
     manipulation alive until the matching up/cancel event. */
  if (infinite) {
    doc.addEventListener?.('pointerlockchange', lockChange);
    window.addEventListener('mousemove', lockMove, true);
    window.addEventListener('mouseup', fin, true);
    window.addEventListener('blur', pc);
  }
  window.addEventListener('pointermove', mv, true);
  window.addEventListener('pointerup', fin, true);
  window.addEventListener('pointercancel', pc, true);
  return { cancel: () => { if (stop() && cancel) cancel(); } };
};

PM.download = (blob: any, name: any) => {
  /* Native WKWebView has no browser download shelf — route through the AppKit save bridge. */
  const bridge = (window as any).webkit && (window as any).webkit.messageHandlers && (window as any).webkit.messageHandlers.saveFile;
  if (bridge) {
    const reader = new window.FileReader();
    reader.onload = () => {
      const res = String(reader.result || '');
      const comma = res.indexOf(',');
      const data = comma >= 0 ? res.slice(comma + 1) : res;
      bridge.postMessage({ name: String(name || 'powermove.bin'), data });
    };
    reader.onerror = () => { if (PM.toast) PM.toast('Save failed'); };
    reader.readAsDataURL(blob);
    return;
  }
  const a = h('a', { href: window.URL.createObjectURL(blob), download: name });
  window.document.body.appendChild(a); a.click();
  window.setTimeout(() => { window.URL.revokeObjectURL(a.href); a.remove(); }, 4000);
};
}
