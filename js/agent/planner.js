/* Powermove — capable offline planner. No API key required.
   It maps semantic intent to the same action tools used by remote models. */
(() => {
const PM = window.PM, A = PM.Agent;
const LP = {};
PM.LocalPlanner = LP;

const say = (content, extra = {}) => A.push({ role: 'assistant', content, ...extra });
const call = (n, a) => A.callTool(n, a);
const lower = s => s.toLowerCase();
const has = (s, ...q) => q.some(x => s.includes(x));
const colors = {
  orange: '#FF6B1A', blue: '#4C8DFF', red: '#FF5C5C', green: '#3FCF8E', yellow: '#FFD66B',
  purple: '#8B75FF', violet: '#6C7BE8', white: '#F2F2F2', black: '#050505', cream: '#E8E2CF', pink: '#FF70B7', cyan: '#55D6E8',
};
function colorIn(s, fallback) {
  const hex = s.match(/#[0-9a-f]{6}\b/i); if (hex) return hex[0];
  for (const k in colors) if (s.includes(k)) return colors[k];
  return fallback;
}
function quoted(s) { const m = s.match(/[“"]([^”"]+)[”"]/); return m ? m[1] : null; }
function numBefore(s, unit) { const m = s.match(new RegExp('(\\d+(?:\\.\\d+)?)\\s*' + unit)); return m ? +m[1] : null; }
function target(s) {
  const sel = PM.firstSel(); if (sel && has(s, 'this', 'it', 'selected', 'selection', 'layer')) return sel;
  const names = PM.proj.layers.filter(l => s.includes(l.name.toLowerCase())).sort((a,b) => b.name.length-a.name.length);
  return names[0] || sel;
}

LP.run = async (text) => {
  const s = lower(text.trim());
  const acts = [];
  const doCall = async (n, a) => { acts.push([n, a]); return call(n, a); };

  /* project inspection / explain */
  if (/^(what|show|summari[sz]e|inspect).*(project|composition|timeline|selected)/.test(s)) {
    const d = A.digest({ shaders: false });
    say('Here’s the live project structure and current editing scope.', { pre: d });
    return;
  }

  /* UI / workspace authorship */
  if (has(s, 'workspace', 'interface', 'ui ', 'panel', 'layout') && has(s, 'make', 'create', 'change', 'move', 'hide', 'show', 'put', 'remove', 'minimal', 'compact')) {
    const sh = has(s, 'shader', 'glsl', 'code');
    const anim = has(s, 'animation', 'graph', 'keyframe', 'motion');
    const edit = has(s, 'editing', 'cutting', 'assets', 'media');
    const minimal = has(s, 'minimal', 'focus', 'viewer only', 'clean');
    const name = quoted(text) || (sh ? 'Shader Workshop' : anim ? 'Motion Desk' : edit ? 'Edit Desk' : minimal ? 'Focus Desk' : 'Custom Workspace');
    let docks;
    if (sh) docks = [
      { id: 'left', size: 460, panels: [{ id: 'shader', flex: true }] },
      { id: 'center', panels: [{ id: 'viewer', flex: true }, { id: 'timeline', size: 220 }] },
      { id: 'right', size: 300, panels: [{ id: 'inspector', flex: true }, { id: 'perf', size: 170 }] },
    ];
    else if (anim) docks = [
      { id: 'left', size: 230, panels: [{ id: 'layers', flex: true }, { id: 'takes', size: 150 }] },
      { id: 'center', panels: [{ id: 'viewer', size: 300 }, { id: 'timeline', flex: true }] },
      { id: 'right', size: 320, panels: [{ id: 'inspector', flex: true }] },
    ];
    else if (minimal) docks = [{ id: 'center', panels: [{ id: 'viewer', flex: true }] }];
    else docks = [
      { id: 'left', size: 320, panels: [{ id: 'chat', flex: true }] },
      { id: 'center', panels: [{ id: 'viewer', flex: true }, { id: 'timeline', size: 310 }] },
      { id: 'right', size: 300, panels: [{ id: 'inspector', flex: true }] },
    ];
    const accent = colorIn(s, sh ? '#4C8DFF' : '#FF6B1A');
    const customPanels = [];
    if (has(s, 'control', 'slider')) customPanels.push({
      title: quoted(text) || 'Direction Controls', note: 'Purposeful controls authored for this workspace.',
      controls: [
        { type: 'slider', label: 'Energy', param: 'Energy', def: 1, min: 0, max: 2, step: .01 },
        { type: 'slider', label: 'Atmosphere', param: 'Atmosphere', def: .2, min: 0, max: 1, step: .01 },
        { type: 'color', label: 'Accent', param: 'Accent', def: accent },
      ],
    });
    await doCall('set_workspace', { create: true, name, density: has(s, 'compact', 'dense') ? 'compact' : minimal ? 'normal' : 'normal', theme: { accent }, docks, customPanels, features: { motionBlur: !sh, snapping: true, guides: !minimal, adaptiveQuality: true } });
    say(`I built “${name}” around ${sh ? 'live shader authoring and GPU diagnostics' : anim ? 'keyframes, curves, and direct manipulation' : minimal ? 'an uninterrupted composition view' : 'the active edit'}—the workspace itself is now part of the tool.`);
    return;
  }

  /* shader creation / revision */
  if (has(s, 'shader', 'glsl', 'fragment') || (target(s) && target(s).type === 'shader' && has(s, 'noise', 'gradient', 'liquid', 'grid', 'chrome', 'aurora'))) {
    let L = target(s);
    if (!L || L.type !== 'shader') {
      const r = await doCall('add_layer', { type: 'shader', name: quoted(text) || shaderName(s), from: PM.time, duration: Math.max(1, PM.proj.dur - PM.time), select: true });
      L = PM.L(r.data && r.data.id);
    }
    const source = shaderFor(s, colorIn(s, '#FF6B1A'));
    await doCall('write_shader', { layer: L.id, source, openEditor: has(s, 'editor', 'code') });
    if (has(s, 'glow', 'bloom')) await doCall('add_effect', { layer: L.id, effect: 'glow', parameters: { threshold: 45, radius: 70, intensity: 120 } });
    const dur = L.dur;
    await doCall('look', { times: [L.from, L.from + dur * .35, L.from + dur * .72].map(t => PM.clamp(t, 0, PM.proj.dur)), note: 'Shader evolution' });
    say(`The ${L.name.toLowerCase()} now evolves as a continuous GPU field, with exposed controls for the qualities that matter instead of implementation noise.`);
    return;
  }

  /* composition setup */
  if (has(s, 'vertical', '9:16', 'portrait', 'square', '1:1', '4k', 'resolution', 'fps', 'duration', 'background')) {
    const o = {};
    if (has(s, 'vertical', '9:16', 'portrait')) { o.width = 1080; o.height = 1920; }
    if (has(s, 'square', '1:1')) { o.width = 1080; o.height = 1080; }
    if (has(s, '4k')) { o.width = 3840; o.height = 2160; }
    const fps = numBefore(s, 'fps'); if (fps) o.fps = fps;
    const dur = numBefore(s, '(?:seconds?|secs?|s\\b)'); if (dur) o.duration = dur;
    if (has(s, 'background', 'bg')) o.background = colorIn(s, PM.proj.bg);
    await doCall('set_composition', o);
    say('The composition format is updated and the edit remains fully live at the new dimensions.');
    return;
  }

  /* add text */
  if (has(s, 'add text', 'new text', 'headline', 'title', 'caption', 'write ') && !has(s, 'shader')) {
    const copy = quoted(text) || text.match(/(?:saying|that says|text[: ]+)\s*(.+)$/i)?.[1] || (has(s, 'headline') ? 'Headline' : 'New text');
    const size = numBefore(s, 'px') || (has(s, 'caption') ? 44 : 120);
    const col = colorIn(s, '#F2F2F2');
    const name = has(s, 'caption') ? 'Caption' : has(s, 'title') ? 'Title' : 'Headline';
    const r = await doCall('add_layer', { type: 'text', name, from: PM.time, duration: Math.max(1, PM.proj.dur - PM.time), content: { text: copy, size, color: col, font: 'Geist', weight: has(s, 'bold') ? 700 : 600, align: 'center' }, properties: { 'position.x': PM.proj.w / 2, 'position.y': PM.proj.h / 2 } });
    const L = PM.L(r.data.id);
    if (has(s, 'animate', 'reveal', 'fade', 'move')) await animateEntrance(L, s, doCall);
    say(`I added “${copy}” with a clean ${has(s, 'caption') ? 'supporting' : 'display'} hierarchy${has(s, 'animate', 'reveal', 'fade', 'move') ? ' and a decisive, editable reveal' : ''}.`);
    return;
  }

  /* shape / solid */
  if (has(s, 'add shape', 'rectangle', 'circle', 'ellipse', 'solid', 'background layer')) {
    const type = has(s, 'solid', 'background layer') ? 'solid' : 'shape';
    const shape = has(s, 'circle', 'ellipse') ? 'ellipse' : has(s, 'star') ? 'star' : 'rect';
    const col = colorIn(s, type === 'solid' ? '#151518' : '#4C8DFF');
    const r = await doCall('add_layer', { type, name: quoted(text) || (type === 'solid' ? 'Background' : shape === 'ellipse' ? 'Circle' : 'Shape'), from: PM.time, duration: Math.max(1, PM.proj.dur - PM.time), content: type === 'solid' ? { color: col, w: PM.proj.w, h: PM.proj.h } : { shape, color: col, w: has(s, 'small') ? 240 : 480, h: has(s, 'small') ? 240 : 480, radius: has(s, 'rounded') ? 48 : 0 }, properties: type === 'shape' ? { 'position.x': PM.proj.w / 2, 'position.y': PM.proj.h / 2 } : {} });
    if (has(s, 'animate', 'reveal', 'grow')) await animateEntrance(PM.L(r.data.id), s, doCall);
    say(`The ${type} is in place${has(s, 'animate', 'reveal', 'grow') ? ', with its entrance exposed as real timeline keyframes' : ''}.`);
    return;
  }

  /* effects */
  if (has(s, 'glow', 'blur', 'grain', 'vignette', 'chromatic', 'pixelate', 'posterize', 'displace', 'shadow', 'invert', 'color grade', 'saturat', 'contrast')) {
    const L = target(s); if (!L) { say('Select the layer you want to treat, then ask again.'); return; }
    const map = [['glow','glow'],['blur','blur'],['grain','grain'],['vignette','vignette'],['chromatic','chroma'],['pixel','pixelate'],['poster','posterize'],['displace','displace'],['shadow','shadow'],['invert','invert']];
    let fx = (map.find(([q]) => s.includes(q)) || [null, 'color'])[1];
    const pars = {};
    if (fx === 'glow') Object.assign(pars, { threshold: 50, radius: has(s, 'soft') ? 110 : 55, intensity: has(s, 'subtle') ? 45 : 100 });
    if (fx === 'blur') pars.amount = numBefore(s, 'px') || (has(s, 'soft') ? 42 : 18);
    if (fx === 'grain') Object.assign(pars, { amount: has(s, 'subtle') ? 6 : 14, size: 1.3 });
    if (fx === 'color') Object.assign(pars, { contrast: has(s, 'more') ? 20 : 8, saturation: has(s, 'desatur') ? -65 : has(s, 'saturat') ? 25 : 0, exposure: 0 });
    await doCall('add_effect', { layer: L.id, effect: fx, parameters: pars });
    await doCall('look', { times: [PM.time] });
    say(`I treated “${L.name}” with ${PM.FX[fx].label.toLowerCase()}, keeping the adjustment live and GPU-native.`);
    return;
  }

  /* generic selected-layer animation */
  if (has(s, 'animate', 'fade', 'move', 'slide', 'scale', 'rotate', 'bounce', 'ease', 'faster', 'slower')) {
    const L = target(s); if (!L) { say('Select the layer you want to move, then ask again.'); return; }
    await animateRequest(L, s, doCall);
    await doCall('look', { times: [L.from, L.from + Math.min(.35, L.dur * .2), L.from + Math.min(.8, L.dur * .4)] });
    say(`I reshaped “${L.name}” into a ${has(s, 'soft', 'slow') ? 'gentler, longer' : 'decisive'} motion phrase, with the curve exposed for hand-tuning.`);
    return;
  }

  /* visibility, delete, rename */
  if (has(s, 'delete', 'remove layer')) {
    const L = target(s); await doCall('delete_layers', { layers: L ? [L.id] : [] });
    say('The targeted layer is removed without disturbing the rest of the timeline.'); return;
  }
  if (has(s, 'hide', 'show')) {
    const L = target(s); if (!L) { say('Select the layer you want to change.'); return; }
    await doCall('update_layer', { layer: L.id, visible: !has(s, 'hide') });
    say(`“${L.name}” is now ${has(s, 'hide') ? 'hidden' : 'visible'}.`); return;
  }
  if (has(s, 'rename')) {
    const L = target(s), name = quoted(text); if (!L || !name) { say('Select a layer and put the new name in quotes.'); return; }
    await doCall('update_layer', { layer: L.id, name }); say(`The layer is now named “${name}”.`); return;
  }

  /* full demo / broad creative prompt */
  if (has(s, 'make', 'create', 'design') && has(s, 'video', 'animation', 'intro', 'ad', 'composition', 'motion')) {
    await buildFilm(s, doCall);
    return;
  }

  say('I can act on the composition, timeline, shaders, effects, or the interface itself. Try “animate this in with a soft power curve,” “make a liquid chrome shader,” or “build a compact shader workspace.”');
};

async function animateEntrance(L, s, call) {
  const d = has(s, 'slow', 'gentle') ? 1.1 : has(s, 'fast', 'snappy') ? .35 : .65;
  const ease = has(s, 'linear') ? 'linear' : has(s, 'bounce', 'overshoot') ? 'backOut' : has(s, 'soft') ? 'glide' : 'power';
  if (has(s, 'slide', 'move', 'reveal')) {
    const y = PM.ev(L, 'position.y', L.from);
    await call('animate', { layer: L.id, channel: 'position.y', keyframes: [{ time: 0, value: y + (has(s, 'up') ? 100 : 64), ease }, { time: d, value: y, ease }], replace: true });
  }
  await call('animate', { layer: L.id, channel: 'opacity', keyframes: [{ time: 0, value: 0, ease }, { time: d * .78, value: 100, ease }], replace: true });
  if (has(s, 'scale', 'grow', 'pop')) {
    await call('animate', { layer: L.id, channel: 'scale.x', keyframes: [{ time: 0, value: has(s, 'pop') ? 72 : 92, ease }, { time: d, value: 100, ease }], replace: true });
    await call('animate', { layer: L.id, channel: 'scale.y', keyframes: [{ time: 0, value: has(s, 'pop') ? 72 : 92, ease }, { time: d, value: 100, ease }], replace: true });
  }
}
async function animateRequest(L, s, call) {
  const fast = has(s, 'faster', 'fast', 'snappy');
  const slow = has(s, 'slower', 'slow', 'gentle');
  const d = fast ? .3 : slow ? 1.3 : .7;
  const ease = has(s, 'linear') ? 'linear' : has(s, 'bounce', 'overshoot') ? 'backOut' : has(s, 'soft', 'gentle') ? 'glide' : 'power';
  if (has(s, 'fade') || !has(s, 'move', 'slide', 'scale', 'rotate')) await call('animate', { layer: L.id, channel: 'opacity', keyframes: [{ time: 0, value: has(s, 'out') ? 100 : 0, ease }, { time: d, value: has(s, 'out') ? 0 : 100, ease }], replace: true });
  if (has(s, 'move', 'slide')) {
    const x = PM.ev(L, 'position.x', PM.time), y = PM.ev(L, 'position.y', PM.time);
    const amount = numBefore(s, 'px') || 120;
    if (has(s, 'left', 'right')) await call('animate', { layer: L.id, channel: 'position.x', keyframes: [{ time: 0, value: x + (has(s,'left') ? amount : -amount), ease }, { time: d, value: x, ease }], replace: true });
    else await call('animate', { layer: L.id, channel: 'position.y', keyframes: [{ time: 0, value: y + (has(s,'up') ? amount : -amount), ease }, { time: d, value: y, ease }], replace: true });
  }
  if (has(s, 'scale', 'grow', 'pop')) for (const ch of ['scale.x','scale.y']) await call('animate', { layer: L.id, channel: ch, keyframes: [{ time: 0, value: has(s,'out') ? 100 : 88, ease }, { time: d, value: has(s,'out') ? 80 : 100, ease }], replace: true });
  if (has(s, 'rotate', 'spin')) await call('animate', { layer: L.id, channel: 'rotation', keyframes: [{ time: 0, value: 0, ease }, { time: d, value: has(s,'spin') ? 360 : 12, ease }], replace: true });
}

function shaderName(s) { return has(s,'chrome') ? 'Liquid Chrome' : has(s,'grid') ? 'Signal Grid' : has(s,'gradient') ? 'Living Gradient' : has(s,'noise') ? 'Noise Field' : 'Shader Field'; }
function shaderFor(s, accent) {
  const rgb = accent;
  if (has(s, 'chrome', 'metal', 'liquid')) return `uniform float uSpeed; // @param 0.45 0 3
uniform float uWarp; // @param 2.8 0 8
uniform float uContrast; // @param 1.3 0.4 3
void main(){
  vec2 p=(uv-.5)*2.; p.x*=iResolution.x/iResolution.y;
  float t=iTime*uSpeed;
  for(int i=0;i<5;i++) p += vec2(sin(p.y*3.1+t+float(i)),cos(p.x*2.8-t-float(i)))*(.075*uWarp);
  float n=fbm(p*1.8+t*.17); float edge=pow(abs(sin(n*10.8+p.x*2.)),uContrast);
  vec3 silver=mix(vec3(.025,.03,.04),vec3(.95,.98,1.),smoothstep(.12,.92,edge));
  silver += vec3(.15,.22,.34)*pow(max(0.,1.-abs(edge-.76)*5.),3.);
  fragColor=vec4(silver,1.);
}`;
  if (has(s, 'grid', 'signal', 'tech')) return `uniform float uSpeed; // @param 1.0 0 4
uniform float uCells; // @param 22 4 90
uniform vec3 uSignal; // @param ${rgb}
uniform float uGlow; // @param 1.2 0 3
void main(){
  vec2 g=uv*uCells; vec2 id=floor(g),f=fract(g)-.5;
  float box=sdBox(f,vec2(.34)); float phase=iTime*uSpeed+hash(id)*6.283+id.x*.16-id.y*.09;
  float pulse=smoothstep(.2,1.,sin(phase));
  float cell=smoothstep(.055,.0,abs(box))*(.12+pulse*uGlow);
  float scan=exp(-abs(fract(uv.y-iTime*.08)-.5)*22.);
  vec3 col=vec3(.012,.014,.018)+uSignal*(cell+scan*.2);
  fragColor=vec4(col,1.);
}`;
  if (has(s, 'gradient', 'aurora', 'soft', 'atmosphere')) return `uniform float uSpeed; // @param 0.22 0 2
uniform float uScale; // @param 2.4 0.4 10
uniform vec3 uAccent; // @param ${rgb}
uniform vec3 uShadow; // @param #08090D
uniform float uSoftness; // @param 0.58 0.1 1.4
void main(){
  vec2 p=(uv-.5)*vec2(iResolution.x/iResolution.y,1.);
  float t=iTime*uSpeed;
  float a=fbm(p*uScale+vec2(t,-t*.43));
  float b=fbm(rot(.7)*p*uScale*1.3+vec2(-t*.36,t*.7)+17.);
  float field=smoothstep(.3,.3+uSoftness,a*.7+b*.46);
  vec3 col=mix(uShadow,uAccent,field); col+=uAccent*pow(field,7.)*.55;
  col*=1.-dot(p,p)*.42;
  fragColor=vec4(col,1.);
}`;
  return `uniform float uSpeed; // @param 0.35 0 3
uniform float uScale; // @param 3.2 0.5 14
uniform vec3 uTint; // @param ${rgb}
uniform float uEnergy; // @param 1.0 0 2
void main(){
  vec2 p=(uv-.5)*vec2(iResolution.x/iResolution.y,1.); float t=iTime*uSpeed;
  float n=fbm(p*uScale+vec2(t,t*.57)); n+=.35*fbm(p*uScale*2.2-t*.26);
  float f=smoothstep(.3,.72,n); vec3 col=mix(vec3(.018,.018,.022),uTint,f);
  col+=uTint*pow(f,7.)*.6*uEnergy; col*=1.-dot(p,p)*.5;
  fragColor=vec4(col,1.);
}`;
}

async function buildFilm(s, call) {
  const accent = colorIn(s, '#FF6B1A');
  await call('set_composition', { name: quoted(s) || 'Powermove Film', duration: 8, background: '#080809', width: has(s,'vertical') ? 1080 : 1920, height: has(s,'vertical') ? 1920 : 1080, fps: 30 });
  for (const l of [...PM.proj.layers]) await call('delete_layers', { layers: [l.id] });
  const bg = await call('add_layer', { type: 'shader', name: 'Atmosphere', from: 0, duration: 8, select: false });
  await call('write_shader', { layer: bg.data.id, source: shaderFor('soft aurora gradient', accent) });
  const orb = await call('add_layer', { type: 'shape', name: 'Signal', from: .2, duration: 7.8, content: { shape: 'ellipse', color: accent, w: 360, h: 360 }, properties: { 'position.x': PM.proj.w*.5, 'position.y': PM.proj.h*.5 }, select: false });
  await call('add_effect', { layer: orb.data.id, effect: 'glow', parameters: { threshold: 10, radius: 100, intensity: 145 } });
  await call('animate', { layer: orb.data.id, channel: 'scale.x', keyframes: [{time:0,value:18,ease:'power'},{time:.8,value:100,ease:'power'},{time:5.5,value:100,ease:'glide'},{time:7.2,value:680,ease:'expoIn'}] });
  await call('animate', { layer: orb.data.id, channel: 'scale.y', keyframes: [{time:0,value:18,ease:'power'},{time:.8,value:100,ease:'power'},{time:5.5,value:100,ease:'glide'},{time:7.2,value:680,ease:'expoIn'}] });
  const title = await call('add_layer', { type:'text', name:'Title', from:.45, duration:4.9, content:{text:quoted(s)||'Make the move.',font:'Geist',weight:650,size:150,color:'#F3F3F2',align:'center'}, properties:{'position.x':PM.proj.w/2,'position.y':PM.proj.h/2}, select:false });
  await call('animate', { layer:title.data.id, channel:'position.y', keyframes:[{time:0,value:PM.proj.h/2+90,ease:'power'},{time:.75,value:PM.proj.h/2,ease:'power'},{time:4.2,value:PM.proj.h/2,ease:'easeIn'},{time:4.8,value:PM.proj.h/2-70,ease:'easeIn'}] });
  await call('animate', { layer:title.data.id, channel:'opacity', keyframes:[{time:0,value:0,ease:'power'},{time:.6,value:100,ease:'power'},{time:4.2,value:100,ease:'easeIn'},{time:4.8,value:0,ease:'easeIn'}] });
  const end = await call('add_layer', { type:'text', name:'Lockup', from:5.3, duration:2.7, content:{text:'Powermove',font:'Geist',weight:650,size:110,color:'#0A0A0B',align:'center'}, properties:{'position.x':PM.proj.w/2,'position.y':PM.proj.h/2}, select:false });
  await call('animate', { layer:end.data.id, channel:'opacity', keyframes:[{time:0,value:0,ease:'glide'},{time:.55,value:100,ease:'glide'}] });
  await call('animate', { layer:end.data.id, channel:'scale.x', keyframes:[{time:0,value:92,ease:'power'},{time:.7,value:100,ease:'power'}] });
  await call('animate', { layer:end.data.id, channel:'scale.y', keyframes:[{time:0,value:92,ease:'power'},{time:.7,value:100,ease:'power'}] });
  await call('look', { times:[.3,1.1,4.9,5.7,7.4] });
  await call('review_motion', { checks:{briefFidelity:true,hierarchy:true,legibility:true,temporalDevelopment:true,transitions:true,noSlideshow:true,noDeadTime:true,finish:true}, critique:'One continuous signal expands into the final field; hierarchy stays singular and readable.' });
  say('I built one continuous motion phrase: the signal arrives, holds the idea, then expands into the final lockup instead of resetting into separate cards.');
}
})();
