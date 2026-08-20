/* Powermove — CPU rasterizer for text & vector content, plus the asset store.
   Everything is cached by a content key so scrubbing never re-rasterizes. */
(() => {
const PM = window.PM;

const cache = new Map();       // key -> {cv, w, h, used}
let tick = 0;
const MAX = 96;

function getCanvas(w, h) {
  const cv = document.createElement('canvas');
  cv.width = Math.max(1, Math.ceil(w)); cv.height = Math.max(1, Math.ceil(h));
  return cv;
}

function evict() {
  if (cache.size <= MAX) return;
  const arr = [...cache.entries()].sort((a, b) => a[1].used - b[1].used);
  for (let i = 0; i < arr.length - MAX; i++) cache.delete(arr[i][0]);
}

/* ── text ──────────────────────────────────────────────── */
function fontStr(d) {
  return `${d.italic ? 'italic ' : ''}${d.weight || 500} ${d.size}px "${d.font}", "Geist", -apple-system, sans-serif`;
}

function rasterText(d, scale) {
  const pad = Math.ceil(d.size * .6) + 24;
  const meas = getCanvas(8, 8).getContext('2d');
  meas.font = fontStr(d);
  if ('letterSpacing' in meas) meas.letterSpacing = (d.tracking || 0) + 'px';
  const lines = String(d.text == null ? '' : d.text).split('\n');
  let wMax = 1;
  for (const l of lines) wMax = Math.max(wMax, meas.measureText(l).width);
  const lh = d.size * (d.leading || 1.15);
  const w = Math.ceil(wMax) + pad * 2;
  const hh = Math.ceil(lh * lines.length) + pad * 2;
  const cv = getCanvas(w * scale, hh * scale);
  const c = cv.getContext('2d');
  c.scale(scale, scale);
  c.font = fontStr(d);
  if ('letterSpacing' in c) c.letterSpacing = (d.tracking || 0) + 'px';
  c.textBaseline = 'alphabetic';
  c.textAlign = d.align === 'center' ? 'center' : d.align === 'right' ? 'right' : 'left';
  c.fillStyle = d.color || '#fff';
  const x = d.align === 'center' ? w / 2 : d.align === 'right' ? w - pad : pad;
  lines.forEach((l, i) => c.fillText(l, x, pad + lh * i + d.size * .82));
  return { cv, w, h: hh, anchorX: d.align === 'center' ? w / 2 : d.align === 'right' ? w - pad : pad, anchorY: pad };
}

/* ── shapes ────────────────────────────────────────────── */
function rr(c, x, y, w, h, r) {
  r = Math.min(r, Math.abs(w) / 2, Math.abs(h) / 2);
  c.beginPath();
  c.moveTo(x + r, y);
  c.arcTo(x + w, y, x + w, y + h, r);
  c.arcTo(x + w, y + h, x, y + h, r);
  c.arcTo(x, y + h, x, y, r);
  c.arcTo(x, y, x + w, y, r);
  c.closePath();
}

function rasterShape(d, scale) {
  const pad = Math.ceil((d.stroke || 0) / 2) + 4;
  const w = d.w + pad * 2, hh = d.h + pad * 2;
  const cv = getCanvas(w * scale, hh * scale);
  const c = cv.getContext('2d');
  c.scale(scale, scale);
  c.translate(pad, pad);
  c.fillStyle = d.color; c.strokeStyle = d.strokeColor || '#fff'; c.lineWidth = d.stroke || 0;
  c.lineJoin = 'round';
  const W = d.w, H = d.h;
  if (d.shape === 'ellipse') { c.beginPath(); c.ellipse(W / 2, H / 2, W / 2, H / 2, 0, 0, 7); }
  else if (d.shape === 'polygon' || d.shape === 'star') {
    const n = Math.max(3, d.points || 5), R = Math.min(W, H) / 2, r2 = d.shape === 'star' ? R * .46 : R;
    c.beginPath();
    for (let i = 0; i < n * (d.shape === 'star' ? 2 : 1); i++) {
      const a = -Math.PI / 2 + (i * Math.PI * 2) / (n * (d.shape === 'star' ? 2 : 1));
      const rad = d.shape === 'star' && i % 2 ? r2 : R;
      c[i ? 'lineTo' : 'moveTo'](W / 2 + Math.cos(a) * rad, H / 2 + Math.sin(a) * rad);
    }
    c.closePath();
  } else if (d.shape === 'line') {
    c.beginPath(); c.moveTo(0, H / 2); c.lineTo(W, H / 2);
    c.lineWidth = Math.max(1, d.stroke || 6); c.strokeStyle = d.color; c.lineCap = 'round'; c.stroke();
    return { cv, w, h: hh, anchorX: pad, anchorY: pad };
  } else rr(c, 0, 0, W, H, d.radius || 0);
  c.fill();
  if (d.stroke > 0) c.stroke();
  return { cv, w, h: hh, anchorX: pad, anchorY: pad };
}

/** Get (and cache) a rasterized bitmap for a layer. `scale` = render supersample. */
PM.raster = (L, scale = 1) => {
  const d = L.d;
  const key = L.type === 'text'
    ? 't|' + [d.text, d.font, d.weight, d.size, d.tracking, d.leading, d.color, d.align, d.italic, scale].join('|')
    : 's|' + [d.shape, d.color, d.w, d.h, d.radius, d.stroke, d.strokeColor, d.points, scale].join('|');
  let e = cache.get(key);
  if (!e) {
    e = L.type === 'text' ? rasterText(d, scale) : rasterShape(d, scale);
    e.dirty = true;
    cache.set(key, e);
    evict();
  }
  e.used = ++tick;
  e.key = key;
  return e;
};
PM.rasterStats = () => ({ size: cache.size });
PM.rasterClear = () => cache.clear();

/* ── assets ────────────────────────────────────────────── */
PM.assets = {
  map: new Map(),
  async add(file) {
    const id = PM.uid('a');
    const url = URL.createObjectURL(file);
    const kind = file.type.startsWith('video') ? 'video' : file.type.startsWith('audio') ? 'audio' : 'image';
    let el, w = 0, hh = 0, dur = 0;
    if (kind === 'image') {
      el = new Image(); el.src = url;
      await el.decode().catch(() => {});
      w = el.naturalWidth; hh = el.naturalHeight;
    } else {
      el = document.createElement(kind === 'video' ? 'video' : 'audio');
      el.src = url; el.preload = 'auto'; el.muted = kind === 'video'; el.playsInline = true;
      await new Promise(r => { el.onloadedmetadata = r; el.onerror = r; setTimeout(r, 6000); });
      w = el.videoWidth || 0; hh = el.videoHeight || 0; dur = el.duration || 0;
    }
    const a = { id, name: file.name, kind, url, el, w, h: hh, dur, size: file.size };
    PM.assets.map.set(id, a);
    PM.proj.assets[id] = { id, name: file.name, kind, w, h: hh, dur };
    PM.bus.emit('assets');
    return a;
  },
  get: (id) => PM.assets.map.get(id),
  remove(id) {
    const live = PM.assets.map.get(id);
    if (live && live.url) try { URL.revokeObjectURL(live.url); } catch (e) { }
    PM.assets.map.delete(id);
    delete PM.proj.assets[id];
    PM.proj.layers.forEach(L => {
      if (L.d && L.d.asset === id) { L.d.asset = null; L.on = false; }
    });
    PM.touch();
    PM.bus.emit('assets');
    PM.bus.emit('layers');
  },
  /** Procedural placeholder so demo projects work with zero imports. */
  gradient(name, c0, c1) {
    const cv = getCanvas(1280, 720);
    const c = cv.getContext('2d');
    const g = c.createLinearGradient(0, 0, 1280, 720);
    g.addColorStop(0, c0); g.addColorStop(1, c1);
    c.fillStyle = g; c.fillRect(0, 0, 1280, 720);
    const id = PM.uid('a');
    const a = { id, name, kind: 'image', el: cv, w: 1280, h: 720, dur: 0, size: 0 };
    PM.assets.map.set(id, a);
    PM.proj.assets[id] = { id, name, kind: 'image', w: 1280, h: 720 };
    return a;
  },
};
})();
