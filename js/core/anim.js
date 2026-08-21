/* Powermove — animation evaluation. Keyframe times are LAYER-LOCAL seconds. */
(() => {
const PM = window.PM, Ease = PM.Ease, clamp = PM.clamp;

let version = 0;
PM.touch = () => { version++; PM.exprCache.clear(); };
PM.animVersion = () => version;

/* ── keyframe evaluation ───────────────────────────────── */
function evalKfs(kf, t) {
  const n = kf.length;
  if (n === 0) return null;
  if (t <= kf[0].t) return kf[0].v;
  if (t >= kf[n - 1].t) return kf[n - 1].v;
  let lo = 0, hi = n - 1;
  while (hi - lo > 1) { const m = (lo + hi) >> 1; kf[m].t <= t ? (lo = m) : (hi = m); }
  const a = kf[lo], b = kf[hi];
  if (a.hold) return a.v;
  const span = b.t - a.t;
  const u = span <= 0 ? 0 : (t - a.t) / span;
  if (a.spring) return a.v + (b.v - a.v) * Ease.spring(u * span, a.spring);
  const f = Ease.bezier(a.eo[0], a.eo[1], b.ei[0], b.ei[1]);
  return a.v + (b.v - a.v) * f(u);
}
PM.evalKfs = evalKfs;

/* ── expressions ───────────────────────────────────────── */
PM.exprCache = new Map();
const HELPERS = `
const PI=Math.PI, sin=Math.sin, cos=Math.cos, tan=Math.tan, abs=Math.abs, pow=Math.pow,
 sqrt=Math.sqrt, floor=Math.floor, ceil=Math.ceil, round=Math.round, min=Math.min, max=Math.max,
 sign=Math.sign, atan2=Math.atan2, exp=Math.exp, log=Math.log;
const clamp=(v,a,b)=>v<a?a:v>b?b:v;
const lerp=(a,b,u)=>a+(b-a)*u;
const linear=(x,x0,x1,y0,y1)=>x1===x0?y0:y0+(y1-y0)*clamp((x-x0)/(x1-x0),0,1);
const ease=(x,x0,x1,y0,y1)=>{const u=clamp(x1===x0?0:(x-x0)/(x1-x0),0,1);return y0+(y1-y0)*(u*u*(3-2*u));};
const random=(seed)=>{let s=Math.sin((seed==null?1:seed)*127.1)*43758.5453;return s-Math.floor(s);};
const wiggle=(freq,amp,seed)=>{const s=(seed||0)*17.3;let v=0,a=amp,f=freq;
  for(let i=0;i<3;i++){v+=a*(Math.sin(t*f*6.2831+s+i*2.4)+Math.sin(t*f*3.94+s*1.7+i))*0.5;a*=.5;f*=2.03;}return v;};
const bounce=(x)=>{x=clamp(x,0,1);return x<1/2.75?7.5625*x*x:x<2/2.75?7.5625*(x-=1.5/2.75)*x+.75:x<2.5/2.75?7.5625*(x-=2.25/2.75)*x+.9375:7.5625*(x-=2.625/2.75)*x+.984375;};
const loop=(dur,x)=>dur<=0?x:x%dur;
const pingpong=(dur,x)=>{if(dur<=0)return x;const m=x%(dur*2);return m<dur?m:dur*2-m;};
`;
function compile(src) {
  let f = PM.exprCache.get(src);
  if (f !== undefined) return f;
  try {
    f = new Function('t', 'T', 'fps', 'value', 'layer', 'comp', 'param', 'ch', 'idx',
      HELPERS + '\nreturn (' + src + ');');
  } catch (e) { f = null; }
  PM.exprCache.set(src, f);
  return f;
}
PM.compileExpr = compile;

/* ── channel evaluation ────────────────────────────────── */
function evalProp(prop, tLocal, ctx) {
  let v = prop.kf.length ? evalKfs(prop.kf, tLocal) : prop.v;
  if (prop.expr) {
    const f = compile(prop.expr);
    if (f) {
      try {
        const r = f(tLocal, ctx.T, ctx.fps, v, ctx.layer, ctx.comp, ctx.param, ctx.key, ctx.idx);
        if (typeof r === 'number' && isFinite(r)) v = r;
        else if (typeof r === 'string') v = r;
      } catch (e) { /* keep base value */ }
    }
  }
  return v;
}

const paramGet = (name) => {
  const p = PM.proj.params[name];
  return p ? p.value : 0;
};

/** Evaluate one channel of a layer at global time T (seconds). */
PM.ev = (L, key, T) => {
  const prop = L.p[key];
  if (!prop) return 0;
  const tl = T - L.from;
  return evalProp(prop, tl, {
    T, fps: PM.proj.fps, layer: L, comp: PM.proj, param: paramGet,
    key, idx: PM.proj.layers.indexOf(L),
  });
};
/** Evaluate an effect / uniform param (also layer-local). */
PM.evP = (L, prop, T, key) => evalProp(prop, T - L.from, {
  T, fps: PM.proj.fps, layer: L, comp: PM.proj, param: paramGet, key, idx: PM.proj.layers.indexOf(L),
});

PM.active = (L, T) => L.on && T >= L.from - 1e-6 && T < L.from + L.dur - 1e-6;

/* ── transform matrices (2x3, column-major [a,b,c,d,e,f]) ── */
function mul(m, n) {
  return [
    m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1],
    m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3],
    m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5],
  ];
}
PM.mul = mul;

