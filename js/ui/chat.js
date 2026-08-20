/* Powermove — assistant rail. Tool activity and rendered evidence stay visible. */
(() => {
const PM = window.PM, h = PM.h, A = PM.Agent;
const U = { root: null, thread: null, input: null, send: null };
PM.Chat = U;

PM.registerPanel('chat', {
  title: 'Assistant', noscroll: true,
  build(body) {
    const head = h('div#chat-head');
    const mode = h('button.ghost', { onpointerdown: e => modeMenu(e, mode) }, PM.icon('sparkle'), h('span', modeLabel()));
    const provider = h('button.ghost', { onpointerdown: e => providerMenu(e, provider) }, h('span', PM.Providers.label(A.provider)), PM.icon('chevD'));
    const clear = h('button.iconbtn', { title: 'Clear conversation', onclick: () => A.clear() }, PM.icon('x'));
    head.append(mode, h('span', { style: { flex: 1 } }), provider, clear);

    const thread = h('div#thread');
    const composer = h('div#composer');
    const input = h('textarea#cinput', { rows: '1', placeholder: 'Describe the move, shader, or workspace…' });
    const send = h('button#csend', { title: 'Send' }, PM.icon('up'));
    const cbox = h('div#cbox', input, send);
    const foot = h('div#cfoot');
    composer.append(cbox, foot);
    body.append(head, thread, composer);
    U.root = body; U.thread = thread; U.input = input; U.send = send; U.mode = mode; U.provider = provider; U.foot = foot;

    input.addEventListener('input', () => { autoSize(); syncState(); });
    input.addEventListener('keydown', e => {
      e.stopPropagation();
      if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); submit(); }
      if (e.key === 'Escape' && A.running) A.stop();
    });
    send.onclick = () => A.running ? A.stop() : submit();
    render(); syncState();
  },
});

function modeLabel() { return A.mode === 'animate' ? 'Motion' : A.mode === 'shader' ? 'Shader' : A.mode === 'review' ? 'Review' : 'Design'; }
function modeMenu(e, b) {
  e.preventDefault();
  PM.menu(b, [
    { header: 'Assistant mode' },
    ...[['design','Design direction'],['animate','Motion craft'],['shader','Shader authoring'],['review','Review & critique']].map(([v, label]) => ({
      label, on: A.mode === v, run: () => { A.mode = v; PM.store.set('agentMode', v); syncHead(); },
    })),
  ]);
}
function providerMenu(e, b) {
  e.preventDefault();
  PM.menu(b, [
    { header: 'Model provider' },
    ...Object.values(PM.Providers.all).map(p => ({ label: p.label, on: A.provider === p.id, run: () => { A.provider = p.id; A.model = ''; PM.store.set('provider', p.id); syncHead(); } })),
    '-',
    { label: 'Assistant settings…', run: settings },
  ]);
}
function syncHead() {
  if (!U.root || !U.root.isConnected) return;
  U.mode.textContent = ''; U.mode.append(PM.icon('sparkle'), h('span', modeLabel()));
  U.provider.textContent = ''; U.provider.append(h('span', PM.Providers.label(A.provider)), PM.icon('chevD'));
  syncState();
}

function submit() {
  const text = U.input.value.trim();
  if (!text || A.running) return;
  U.input.value = ''; autoSize(); syncState();
  A.send(text);
}
function autoSize() {
  if (!U.input) return;
  U.input.style.height = '22px';
  U.input.style.height = Math.min(150, U.input.scrollHeight) + 'px';
}
function syncState() {
  if (!U.send) return;
  const hot = !!(U.input && U.input.value.trim());
  U.send.classList.toggle('hot', hot || A.running);
  U.send.textContent = '';
  U.send.appendChild(A.running ? PM.icon('x') : PM.icon('up'));
  U.send.title = A.running ? 'Stop' : 'Send';
  U.foot.textContent = '';
  const sel = PM.selLayers();
  U.foot.append(
    h('span.ctx', sel.length ? [h('b', String(sel.length)), sel.length === 1 ? sel[0].name : 'selected layers'] : [h('b', 'All'), 'composition']),
    h('span.sp'),
    h('span', A.running ? 'working…' : A.provider === 'local' ? 'offline · ⏎ send' : 'tool mode · ⏎ send'),
  );
}

function render() {
  if (!U.thread || !U.thread.isConnected) return;
  const nearEnd = U.thread.scrollHeight - U.thread.scrollTop - U.thread.clientHeight < 100;
  U.thread.textContent = '';
  if (!A.thread.length) U.thread.appendChild(welcome());
  A.thread.forEach(m => U.thread.appendChild(message(m)));
  if (A.running) U.thread.appendChild(h('div.msg.sys', h('span.think', 'Reading the live composition and making the move…')));
  if (nearEnd || A.running) requestAnimationFrame(() => U.thread.scrollTop = U.thread.scrollHeight);
}

