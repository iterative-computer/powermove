/* Ported from js/gl/raster.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {

const cache = new Map<any, any>();       // key -> {cv, w, h, used}
let tick = 0;
const MAX = 96;

function getCanvas(w: any, h: any) {
  const cv = window.document.createElement('canvas');
  cv.width = Math.max(1, Math.ceil(w)); cv.height = Math.max(1, Math.ceil(h));
  return cv;
}

function evict() {
  if (cache.size <= MAX) return;
  const arr = [...cache.entries()].sort((a, b) => a[1].used - b[1].used);
  for (let i = 0; i < arr.length - MAX; i++) cache.delete((arr[i] as any)[0]);
}

/* ── text ──────────────────────────────────────────────── */
function fontStr(d: any) {
  return `${d.italic ? 'italic ' : ''}${d.weight || 500} ${d.size}px "${d.font}", "Geist", -apple-system, sans-serif`;
}

/* Use the exact same canvas text metrics as the rasterizer when a procedural
   tool needs to reason about glyph placement. The returned offsets are in the
   source text layer's local coordinate system, before its transform. */
function textLayout(d: any) {
  const size = Math.max(1, Number(d.size) || 16);
  const meas = getCanvas(8, 8).getContext('2d') as any;
  const align = d.align === 'center' ? 'center' : d.align === 'right' ? 'right' : 'left';
  meas.font = fontStr(d);
  meas.textAlign = align;
  meas.textBaseline = 'alphabetic';
  if ('letterSpacing' in meas) meas.letterSpacing = (d.tracking || 0) + 'px';
  const lines = String(d.text == null ? '' : d.text).split('\n');
  const lh = size * (d.leading || 1.15);
  const width = (value: any) => meas.measureText(value).width;
  const graphemes = (value: any): any[] => {
    if (typeof Intl !== 'undefined' && Intl.Segmenter) {
      return [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(value)].map(item => item.segment);
    }
    return Array.from(value);
  };
  const output: any = { characters: [], words: [], lines: [] };
  /* measureText(prefix) omits kerning between the prefix and the next glyph.
     Measuring the joined run and subtracting the isolated segment preserves
     that incoming pair adjustment when the segment becomes its own layer. */
  const segmentX = (lineStart: any, prefix: any, segment: any) => lineStart + width(prefix + segment) - width(segment);
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

function rasterText(d: any, scale: any) {
  const size = Math.max(1, Number(d.size) || 16);
  const pad = Math.ceil(size * .6) + 24;
  const meas = getCanvas(8, 8).getContext('2d') as any;
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
  const c = cv.getContext('2d') as any;
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
  const selection: any = {
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
function rr(c: any, x: any, y: any, w: any, h: any, r: any) {
  r = Math.min(r, Math.abs(w) / 2, Math.abs(h) / 2);
  c.beginPath();
  c.moveTo(x + r, y);
  c.arcTo(x + w, y, x + w, y + h, r);
  c.arcTo(x + w, y + h, x, y + h, r);
  c.arcTo(x, y + h, x, y, r);
  c.arcTo(x, y, x + w, y, r);
  c.closePath();
}

function rasterShape(d: any, scale: any) {
  const pad = Math.ceil((d.stroke || 0) / 2) + 4;
  const w = d.w + pad * 2, hh = d.h + pad * 2;
  const cv = getCanvas(w * scale, hh * scale);
  const c = cv.getContext('2d') as any;
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
PM.raster = (L: any, scale: any = 1) => {
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
function assetKind(file: any) {
  const mime = String(file.type || '').toLowerCase();
  if (mime.startsWith('image/')) return 'image';
  if (mime.startsWith('video/')) return 'video';
  if (PM.Audio && PM.Audio.accepts(file)) return 'audio';
  const ext = (String(file.name || '').split('.').pop() as any).toLowerCase();
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp'].includes(ext)) return 'image';
  if (['mp4', 'mov', 'm4v', 'webm'].includes(ext)) return 'video';
  return null;
}
PM.assetKind = assetKind;
function disposeAsset(a: any) {
  if (!a) return;
  if (a.kind === 'audio' && PM.Audio) PM.Audio.disposeAsset(a);
  try { if (a.el && a.el.pause) a.el.pause(); } catch (e) { }
  try { if (a.el && a.el.close) a.el.close(); } catch (e) { }
  if (a.url && String(a.url).startsWith('blob:')) window.URL.revokeObjectURL(a.url);
}
function waitForVideoMetadata(el: any, timeout: any = 15000) {
  return new Promise((resolve: any, reject: any) => {
    let settled = false;
    const finish: any = (error: any) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timer);
      el.removeEventListener('loadedmetadata', loaded);
      el.removeEventListener('durationchange', loaded);
      el.removeEventListener('error', failed);
      error ? reject(error) : resolve();
    };
    const loaded = () => {
      const duration = Number(el.duration);
      if (Number.isFinite(duration) && duration > 0) finish();
    };
    const failed = () => finish(new Error('Could not read this video file'));
    const timer = window.setTimeout(() => finish(new Error('Timed out reading video metadata')), timeout);
    el.addEventListener('loadedmetadata', loaded);
    el.addEventListener('durationchange', loaded);
    el.addEventListener('error', failed);
    loaded();
  });
}
async function prepareAsset({ id, name, kind, blob, meta = {} }: any) {
  if (kind === 'audio') return PM.Audio.prepareAsset({ id, name, blob, meta });
  const url = window.URL.createObjectURL(blob);
  let el: any, w = 0, hh = 0, dur = 0;
  try {
    if (kind === 'image') {
      if (window.createImageBitmap) {
        try { el = await window.createImageBitmap(blob); } catch (e) { }
        if (el) { w = el.width; hh = el.height; }
      }
      if (!el) {
        el = new window.Image(); el.src = url;
        await el.decode();
        w = el.naturalWidth; hh = el.naturalHeight;
      }
      if (!(w > 0 && hh > 0)) throw new Error('Could not read this image file');
    } else if (kind === 'video') {
      el = window.document.createElement('video');
      el.preload = 'metadata'; el.muted = true; el.playsInline = true;
      const ready = waitForVideoMetadata(el);
      el.src = url;
      try { el.load(); } catch (e) { }
      await ready;
      w = el.videoWidth || 0; hh = el.videoHeight || 0; dur = el.duration || 0;
    } else throw new Error('Unsupported media kind');
    return {
      id, name, kind, url, el,
      w: w || meta.w || 0, h: hh || meta.h || 0,
      dur: dur || meta.dur || 0, size: blob.size || meta.size || 0,
    };
  } catch (error) {
    try { if (el && el.close) el.close(); } catch (e) { }
    window.URL.revokeObjectURL(url);
    throw error;
  }
}
let assetEpoch = 0;
async function ingestAsset(file: any, { silent = false }: any = {}) {
  const targetProject = PM.proj;
  const targetEpoch = assetEpoch;
  const assertCurrentProject = () => {
    if (PM.proj !== targetProject || assetEpoch !== targetEpoch) {
      const error = new Error('Import stopped because you switched projects · import the file again in the intended project');
      (error as any).code = 'STALE_MEDIA_IMPORT';
      throw error;
    }
  };
  const kind = assetKind(file);
  if (!kind) throw new Error('Unsupported media file');
  const fingerprint = await PM.MediaImport.fingerprint(file);
  assertCurrentProject();
  const storageKey = PM.MediaImport.storageKeyFor(fingerprint);
  const provisionalId = PM.uid('a');
  /* Persistence and metadata decoding are independent. Starting both together
     removes a full-file wait from the critical import path. */
  const persist = PM.MediaStore.put(storageKey, file, { storageKey, fingerprint, type: file.type });
  const preparation = prepareAsset({ id: provisionalId, name: file.name, kind, blob: file });
  const [preparedResult, persistedResult]: any = await Promise.allSettled([preparation, persist]);
  const prepared = preparedResult.status === 'fulfilled' ? preparedResult.value : null;
  if (preparedResult.status === 'rejected' || persistedResult.status === 'rejected') {
    if (prepared) disposeAsset(prepared);
    throw preparedResult.status === 'rejected' ? preparedResult.reason : persistedResult.reason;
  }
  try { assertCurrentProject(); }
  catch (error) { disposeAsset(prepared); throw error; }
  const persisted = persistedResult.value;
  const identity = {
    name: file.name, kind, fingerprint, storageKey,
    size: prepared.size, dur: prepared.dur, w: prepared.w, h: prepared.h,
    channels: prepared.channels || 0, sampleRate: prepared.sampleRate || 0,
  };
  const plan = PM.MediaImport.match(PM.proj, PM.assets.map, identity);
  const existingMeta = plan.canonicalId && PM.proj.assets[plan.canonicalId];
  const existingLive = plan.canonicalId && PM.assets.map.get(plan.canonicalId);
  const wasMissing = !!(existingMeta && !existingLive);
  const id = plan.canonicalId || provisionalId;
  let asset = prepared;
  if (existingLive) {
    disposeAsset(prepared);
    asset = existingLive;
  } else {
    asset.id = id;
    PM.assets.map.set(id, asset);
    if (kind === 'audio' && PM.Audio) PM.Audio.rebalanceCache();
  }
  Object.assign(asset, identity, { id, persisted });
  PM.proj.assets[id] = { id, ...identity, persisted };
  const relinkedLayers = PM.MediaImport.coalesce(PM.proj, id, plan.aliases);
  plan.aliases.forEach((alias: any) => {
    const duplicate = PM.assets.map.get(alias);
    if (duplicate && duplicate !== asset) disposeAsset(duplicate);
    PM.assets.map.delete(alias);
  });
  const relinked = wasMissing || plan.aliases.length > 0;
  const result = {
    asset,
    status: relinked ? 'relinked' : existingLive ? 'reused' : 'created',
    relinkedLayers,
    retiredAssets: plan.aliases.length,
    persisted,
  };
  if (!silent) {
    PM.touch();
    PM.bus.emit('assets');
    if (relinkedLayers) PM.bus.emit('layers');
  }
  return result;
}
PM.assets = {
  map: new Map<any, any>(),
  async add(file: any, options: any) {
    const result = await ingestAsset(file, options);
    result.asset.importResult = result;
    return result.asset;
  },
  async importBatch(files: any, { concurrency, onProgress }: any = {}) {
    const list = Array.from(files || []);
    const cores = Math.max(1, Number(window.navigator && window.navigator.hardwareConcurrency) || 4);
    const limit = concurrency == null ? Math.max(1, Math.min(3, Math.floor(cores / 2))) : concurrency;
    let completed = 0;
    const results = await PM.MediaImport.mapBounded(list, limit, async (file: any, index: any) => {
      let result: any;
      try { result = await ingestAsset(file, { silent: true }); }
      catch (error) { result = { file, status: 'failed', error }; }
      completed++;
      if (onProgress) onProgress({ completed, total: list.length, index, file, result });
      return result;
    });
    if (results.some((result: any) => result.status !== 'failed')) {
      PM.touch();
      PM.bus.emit('assets');
      if (results.some((result: any) => result.relinkedLayers)) PM.bus.emit('layers');
    }
    return results;
  },
  get: (id: any) => PM.assets.map.get(id),
  clear() {
    assetEpoch++;
    for (const a of PM.assets.map.values()) disposeAsset(a);
    PM.assets.map.clear();
  },
  async restoreProject(project: any) {
    const epoch = assetEpoch;
    const metas: any[] = Object.values(project && project.assets || {})
      .filter((meta: any) => meta && meta.id && ['image', 'video', 'audio'].includes(meta.kind));
    const restored: any[] = [], missing: any[] = [];
    const results = await PM.MediaImport.mapBounded(metas, 3, async (meta: any) => {
      if (epoch !== assetEpoch || PM.proj !== project) return { stale: true };
      const blob = await PM.MediaStore.get(meta);
      if (!blob) return { meta, missing: true };
      try {
        const asset = await prepareAsset({ id: meta.id, name: meta.name, kind: meta.kind, blob, meta });
        Object.assign(asset, {
          fingerprint: meta.fingerprint, storageKey: meta.storageKey, persisted: true,
          channels: asset.channels || meta.channels || 0,
          sampleRate: asset.sampleRate || meta.sampleRate || 0,
        });
        return { asset };
      } catch (error) { return { meta, missing: true, error }; }
    });
    results.forEach((result: any) => {
      if (result.asset) restored.push(result.asset);
      else if (result.missing) missing.push(result.meta);
    });
    if (epoch !== assetEpoch || PM.proj !== project) {
      restored.forEach(disposeAsset);
      return { restored: [], missing: [], stale: true };
    }
    restored.forEach((a: any) => PM.assets.map.set(a.id, a));
    return { restored, missing };
  },
  /** Procedural placeholder so demo projects work with zero imports. */
  gradient(name: any, c0: any, c1: any) {
    const cv = getCanvas(1280, 720);
    const c = cv.getContext('2d') as any;
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
}
