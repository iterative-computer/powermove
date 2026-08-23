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

/* Use the exact same canvas text metrics as the rasterizer when a procedural
   tool needs to reason about glyph placement. The returned offsets are in the
   source text layer's local coordinate system, before its transform. */
function textLayout(d) {
  const size = Math.max(1, Number(d.size) || 16);
  const meas = getCanvas(8, 8).getContext('2d');
  const align = d.align === 'center' ? 'center' : d.align === 'right' ? 'right' : 'left';
  meas.font = fontStr(d);
  meas.textAlign = align;
  meas.textBaseline = 'alphabetic';
  if ('letterSpacing' in meas) meas.letterSpacing = (d.tracking || 0) + 'px';
  const lines = String(d.text == null ? '' : d.text).split('\n');
  const lh = size * (d.leading || 1.15);
  const width = value => meas.measureText(value).width;
  const graphemes = value => {
    if (typeof Intl !== 'undefined' && Intl.Segmenter) {
      return [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(value)].map(item => item.segment);
    }
    return Array.from(value);
  };
  const output = { characters: [], words: [], lines: [] };
  /* measureText(prefix) omits kerning between the prefix and the next glyph.
     Measuring the joined run and subtracting the isolated segment preserves
     that incoming pair adjustment when the segment becomes its own layer. */
  const segmentX = (lineStart, prefix, segment) => lineStart + width(prefix + segment) - width(segment);
  let characterIndex = 0, wordIndex = 0, lineIndex = 0;
  lines.forEach((line, row) => {
    const lineWidth = width(line);
    const startX = align === 'center' ? -lineWidth / 2 : align === 'right' ? -lineWidth : 0;
    const y = row * lh;
    if (line.length) output.lines.push({ text: line, x: startX, y, line: row, index: lineIndex++ });

    let prefix = '';
    for (const segment of graphemes(line)) {
      const x = segmentX(startX, prefix, segment);
      if (!/^\s+$/u.test(segment)) output.characters.push({ text: segment, x, y, line: row, index: characterIndex++ });
      prefix += segment;
    }

    const matcher = /\S+/gu;
    let match;
    while ((match = matcher.exec(line))) {
      const prefix = line.slice(0, match.index);
      output.words.push({ text: match[0], x: segmentX(startX, prefix, match[0]), y, line: row, index: wordIndex++ });
    }
  });
  return output;
}
PM.textLayout = textLayout;