function welcome() {
  const examples = [
    'Animate the title in with a soft power curve',
    'Make a living orange gradient shader',
    'Build a compact workspace for animation',
    'Create an 8-second motion intro',
  ];
  return h('div.hello',
    h('div', { style: { color: 'var(--tx)', fontWeight: 570, marginBottom: '5px' } }, 'Direct the work, not the controls.'),
    h('div', 'I can read every layer, keyframe, effect, shader, and panel—then edit them directly.'),
    h('div.qbtns', ...examples.map(x => h('button.qbtn', { onclick: () => { U.input.value = x; autoSize(); syncState(); U.input.focus(); } }, x))),
  );
}
function message(m) {
  if (m.role === 'user') return h('div.msg.user', m.content || '');
  if (m.role === 'error') return h('div.msg.err', m.content || 'Unknown error');
  if (m.role === 'tool') return toolCard(m);
  if (m.role === 'question') return question(m);
  const box = h('div.msg.sys');
  if (m.content) box.appendChild(rich(m.content));
  if (m.pre) box.appendChild(h('pre', m.pre));
  if (m.shots && m.shots.length) box.appendChild(shots(m.shots));
  return box;
}
function toolCard(m) {
  const card = h('div.tool.' + (m.state || 'run'));
  const head = h('div.th', h('i.dt'), h('span.nm', toolName(m.name)), h('span.sp'), h('span', m.state === 'run' ? 'running' : m.state === 'bad' ? 'failed' : 'done'));
  card.appendChild(head);
  const summary = m.content || compactArgs(m.args);
  if (summary) card.appendChild(h('div.tb', summary));
  if (m.data && m.data.shots) card.appendChild(shots(m.data.shots));
  return card;
}
function question(m) {
  return h('div.msg.sys', h('div', m.content), h('div.qbtns', ...(m.options || []).map(x => h('button.qbtn', { onclick: () => A.send(x) }, x))));
}
function shots(list) {
  return h('div.shots', ...list.map(s => h('img', { src: s.image, title: `${s.time}s`, alt: `Composition at ${s.time}s` })));
}
function rich(text) {
  const box = h('div');
  String(text).split(/\n\n+/).forEach(p => {
    const el = h('p');
    const chunks = p.split(/(`[^`]+`)/g);
    chunks.forEach(c => el.appendChild(c.startsWith('`') && c.endsWith('`') ? h('code', c.slice(1,-1)) : document.createTextNode(c)));
    box.appendChild(el);
  });
  return box;
}
function toolName(n) { return String(n || 'tool').replace(/_/g, ' '); }
function compactArgs(a) {
  if (!a || !Object.keys(a).length) return '';
  const s = JSON.stringify(a);
  return s.length > 220 ? s.slice(0, 217) + '…' : s;
}

function settings() {
  const config = PM.Providers.config();
  const provider = { value: A.provider };
  const model = { value: A.model || (config[A.provider] || {}).model || '' };
  const key = { value: (config[A.provider] || {}).key || '' };
  const url = { value: (config[A.provider] || {}).url || '' };
  const body = h('div', { style: { display: 'flex', flexDirection: 'column', gap: '5px' } });
  const providerCtl = PM.selectField(() => provider.value, v => { provider.value = v; }, Object.values(PM.Providers.all).map(p => ({ v: p.id, label: p.label })));
  body.appendChild(PM.row('Provider', providerCtl));
  const field = (label, obj, placeholder, secret) => {
    const inp = h('input', { value: obj.value, placeholder, type: secret ? 'password' : 'text', style: { width: '210px', textAlign: 'right', fontFamily: 'var(--f-mono)', fontSize: '11px' } });
    inp.addEventListener('input', () => obj.value = inp.value);
    inp.addEventListener('keydown', e => e.stopPropagation());
    body.appendChild(PM.row(label, inp));
  };
  field('Model', model, 'provider default');
  field('API key', key, 'stored in this browser', true);
  field('Endpoint', url, 'optional / compatible URL');
  body.appendChild(h('div', { style: { color: 'var(--tx-3)', fontSize: '11px', lineHeight: 1.55, padding: '8px 4px 0' } },
    'Local mode needs no key. Browser-held API keys are appropriate for personal local use only.'));
  PM.modal({
    title: 'Assistant Settings', body, width: 470,
    actions: [{ label: 'Cancel' }, { label: 'Save', pri: true, run: () => {
      config[provider.value] = { ...(config[provider.value] || {}), key: key.value.trim(), url: url.value.trim(), model: model.value.trim() };
      PM.Providers.saveConfig(config);
      A.provider = provider.value; A.model = model.value.trim();
      PM.store.set('provider', A.provider); PM.store.set('model', A.model); syncHead();
    } }],
  });
}

PM.bus.on('chat', render);
PM.bus.on('chat:state', () => { render(); syncState(); });
PM.bus.on('sel', syncState);
})();
