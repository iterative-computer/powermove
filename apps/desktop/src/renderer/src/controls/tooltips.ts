/*
 * Every tooltip in the app. The browser's own (from `title`) waits about a
 * second for each control, even when you are running the pointer along a row
 * of them, and drops below the pointer wherever there is room. This replaces
 * them all without touching the controls:
 *
 * - Hovering a control with a `title` shows its tip after a short wait; once a
 *   tip is showing, moving to a neighbour shows the next one at once.
 * - "Name — hint" reads as a name and a quieter hint; a trailing shortcut,
 *   "(⌘Z)" or "(F)", or `data-tip-key`, shows as a key.
 * - The tip opens away from the nearer window edge (above controls low in the
 *   window), and stays on screen.
 * - Clicking, scrolling, a key or leaving the window hides it. Keyboard focus
 *   shows it too.
 *
 * While a control is hovered its `title` is held aside in `data-tip-title`
 * (so the browser shows nothing) and the tip is its accessible description;
 * it is put back on leave or click. Iframes keep their titles (their names).
 */

const SHOW_MS = 380;
/** After a tip hides, the next one within this window shows at once. */
const WARM_MS = 600;
const GAP = 7;
const MARGIN = 6;

const KEY = /\s*\(((?:[⌘⌥⇧⌃]+\s*)?(?:[A-Z0-9]|F\d{1,2}|Esc|Space|Tab|Enter|Home|End|Delete|[←→↑↓]|[⌘⌥⇧⌃][^()]{0,12}))\)\s*$/;

export type TipText = { name: string; hint: string; key: string };

/** Split a title into name, hint and shortcut. */
export function parseTip(text: string, key = ''): TipText {
  let rest = text.trim();
  const k = !key ? rest.match(KEY) : null;
  if (k) { key = k[1]!.trim(); rest = rest.slice(0, k.index).trim(); }
  const dash = rest.search(/\s[—–]\s/);
  if (dash > 0) return { name: rest.slice(0, dash).trim(), hint: rest.slice(dash + 3).trim(), key };
  return { name: rest, hint: '', key };
}

const CSS = `
.pm-tip{position:fixed;z-index:2147483000;max-width:260px;padding:5px 8px 6px;border-radius:7px;background:var(--bg-float,#222);color:var(--tx,#fff);box-shadow:var(--shadow-float,0 4px 16px rgba(0,0,0,.25));font:var(--fs-sm,12px)/1.35 var(--f-ui,system-ui);pointer-events:none;opacity:0;transform:translateY(var(--tip-dy,2px));transition:opacity 90ms ease,transform 90ms ease;white-space:normal}
.pm-tip.on{opacity:1;transform:none}
.pm-tip.instant{transition:none}
.pm-tip-head{display:flex;align-items:center;gap:8px;justify-content:space-between}
.pm-tip-name{font-weight:var(--fw-medium,500)}
.pm-tip-key{flex:none;padding:0 5px;border-radius:4px;background:var(--ink-2,rgba(127,127,127,.18));color:var(--tx-2,inherit);font:11px/16px var(--f-ui,system-ui);font-variant-numeric:tabular-nums}
.pm-tip-hint{margin-top:1px;color:var(--tx-3,rgba(255,255,255,.65))}
`;