PM.localMatrix = (L, T) => {
  const px = PM.ev(L, 'position.x', T), py = PM.ev(L, 'position.y', T);
  const ax = PM.ev(L, 'anchor.x', T), ay = PM.ev(L, 'anchor.y', T);
  const sx = PM.ev(L, 'scale.x', T) / 100, sy = PM.ev(L, 'scale.y', T) / 100;
  const r = PM.ev(L, 'rotation', T) * Math.PI / 180;
  const sk = Math.tan(PM.ev(L, 'skew', T) * Math.PI / 180);
  const c = Math.cos(r), s = Math.sin(r);
  // translate(p) * rotate * skew * scale * translate(-a)
  let m = [c, s, -s, c, px, py];
  m = mul(m, [1, 0, sk, 1, 0, 0]);
  m = mul(m, [sx, 0, 0, sy, 0, 0]);
  m = mul(m, [1, 0, 0, 1, -ax, -ay]);
  return m;
};

/* Cycle-safe parenting: a visited set breaks parent loops deterministically
   instead of silently freezing transforms after an arbitrary depth cap. */
PM.worldMatrix = (L, T, seen = null) => {
  let m = PM.localMatrix(L, T);
  if (L.parent) {
    seen = seen || new Set();
    if (seen.has(L.id)) return m;
    seen.add(L.id);
    const p = PM.L(L.parent);
    if (p && !seen.has(p.id)) m = mul(PM.worldMatrix(p, T, seen), m);
  }
  return m;
};

PM.worldOpacity = (L, T, seen = null) => {
  let o = PM.ev(L, 'opacity', T) / 100;
  if (L.parent) {
    seen = seen || new Set();
    if (seen.has(L.id)) return clamp(o, 0, 1);
    seen.add(L.id);
    const p = PM.L(L.parent);
    if (p && !seen.has(p.id)) o *= PM.worldOpacity(p, T, seen);
  }
  return clamp(o, 0, 1);
};

/* True when assigning parentId to L would create a parenting cycle. */
PM.wouldCycle = (L, parentId) => {
  if (!parentId) return false;
  if (parentId === L.id) return true;
  let cur = PM.L(parentId), guard = 0;
  while (cur && guard++ < 256) {
    if (cur.id === L.id) return true;
    cur = cur.parent ? PM.L(cur.parent) : null;
  }
  return false;
};

/* ── keyframe operations ───────────────────────────────── */
const sortKf = (p) => p.kf.sort((a, b) => a.t - b.t);

PM.setKey = (L, key, T, value, ease) => {
  const p = L.p[key] || (L.fxProp && L.fxProp[key]);
  if (!p) return null;
  return PM.setKeyOn(p, T - L.from, value, ease, PM.proj.fps);
};
PM.setKeyOn = (p, tLocal, value, ease = 'power', fps = 30) => {
  const t = PM.snapF(tLocal, fps);
  let k = p.kf.find(k => Math.abs(k.t - t) < .5 / fps);
  if (k) { k.v = value; }
  else { k = PM.KF(t, value, ease); p.kf.push(k); sortKf(p); }
  PM.touch();
  return k;
};
PM.removeKey = (p, k) => { p.kf = p.kf.filter(x => x !== k && x.i !== k.i); PM.touch(); };
PM.hasKeyAt = (L, p, T) => p.kf.find(k => Math.abs(k.t - (T - L.from)) < .5 / PM.proj.fps) || null;

PM.toggleStopwatch = (L, key, T) => {
  const p = L.p[key]; if (!p) return;
  if (p.kf.length) { p.v = PM.ev(L, key, T); p.kf = []; }
  else PM.setKeyOn(p, T - L.from, p.v, 'power', PM.proj.fps);
  PM.touch(); PM.invalidate();
};

PM.applyEaseTo = (keys, name) => {
  const { eo, ei } = Ease.handles(name);
  keys.forEach(k => { k.eo = [...eo]; k.ei = [...ei]; k.hold = name === 'hold'; });
  PM.touch();
};

/** Build an animation from a spec: [{t, v, ease}] in layer-local seconds. */
PM.animate = (L, key, points, opt = {}) => {
  const p = L.p[key] || (opt.prop || null);
  if (!p) return false;
  if (!opt.keep) p.kf = [];
  points.forEach(pt => PM.setKeyOn(p, pt.t, pt.v, pt.ease || opt.ease || 'power', PM.proj.fps));
  PM.touch();
  return true;
};

/* Collect every animatable prop of a layer (transform + effects + shader uniforms). */
PM.allProps = (L) => {
  const out = [];
  for (const k in L.p) out.push({ key: k, prop: L.p[k], label: PM.CH[k] ? PM.CH[k].label : k, group: 'Transform' });
  L.fx.forEach(fx => { for (const k in fx.p) out.push({ key: fx.id + '.' + k, prop: fx.p[k], label: k, group: fx.type }); });
  if (L.type === 'shader') for (const k in L.d.uniforms) out.push({ key: 'u.' + k, prop: L.d.uniforms[k], label: k, group: 'Shader' });
  return out;
};
PM.findProp = (L, key) => {
  if (L.p[key]) return L.p[key];
  if (key.startsWith('u.')) return L.d.uniforms && L.d.uniforms[key.slice(2)];
  const [fid, pk] = key.split('.');
  const fx = L.fx.find(f => f.id === fid);
  return fx ? fx.p[pk] : null;
};
})();
