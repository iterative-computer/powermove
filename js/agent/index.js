/* Powermove — agent core. Semantic project knowledge + an action loop.
   Modeled on Supermove's harness: brief → plan → act → look → critique → review. */
(() => {
const PM = window.PM;

const A = {
  thread: [],           // {role, content, tools:[], shots:[]}
  running: false,
  abort: null,
  provider: PM.store.get('provider', 'local'),
  model: PM.store.get('model', ''),
  mode: PM.store.get('agentMode', 'design'),
  maxSteps: 14,
};
PM.Agent = A;

/* ── semantic project digest ───────────────────────────── */
/** A compact, information-dense description of the project the model reasons over. */
A.digest = (opt = {}) => {
  const p = PM.proj;
  const L = (l, i) => {
    const props = PM.allProps(l).filter(x => x.prop.kf.length || x.prop.expr);
    const anim = props.map(x => {
      const kf = x.prop.kf;
      const span = kf.length ? `${PM.round(kf[0].t, 2)}→${PM.round(kf[kf.length - 1].t, 2)}s` : '';
      const vals = kf.length ? `${PM.round(kf[0].v, 1)}→${PM.round(kf[kf.length - 1].v, 1)}` : '';
      const ease = kf.length > 1 ? PM.Ease.nameOf(kf[0].eo, kf[1].ei) : '';
      return `${x.key}[${kf.length}k ${span} ${vals}${ease ? ' ' + ease : ''}${x.prop.expr ? ' expr:' + x.prop.expr.slice(0, 40) : ''}]`;
    });
    const fx = l.fx.filter(f => f.on).map(f => PM.FX[f.type] ? PM.FX[f.type].label : f.type);
    const d = l.d;
    let content = '';
    if (l.type === 'text') content = `"${String(d.text).slice(0, 60)}" ${d.font} ${d.weight} ${d.size}px ${d.color}`;
    else if (l.type === 'solid' || l.type === 'shape') content = `${d.shape || 'solid'} ${d.w}×${d.h} ${d.color}${d.radius ? ' r' + d.radius : ''}`;
    else if (l.type === 'shader') content = `${Object.keys(d.uniforms || {}).length} uniforms · ${d.code.split('\n').length} lines`;
    else if (l.type === 'image' || l.type === 'video') content = (p.assets[d.asset] || {}).name || 'no source';
    else if (l.type === 'audio') content = ((p.assets[d.asset] || {}).name || 'no source') + ` gain ${d.gain}`;
    const pos = `pos(${PM.round(PM.ev(l, 'position.x', PM.time))},${PM.round(PM.ev(l, 'position.y', PM.time))}) scale ${PM.round(PM.ev(l, 'scale.x', PM.time))}% op ${PM.round(PM.ev(l, 'opacity', PM.time))}%`;
    return `#${i + 1} "${l.name}" ${l.type} ${PM.round(l.from, 2)}→${PM.round(l.from + l.dur, 2)}s` +
      `${l.on ? '' : ' HIDDEN'}${l.lock ? ' LOCKED' : ''}${l.parent ? ' parent:' + (PM.L(l.parent) || {}).name : ''}` +
      `${l.blend !== 'normal' ? ' blend:' + l.blend : ''}${l.mblur ? ' +mblur' : ''}\n` +
      `    content: ${content}\n    transform: ${pos}` +
      (anim.length ? `\n    animated: ${anim.join(' ')}` : '') +
      (fx.length ? `\n    effects: ${fx.join(', ')}` : '') +
      (Object.keys(l.locked_intent || {}).length ? `\n    hand-edited (preserve): ${Object.keys(l.locked_intent).join(', ')}` : '');
  };
  const ws = PM.WS.current;
  const lines = [
    `COMPOSITION "${p.name}" ${p.w}×${p.h} @${p.fps}fps duration ${p.dur}s bg ${p.bg} revision ${p.revision || 0}`,
    `playhead ${PM.round(PM.time, 3)}s (frame ${Math.round(PM.time * p.fps)})  work area ${PM.round(p.work[0], 2)}→${PM.round(p.work[1], 2)}s`,
    `LAYERS (${p.layers.length}, top first):`,
    ...(p.layers.length ? p.layers.map(L) : ['    (empty composition)']),
  ];
  const sel = PM.selLayers();
  lines.push(`SELECTION: ${sel.length ? sel.map(l => `"${l.name}"`).join(', ') : 'none (full composition scope)'}` +
    (PM.sel.chan ? ` · channel ${PM.sel.chan}` : ''));
  if (Object.keys(p.params).length)
    lines.push(`SCENE PARAMS: ${Object.values(p.params).map(x => `${x.name}=${x.value}`).join(' ')}`);
  if (Object.keys(p.assets).length)
    lines.push(`MEDIA: ${Object.values(p.assets).map(a => `${a.name}(${a.kind}${a.dur ? ' ' + PM.round(a.dur, 1) + 's' : ''})`).join(', ')}`);
  if (p.markers.length) lines.push(`MARKERS: ${p.markers.map(m => `${m.name}@${PM.round(m.t, 2)}s`).join(' ')}`);
  if (p.notes) lines.push(`DIRECTION NOTES: ${p.notes}`);
  lines.push(`WORKSPACE "${ws.name}" density:${ws.density} accent:${(ws.theme || {}).accent || 'default'}`);
  lines.push(`  docks: ${ws.layout.docks.map(d => `${d.id}[${d.panels.map(x => x.id).join(',')}]`).join(' ')}`);
  lines.push(`  features: ${Object.entries(ws.features || {}).map(([k, v]) => k + '=' + v).join(' ')}`);
  lines.push(`  available panels: ${Object.keys(PM.PANELS).join(', ')}`);
  if (opt.shaders !== false) {
    const sh = p.layers.filter(l => l.type === 'shader');
    sh.forEach(l => lines.push(`SHADER "${l.name}" source:\n${l.d.code}`));
  }
  return lines.join('\n');
};

/** Structured snapshot used by tools and the local planner. */
A.state = () => ({
  comp: { w: PM.proj.w, h: PM.proj.h, fps: PM.proj.fps, dur: PM.proj.dur, bg: PM.proj.bg, time: PM.time, revision: PM.proj.revision || 0 },
  layers: PM.proj.layers.map((l, i) => ({
    index: i + 1, id: l.id, name: l.name, type: l.type, from: l.from, dur: l.dur,
    visible: l.on, locked: l.lock, blend: l.blend, effects: l.fx.map(f => f.type),
    animated: PM.allProps(l).filter(x => x.prop.kf.length).map(x => x.key),
  })),
  selection: PM.sel.layers.map(id => (PM.L(id) || {}).name).filter(Boolean),
  workspace: PM.WS.current.name,
});

/* ── system prompt ─────────────────────────────────────── */
A.system = () => `You are the motion designer inside Powermove, a GPU-native motion-design tool. You do not describe work — you perform it by calling tools.

# How you work
- Read the COMPOSITION STATE below before acting. It is the live truth: layers, timing, keyframes, easing, effects, shader source, selection, and the current workspace layout.
- Time is SECONDS everywhere. Keyframe times are LAYER-LOCAL (relative to a layer's start). fps is sampling density only.
- Take action with tools. Batch related edits, then call \`look\` to render real frames and judge them like a director. You are blind until you look.
- Prefer one \`edit_source\` transaction for related source changes. It is the same validated command language used by canvas handles, inspectors, and generated controls.
- After a new composition or a substantial motion change: look at representative moments from every beat plus the frames around each transition, critique, fix, and look again. Do this at least twice before saying you are done.
- Keep replies to one or two sentences about what you changed and why it feels right. Speak as a designer: "the reveal", "the settle", "the ember pass" — never file names or code terms, unless the user is working on a shader.

# Scope discipline
- A selected layer is the default boundary of a request. Preserve unrelated layers, timing, and channels.
- Layers marked "hand-edited (preserve)" carry locked creative intent from direct manipulation. Never overwrite those channels unless the user explicitly asks.
- If the user asks for another direction, make it meaningfully different inside the stated scope. Takes are saved automatically; never duplicate layers as backups.

# Motion craft
- Design the temporal system before keyframing: what develops, what each beat inherits, why the next state belongs in the same film. Repeated enter–hold–exit timing and interchangeable layouts are failures, not styles.
- Default starting points, never a house look: stagger related parts 40–140ms, arrivals around 0.92→1 scale, decisive ease-out in and ease-in out, and reserve overshoot for the one element that earns it. The signature curve is \`power\`.
- Every visible element must earn its place. Do not add fake UI, particles, or microcopy for texture.
- Preserve the user's copy casing and use comfortable letter spacing unless they ask otherwise.

# Interface authorship
You can rebuild the app's own UI. \`set_workspace\` moves, adds, hides, and resizes panels; sets theme accent, density, and radius; toggles features; and defines custom panels. Controls may write scene parameters through param("Name"), or bind directly to real layer source with target plus a properties.*, content.*, or layer.* path. Buttons may submit typed source-edit commands. When the user asks for a workspace or a UI change, actually change it — do not just describe it.

# Shaders
Shader layers are GLSL ES 3.0 fragment shaders. Write to \`fragColor\`; \`uv\` is 0..1 across the layer; \`iTime\` is layer-local seconds, \`iProgress\` is 0..1 across the layer, \`iResolution\` is layer pixels. Helpers available: hash, noise, fbm, rot, palette, luma, sdCircle, sdBox, rgb2hsv, hsv2rgb. Declare controls as \`uniform float uName; // @param default min max\` or \`uniform vec3 uColor; // @param #RRGGBB\` — every annotated uniform becomes an inspector control and is keyframable. Never redeclare the built-ins. Always \`look\` after writing a shader; a compile error means the canvas still shows the old frame.

Mode: ${A.mode}. Platform: browser, WebGL2.`;

/* ── thread management ─────────────────────────────────── */
A.push = (m) => { A.thread.push(m); PM.bus.emit('chat'); return m; };
A.clear = () => { A.thread = []; PM.bus.emit('chat'); };

A.send = async (text, opts = {}) => {
  if (A.running) return PM.toast('Assistant is working…');
  if (!text.trim()) return;
  A.push({ role: 'user', content: text });
  A.running = true;
  PM.bus.emit('chat:state');
  PM.takes.save('Before: ' + text.slice(0, 40));
  try {
    const provider = PM.Providers.get(A.provider);
    await provider.run(text, opts);
  } catch (e) {
    console.error(e);
    A.push({ role: 'error', content: String(e.message || e) });
  } finally {
    A.running = false;
    PM.bus.emit('chat:state');
    PM.invalidate();
  }
};

A.stop = () => { if (A.abort) A.abort.abort(); A.running = false; PM.bus.emit('chat:state'); };
})();
