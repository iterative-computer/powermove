/* Powermove — assistant rail. A collapsible sidebar that hosts the live chat
   panel outside the dock system, so the AI is always one keystroke away in any
   workspace. The panel element is mounted once and owned by the rail; workspace
   normalization strips docked chat panels so ownership never fights. */
(() => {
const PM = window.PM, h = PM.h;
const R = { el: null, body: null, open: false };
PM.ChatRail = {
  get isOpen() { return R.open; },
  toggle() { this.isOpen ? this.close() : this.open(); },
  open(focus) {
    ensure();
    R.open = true;
    R.el.classList.add('on');
    PM.store.set('chatRail', true);
    if (focus !== false) requestAnimationFrame(() => {
      const inp = PM.$('#cinput');
      if (inp) inp.focus();
    });
    PM.bus.emit('chatrail');
  },
  close() {
    R.open = false;
    if (R.el) R.el.classList.remove('on');
    PM.store.set('chatRail', false);
    if (document.activeElement && document.activeElement.blur) document.activeElement.blur();
    PM.bus.emit('chatrail');
  },
  /* boot: restore last state without stealing focus */
  init() {
    ensure();
    if (PM.store.get('chatRail', true)) this.open(false);
  },
};

function ensure() {
  if (R.el) return;
  const tab = h('button#chat-rail-tab', {
    title: 'Assistant · ⌘L',
    'aria-label': 'Toggle assistant sidebar',
    onclick: () => PM.ChatRail.toggle(),
  }, PM.icon('sparkle'));
  R.body = h('div.chat-rail-body');
  /* the hosted chat panel carries its own header; only a close affordance overlays */
  const closeBtn = h('button.chat-rail-close', {
    title: 'Collapse assistant', onclick: () => PM.ChatRail.close(),
  }, PM.icon('x'));
  const inner = h('div.chat-rail-inner', closeBtn, R.body);
  R.el = h('div#chat-rail', tab, inner);
  document.body.appendChild(R.el);
  mount();
}

function mount() {
  /* The chat panel builds itself on first mount; later layouts reuse the node. */
  const el = PM.Layout.mountFloatingPanel('chat');
  if (el && el.parentNode !== R.body) R.body.appendChild(el);
}
})();