function rasterText(d, scale) {
  const size = Math.max(1, Number(d.size) || 16);
  const pad = Math.ceil(size * .6) + 24;
  const meas = getCanvas(8, 8).getContext('2d');
  const align = d.align === 'center' ? 'center' : d.align === 'right' ? 'right' : 'left';
  meas.font = fontStr(d);
  meas.textAlign = align;
  meas.textBaseline = 'alphabetic';
  if ('letterSpacing' in meas) meas.letterSpacing = (d.tracking || 0) + 'px';
  const lines = String(d.text == null ? '' : d.text).split('\n');
  let wMax = 1;
  const metrics = lines.map(line => {
    const measured = meas.measureText(line);
    wMax = Math.max(wMax, measured.width);
    return measured;
  });
  const lh = size * (d.leading || 1.15);
  const w = Math.ceil(wMax) + pad * 2;
  const hh = Math.ceil(lh * lines.length) + pad * 2;
  const cv = getCanvas(w * scale, hh * scale);
  const c = cv.getContext('2d');
  c.scale(scale, scale);
  c.font = fontStr(d);
  if ('letterSpacing' in c) c.letterSpacing = (d.tracking || 0) + 'px';
  c.textBaseline = 'alphabetic';
  c.textAlign = align;
  c.fillStyle = d.color || '#fff';
  const x = d.align === 'center' ? w / 2 : d.align === 'right' ? w - pad : pad;
  const anchorX = d.align === 'center' ? w / 2 : d.align === 'right' ? w - pad : pad;
  const anchorY = pad;
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  lines.forEach((line, i) => {
    const baseline = pad + lh * i + size * .82;
    const measured = metrics[i];
    c.fillText(line, x, baseline);

    const fallbackLeft = d.align === 'center' ? x - measured.width / 2 : d.align === 'right' ? x - measured.width : x;
    const fallbackRight = fallbackLeft + measured.width;
    const leftMetric = Number(measured.actualBoundingBoxLeft);
    const rightMetric = Number(measured.actualBoundingBoxRight);
    const ascentMetric = Number(measured.actualBoundingBoxAscent);
    const descentMetric = Number(measured.actualBoundingBoxDescent);
    const left = Number.isFinite(leftMetric) && Number.isFinite(rightMetric) && (leftMetric || rightMetric)
      ? x - leftMetric : fallbackLeft;
    const right = Number.isFinite(leftMetric) && Number.isFinite(rightMetric) && (leftMetric || rightMetric)
      ? x + rightMetric : fallbackRight;
    const top = Number.isFinite(ascentMetric) && ascentMetric > 0 ? baseline - ascentMetric : baseline - size * .8;
    const bottom = Number.isFinite(descentMetric) && descentMetric >= 0 ? baseline + descentMetric : baseline + size * .2;
    minX = Math.min(minX, left); maxX = Math.max(maxX, right);
    minY = Math.min(minY, top); maxY = Math.max(maxY, bottom);
  });

  if (!Number.isFinite(minX) || maxX <= minX) { minX = x - .5; maxX = x + .5; }
  if (!Number.isFinite(minY) || maxY <= minY) { minY = pad; maxY = pad + size; }
  const interactionPad = PM.clamp(size * .055, 3, 12);
  const selection = {
    x0: minX - anchorX - interactionPad,
    y0: minY - anchorY - interactionPad,
    x1: maxX - anchorX + interactionPad,
    y1: maxY - anchorY + interactionPad,
  };
  selection.w = selection.x1 - selection.x0;
  selection.h = selection.y1 - selection.y0;
  return { cv, w, h: hh, anchorX, anchorY, selection };
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
PM.rasterClear = () => {
  cache.clear();
  PM.GL && PM.GL.dropTextures && PM.GL.dropTextures('r:');
};

/* ── assets ────────────────────────────────────────────── */
function assetKind(file) {
  const mime = String(file.type || '').toLowerCase();
  if (mime.startsWith('image/')) return 'image';
  if (mime.startsWith('video/')) return 'video';
  if (mime.startsWith('audio/')) return 'audio';
  const ext = String(file.name || '').split('.').pop().toLowerCase();
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp'].includes(ext)) return 'image';
  if (['mp4', 'mov', 'm4v', 'webm'].includes(ext)) return 'video';
  if (['wav', 'mp3', 'm4a', 'aac', 'ogg', 'oga', 'flac', 'aif', 'aiff'].includes(ext)) return 'audio';
  return null;
}
PM.assetKind = assetKind;
async function prepareAsset({ id, name, kind, blob, meta = {} }) {
  const url = URL.createObjectURL(blob);
  let el, w = 0, hh = 0, dur = 0;
  try {
    if (kind === 'image') {
      el = new Image(); el.src = url;
      await el.decode().catch(() => {});
      w = el.naturalWidth; hh = el.naturalHeight;
    } else {
      el = document.createElement(kind === 'video' ? 'video' : 'audio');
      el.src = url; el.preload = 'auto'; el.muted = kind === 'video'; el.playsInline = true;
      await new Promise(r => { el.onloadedmetadata = r; el.onerror = r; setTimeout(r, 6000); });
      if (el.error || (kind === 'audio' && !(Number.isFinite(el.duration) && el.duration > 0))) {
        throw new Error(kind === 'audio'
          ? 'Could not read this audio file · try WAV, MP3, M4A, AAC, OGG, or FLAC'
          : 'Could not read this video file');
      }
      w = el.videoWidth || 0; hh = el.videoHeight || 0; dur = el.duration || 0;
    }
    return {
      id, name, kind, url, el,
      w: w || meta.w || 0, h: hh || meta.h || 0,
      dur: dur || meta.dur || 0, size: blob.size || meta.size || 0,
    };
  } catch (error) {
    URL.revokeObjectURL(url);
    throw error;
  }
}
PM.assets = {
  map: new Map(),
  async add(file) {
    const id = PM.uid('a');
    const kind = assetKind(file);
    if (!kind) throw new Error('Unsupported media file');
    const a = await prepareAsset({ id, name: file.name, kind, blob: file });
    a.persisted = await PM.MediaStore.put(id, file);
    PM.assets.map.set(id, a);
    PM.proj.assets[id] = { id, name: file.name, kind, w: a.w, h: a.h, dur: a.dur, size: a.size };
    PM.bus.emit('assets');
    return a;
  },
  get: (id) => PM.assets.map.get(id),
  clear() {
    for (const a of PM.assets.map.values()) {
      try { if (a.el && a.el.pause) a.el.pause(); } catch (e) { }
      if (a.url && String(a.url).startsWith('blob:')) URL.revokeObjectURL(a.url);
    }
    PM.assets.map.clear();
  },
  async restoreProject(project) {
    const ready = [], missing = [];
    for (const meta of Object.values(project && project.assets || {})) {
      if (!meta || !meta.id || !['image', 'video', 'audio'].includes(meta.kind)) continue;
      const blob = await PM.MediaStore.get(meta.id);
      if (!blob) { missing.push(meta); continue; }
      try {
        const a = await prepareAsset({ id: meta.id, name: meta.name, kind: meta.kind, blob, meta });
        a.persisted = true;
        ready.push(a);
      } catch (e) { missing.push(meta); }
    }
    if (PM.proj !== project) {
      ready.forEach(a => URL.revokeObjectURL(a.url));
      return { restored: [], missing: [], stale: true };
    }
    ready.forEach(a => PM.assets.map.set(a.id, a));
    const restored = ready;
    return { restored, missing };
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