export function installTooltips(doc: Document = document): () => void {
  const win = doc.defaultView!;
  const style = doc.createElement('style');
  style.textContent = CSS;
  doc.head.append(style);
  const tip = doc.createElement('div');
  tip.className = 'pm-tip';
  tip.id = 'pm-tooltip';
  tip.setAttribute('role', 'tooltip');
  const head = doc.createElement('div'); head.className = 'pm-tip-head';
  const name = doc.createElement('span'); name.className = 'pm-tip-name';
  const key = doc.createElement('span'); key.className = 'pm-tip-key';
  const hint = doc.createElement('div'); hint.className = 'pm-tip-hint';
  head.append(name, key);
  tip.append(head, hint);
  const attach = () => { if (!tip.isConnected) doc.body.append(tip); };

  let target: HTMLElement | null = null;
  let held: string | null = null;
  let timer = 0;
  let hiddenAt = -Infinity;
  let visible = false;

  /** Put the hovered control's title back and drop its description. */
  const release = () => {
    if (!target) return;
    if (held !== null && !target.hasAttribute('title')) target.setAttribute('title', held);
    target.removeAttribute('data-tip-title');
    if (target.getAttribute('aria-describedby') === tip.id) target.removeAttribute('aria-describedby');
    target = null; held = null;
  };
  const hide = () => {
    win.clearTimeout(timer);
    if (visible) hiddenAt = performance.now();
    visible = false;
    tip.classList.remove('on');
    release();
  };

  const place = (el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    const vw = win.innerWidth, vh = win.innerHeight;
    tip.style.left = '0px'; tip.style.top = '0px';
    const w = tip.offsetWidth, h = tip.offsetHeight;
    // Away from the nearer edge: above controls in the lower half.
    const below = r.top + r.height / 2 < vh / 2;
    let top = below ? r.bottom + GAP : r.top - GAP - h;
    if (top < MARGIN) top = r.bottom + GAP;
    if (top + h > vh - MARGIN) top = Math.max(MARGIN, r.top - GAP - h);
    const left = Math.min(Math.max(MARGIN, r.left + r.width / 2 - w / 2), vw - w - MARGIN);
    tip.style.left = `${Math.round(left)}px`; tip.style.top = `${Math.round(top)}px`;
    tip.style.setProperty('--tip-dy', below ? '-2px' : '2px');
  };

  const show = (el: HTMLElement, text: string, instant: boolean) => {
    attach();
    const t = parseTip(text, el.dataset.tipKey ?? '');
    name.textContent = t.name;
    key.textContent = t.key; key.hidden = !t.key;
    hint.textContent = t.hint; hint.hidden = !t.hint;
    tip.classList.toggle('instant', instant);
    place(el);
    visible = true;
    tip.classList.add('on');
  };

  /** The control under an event that carries a tip. */
  const tipped = (node: EventTarget | null): HTMLElement | null => {
    const el = node instanceof Element ? node.closest<HTMLElement>('[title],[data-tip]') : null;
    if (!el || !(el instanceof HTMLElement) || el instanceof HTMLIFrameElement) return null;
    if (el === doc.body || el === doc.documentElement) return null;
    return el;
  };

  const enter = (el: HTMLElement, viaFocus: boolean) => {
    if (el === target) return;
    const wasVisible = visible;
    hide();
    const text = el.getAttribute('title') ?? el.dataset.tip ?? '';
    if (!text.trim()) return;
    target = el;
    if (el.hasAttribute('title')) { held = text; el.setAttribute('data-tip-title', text); el.removeAttribute('title'); }
    el.setAttribute('aria-describedby', tip.id);
    const warm = wasVisible || performance.now() - hiddenAt < WARM_MS;
    if (warm || viaFocus) { show(el, text, warm); return; }
    timer = win.setTimeout(() => { if (target === el && el.isConnected) show(el, text, false); }, SHOW_MS);
  };

  // What the pointer is over: a title may arrive after it (a canvas names the
  // region under the pointer as it moves).
  let hovered: Element | null = null;
  const onOver = (event: PointerEvent) => {
    hovered = event.target instanceof Element ? event.target : null;
    // No tips while dragging or for touch.
    if (event.buttons || event.pointerType === 'touch') return;
    const el = tipped(event.target);
    if (el) enter(el, false);
    else if (target) hide();
  };
  const onOut = (event: PointerEvent) => {
    if (!target) return;
    const to = event.relatedTarget as Node | null;
    if (to && target.contains(to)) return;
    if (tipped(to) === target) return;
    hide();
  };
  const onFocus = (event: FocusEvent) => {
    const el = event.target instanceof HTMLElement ? event.target : null;
    if (!el?.matches(':focus-visible')) return;
    const t = tipped(el);
    if (t === el) enter(el, true);
  };
  const onBlur = (event: FocusEvent) => { if (event.target === target) hide(); };
  const onLeaveWindow = (event: PointerEvent) => { if (!event.relatedTarget) hovered = null; };
  // A control removed while hovered hides its tip; one retitled (a re-render
  // puts `title` back) updates it and keeps the browser's own tip away.
  const watch = new MutationObserver(records => {
    if (target && !target.isConnected) { hide(); return; }
    if (target && records.some(r => r.type === 'attributes' && r.target === target) && target.hasAttribute('title')) {
      held = target.getAttribute('title');
      target.setAttribute('data-tip-title', held ?? '');
      target.removeAttribute('title');
      if (visible && held?.trim()) show(target, held, true);
      return;
    }
    // A title arriving on (or above) what the pointer is already over.
    if (!target && hovered?.isConnected) {
      const el = tipped(hovered);
      if (el && records.some(r => r.type === 'attributes' && r.target === el)) enter(el, false);
    }
  });
  watch.observe(doc.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ['title'] });

  doc.addEventListener('pointerover', onOver, true);
  doc.addEventListener('pointerout', onOut, true);
  doc.addEventListener('pointerout', onLeaveWindow, true);
  doc.addEventListener('pointerdown', hide, true);
  doc.addEventListener('keydown', hide, true);
  doc.addEventListener('wheel', hide, { capture: true, passive: true });
  doc.addEventListener('scroll', hide, true);
  doc.addEventListener('focusin', onFocus, true);
  doc.addEventListener('focusout', onBlur, true);
  win.addEventListener('blur', hide);
  return () => {
    hide();
    watch.disconnect();
    doc.removeEventListener('pointerover', onOver, true);
    doc.removeEventListener('pointerout', onOut, true);
    doc.removeEventListener('pointerout', onLeaveWindow, true);
    doc.removeEventListener('pointerdown', hide, true);
    doc.removeEventListener('keydown', hide, true);
    doc.removeEventListener('wheel', hide, true);
    doc.removeEventListener('scroll', hide, true);
    doc.removeEventListener('focusin', onFocus, true);
    doc.removeEventListener('focusout', onBlur, true);
    win.removeEventListener('blur', hide);
    tip.remove(); style.remove();
  };
}
