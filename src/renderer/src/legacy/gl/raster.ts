import { fontAnchorOffset } from '../core/font-anchor';
import { animatedGlyphs, textControlValues } from '../core/text-animation';
import { rasterPaths, pathValues, groupMatrix } from '../core/vector-paths';
import { createVariableFontRenderer, variationEntries, variationSettings } from '../../typography/font-renderer';
export { variationEntries as textVariationEntries, variationSettings as formatFontVariationSettings } from '../../typography/font-renderer';
import { resolveContent } from '../core/content-properties';
/* Ported from js/gl/raster.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { parseObj } from '../../kernel/obj';

const VIDEO_READ_FAILURE = 'Could not read this video file';

/** Selects user-facing copy for video metadata failures without depending on DOM APIs. */
export function videoImportFailureMessage(fileName: unknown, failure: unknown): string {
  const name = String(fileName || '');
  const extension = /\.([^.]+)$/.exec(name)?.[1]?.toLowerCase();
  if (extension !== 'mov' && extension !== 'mp4') return VIDEO_READ_FAILURE;

  const detail = typeof failure === 'string'
    ? failure
    : failure && typeof failure === 'object' && 'message' in failure
      ? String((failure as { message?: unknown }).message || '')
      : '';
  const rawCode = failure && typeof failure === 'object' && 'code' in failure
    ? Number((failure as { code?: unknown }).code)
    : Number.NaN;
  const unsupportedCodec = rawCode === 3 || rawCode === 4
    || /codec|decod(?:e|er|ing)|demux|no[_\s-]*supported[_\s-]*streams|src[_\s-]*not[_\s-]*supported|format error/i.test(detail);
  if (!unsupportedCodec) return VIDEO_READ_FAILURE;

  return extension === 'mov'
    ? "This file's codec is not supported by this build · ProRes .mov files need transcoding before import"
    : "This file's codec is not supported by this build · transcode it to H.264, HEVC, VP9, or AV1 and import it again";
}

export function waitForPresentedVideoFrame(el: any, timeout = 1800): Promise<boolean> {
  if (typeof el.requestVideoFrameCallback !== 'function') return Promise.resolve(true);
  return new Promise(resolve => {
    let settled = false;
    let callback = 0;
    const finish = (presented: boolean) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timer);
      if (!presented && callback && typeof el.cancelVideoFrameCallback === 'function') {
        try { el.cancelVideoFrameCallback(callback); } catch (e) { }
      }
      try { el.pause(); } catch (e) { }
      resolve(presented);
    };
    callback = el.requestVideoFrameCallback(() => finish(true));
    const timer = window.setTimeout(() => finish(false), timeout);
    try {
      Promise.resolve(el.play()).catch(() => finish(false));
    } catch (e) { finish(false); }
  });
}


function cssString(value: unknown): string {
  return String(value ?? '').replace(/\\/g, '\\\\').replace(/"/g, '\\"');
}

export function install(PM: PMRegistry): void {

const cache = new Map<any, any>();       // key -> {cv, w, h, used}
let tick = 0;
const MAX = 96;
const MAX_BYTES = PM.Memory?.budget?.('raster') || 192 * 1024 * 1024;
let cacheBytes = 0;

function getCanvas(w: any, h: any) {
  const cv = window.document.createElement('canvas');
  cv.width = Math.max(1, Math.ceil(w)); cv.height = Math.max(1, Math.ceil(h));
  return cv;
}

function release(key: any, entry: any) {
  if (!entry) return;
  cache.delete(key);
  cacheBytes = Math.max(0, cacheBytes - (entry.bytes || 0));
  PM.GL?.dropTextures?.('r:' + key);
  if (entry.cv) { entry.cv.width = 0; entry.cv.height = 0; }
}

function evict(targetBytes: any = MAX_BYTES) {
  if (cache.size <= MAX && cacheBytes <= targetBytes) return;
  const arr = [...cache.entries()].sort((a, b) => a[1].used - b[1].used);
  for (const [key, entry] of arr) {
    if (cache.size <= MAX && cacheBytes <= targetBytes) break;
    if (cache.size <= 1) break;
    release(key, entry);
  }
}

/* ── text ──────────────────────────────────────────────── */
const variationFontFamily = createVariableFontRenderer(() => { PM.rasterClear?.(); PM.invalidate?.(); });

function fontStr(d: any) {
  const variableFamily = variationFontFamily(d);
  const families = [variableFamily, d.font, 'SF Pro Display']
    .filter(Boolean)
    .map((family) => `"${cssString(family)}"`)
    .join(', ');
  return `${d.italic ? 'italic ' : ''}${d.weight || 500} ${d.size}px ${families}, -apple-system, sans-serif`;
}

function resolvedTextContent(input: any, time = PM.time) {
  const layer = input?.type === 'text' ? input : null;
  const d = layer ? resolveContent(PM, layer, time) : input;
  const value = (key: string) => {
    const source = d?.[key];
    if (!source || typeof source !== 'object' || Array.isArray(source)) return Number(source) || 0;
    return layer && PM.evP
      ? Number(PM.evP(layer, source, time, 'c.' + key)) || 0
      : Number(source.v) || 0;
  };
  const boxWidth = Math.max(0, value('boxWidth'));
  const boxHeight = Math.max(0, value('boxHeight'));
  const resolved = { ...d, boxWidth, boxHeight, paragraph: boxWidth > 0 };
  if (layer && PM.evP) {
    for (const key of Object.keys(d ?? {})) {
      if (!key.startsWith('fontAxis.')) continue;
      const source = d[key];
      if (!source || typeof source !== 'object' || !Array.isArray(source.kf)) continue;
      resolved[key] = Number(PM.evP(layer, source, time, 'c.' + key));
    }
  }
  return resolved;
}

/** The Type tool's drag gesture creates AE-style paragraph text. Keep the
    complete source string, but wrap its rendered lines inside the authored
    box and clip overflow below the box. */
function textLines(d: any, context: any, lineHeight: number): string[] {
  const source = String(d.text == null ? '' : d.text).split('\n');
  const boxWidth = Number(d.boxWidth);
  if (!d.paragraph || !Number.isFinite(boxWidth) || boxWidth <= 0) return source;
  const width = (value: string) => context.measureText(value).width;
  const wrapped: string[] = [];
  for (const paragraph of source) {
    if (!paragraph) { wrapped.push(''); continue; }
    let line = '';
    for (const token of paragraph.split(/(\s+)/u).filter(Boolean)) {
      const candidate = line + token;
      if (line && width(candidate) > boxWidth) {
        wrapped.push(line.trimEnd());
        line = token.trimStart();
      } else line = candidate;
      while (line && width(line) > boxWidth) {
        let cut = 1;
        while (cut < line.length && width(line.slice(0, cut + 1)) <= boxWidth) cut++;
        wrapped.push(line.slice(0, cut));
        line = line.slice(cut);
      }
    }
    wrapped.push(line.trimEnd());
  }
  const boxHeight = Number(d.boxHeight);
  if (!Number.isFinite(boxHeight) || boxHeight <= 0) return wrapped;
  return wrapped.slice(0, Math.max(1, Math.floor(boxHeight / Math.max(1, lineHeight))));
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
  const lh = size * (d.leading || 1.15);
  const lines = textLines(d, meas, lh);
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
  let characterIndex = 0, wordIndex = 0, lineIndex = 0, sourceOffset=0;
  lines.forEach((line, row) => {
    const lineWidth = width(line);
    const startX = align === 'center' ? -lineWidth / 2 : align === 'right' ? -lineWidth : 0;
    const y = row * lh;
    if (line.length) output.lines.push({ text: line, x: startX, y, line: row, index: lineIndex++ });

    const lineSource=Math.max(sourceOffset,String(d.text||'').indexOf(line,sourceOffset));
    let prefix = '';let localWord=-1,wasSpace=true;
    for (const segment of graphemes(line)) {
      const x = segmentX(startX, prefix, segment);
      const space=/^\s+$/u.test(segment);if(!space&&wasSpace)localWord++;
      if (!space) output.characters.push({ text: segment, x, y, line: row, word:wordIndex+localWord, sourceStart:lineSource+prefix.length,index: characterIndex++ });
      wasSpace=space;
      prefix += segment;
    }

    sourceOffset=lineSource+line.length;
    const matcher = /\S+/gu;
    let match;
    while ((match = matcher.exec(line))) {
      const prefix = line.slice(0, match.index);
      output.words.push({ text: match[0], x: segmentX(startX, prefix, match[0]), y, line: row, index: wordIndex++ });
    }
  });
  return output;
}
PM.textLayout = (input: any, time = PM.time) => textLayout(resolvedTextContent(input, time));

function rasterText(d: any, scale: any) {
  const size = Math.max(1, Number(d.size) || 16);
  const pad = Math.ceil(size * .6) + 24;
  const meas = getCanvas(8, 8).getContext('2d') as any;
  const align = d.align === 'center' ? 'center' : d.align === 'right' ? 'right' : 'left';
  meas.font = fontStr(d);
  meas.textAlign = align;
  meas.textBaseline = 'alphabetic';
  if ('letterSpacing' in meas) meas.letterSpacing = (d.tracking || 0) + 'px';
  const lh = size * (d.leading || 1.15);
  const lines = textLines(d, meas, lh);
  let wMax = 1;
  const metrics = lines.map(line => {
    const measured = meas.measureText(line);
    wMax = Math.max(wMax, measured.width);
    return measured;
  });
  const paragraphWidth = d.paragraph && Number.isFinite(Number(d.boxWidth)) ? Math.max(1, Number(d.boxWidth)) : 0;
  const paragraphHeight = d.paragraph && Number.isFinite(Number(d.boxHeight)) ? Math.max(1, Number(d.boxHeight)) : 0;
  const w = Math.ceil(Math.max(wMax, paragraphWidth)) + pad * 2;
  const hh = Math.ceil(Math.max(lh * lines.length, paragraphHeight)) + pad * 2;
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
  if (paragraphWidth > 0) {
    selection.x0 = pad - anchorX - interactionPad;
    selection.x1 = pad + paragraphWidth - anchorX + interactionPad;
    selection.y0 = pad - anchorY - interactionPad;
    selection.y1 = pad + Math.max(paragraphHeight, lh) - anchorY + interactionPad;
  }
  selection.w = selection.x1 - selection.x0;
  selection.h = selection.y1 - selection.y0;
  return { cv, w, h: hh, anchorX, anchorY, selection };
}

function rasterAnimatedText(layer:any,d:any,time:number,scale:number) {
  const glyphs=animatedGlyphs(PM,layer,time,d,textLayout(d)),measure=getCanvas(8,8).getContext('2d')!;
  const [animators,styles]=textControlValues(PM,layer,time);
  if(!styles.length&&animators.every((a:any)=>!a.p.x&&!a.p.y&&!a.p.rotation&&!a.p.tracking&&a.p.scale===100&&a.p.opacity===100))return rasterText(d,scale);
  // Range font metrics contribute to the following characters' advances.
  for(const line of new Set(glyphs.map((g:any)=>g.line))){
    const run=glyphs.filter((g:any)=>g.line===line);let advance=0;
    for(const glyph of run){glyph.x+=advance;measure.font=fontStr(d);const base=measure.measureText(glyph.text).width;measure.font=fontStr(glyph.style);advance+=measure.measureText(glyph.text).width-base;}
    const align=d.align==='center'?.5:d.align==='right'?1:0;for(const glyph of run)glyph.x-=advance*align;
  }
  let x0=0,y0=0,x1=1,y1=Math.max(1,d.size);
  const records=glyphs.map((g:any)=>{measure.font=fontStr(g.style);const metrics=measure.measureText(g.text),w=metrics.width,h=Number(g.style.size)||d.size,r=g.rotation*Math.PI/180,c=Math.cos(r)*g.scale,s=Math.sin(r)*g.scale,baseline=g.y+d.size*.82;
    for(const [x,y] of ([[0,-h],[w,-h],[0,h*.3],[w,h*.3]] as Array<[number,number]>)){const xx=g.x+c*x-s*y,yy=baseline+s*x+c*y;x0=Math.min(x0,xx-4);y0=Math.min(y0,yy-4);x1=Math.max(x1,xx+4);y1=Math.max(y1,yy+4);}return {...g,baseline};});
  const w=Math.max(1,x1-x0),h=Math.max(1,y1-y0),density=Math.min(scale,8192/w,8192/h),cv=getCanvas(Math.ceil(w*density),Math.ceil(h*density)),ctx=cv.getContext('2d')!;ctx.scale(density,density);ctx.translate(-x0,-y0);
  for(const g of records){ctx.save();ctx.translate(g.x,g.baseline);ctx.rotate(g.rotation*Math.PI/180);ctx.scale(g.scale,g.scale);ctx.globalAlpha=g.opacity;ctx.font=fontStr(g.style);ctx.fillStyle=g.style.color;ctx.fillText(g.text,0,0);ctx.restore();}
  return {cv,w,h,anchorX:-x0,anchorY:-y0,selection:{x0,y0,x1,y1,w,h}};
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
  const strokePad = Math.max(0, Number(d.stroke) || 0) / 2;
  const selection = {
    x0: -d.w / 2 - strokePad, y0: -d.h / 2 - strokePad,
    x1: d.w / 2 + strokePad, y1: d.h / 2 + strokePad,
    w: d.w + strokePad * 2, h: d.h + strokePad * 2,
  };
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
    return { cv, w, h: hh, anchorX: pad, anchorY: pad, selection };
  } else rr(c, 0, 0, W, H, d.radius || 0);
  c.fill();
  if (d.stroke > 0) c.stroke();
  return { cv, w, h: hh, anchorX: pad, anchorY: pad, selection };
}

/** Get (and cache) a rasterized bitmap for a layer. `scale` = render supersample. */
PM.raster = (L: any, scale: any = 1, time: any = PM.time) => {
  const d = L.type === 'text' ? resolvedTextContent(L, time) : resolveContent(PM, L, time);
  const controls=L.type==='text'?textControlValues(PM,L,time):null;
  const key = L.d.paths?.length ? 'paths|'+JSON.stringify(L.d.paths.map((path:any)=>({values:pathValues(PM,L,path,time),matrix:groupMatrix(PM,L,path,time)})))+'|'+scale : L.type === 'text'
    ? 't|' + [d.text, d.boxWidth, d.boxHeight, d.font, d.weight, d.size, d.tracking, d.leading, d.color, d.align, d.italic, variationSettings(d), JSON.stringify(controls), scale].join('|')
    : 's|' + [d.shape, d.color, d.w, d.h, d.radius, d.stroke, d.strokeColor, d.points, scale].join('|');
  let e = cache.get(key);
  if (!e) {
    e = L.d.paths?.length ? rasterPaths(PM,L,time,scale) : L.type === 'text' ? (L.d.animators?.length||L.d.styles?.length ? rasterAnimatedText(L,d,time,scale) : rasterText(d, scale)) : rasterShape(d, scale);
    e.dirty = true;
    e.used = ++tick;
    e.bytes = Math.max(0, Number(e.cv?.width || 0) * Number(e.cv?.height || 0) * 4);
    cache.set(key, e);
    cacheBytes += e.bytes;
    if (PM.Memory?.maintain) PM.Memory.maintain('raster', cache.size > MAX);
    else evict();
  }
  e.used = ++tick;
  e.key = key;
  if (L.type === 'text' && L.d.fontAnchorBounds && e.selection) {
    // Character animator movement must not be canceled by typography anchoring.
    const typeBounds = L.d.animators?.length
      ? PM.raster({ ...L, d: { ...L.d, animators: [], fontAnchorBounds: null } }, scale, time).selection
      : e.selection;
    const offset = fontAnchorOffset(L.d.fontAnchorBounds, typeBounds,
      PM.ev(L, 'anchor.x', time), PM.ev(L, 'anchor.y', time));
    // Placement belongs to the layer; cached glyph pixels remain shareable.
    return { ...e, anchorX: e.anchorX - offset.x, anchorY: e.anchorY - offset.y,
      fontOffset: offset, selection: { ...e.selection,
        x0: e.selection.x0 + offset.x, x1: e.selection.x1 + offset.x,
        y0: e.selection.y0 + offset.y, y1: e.selection.y1 + offset.y } };
  }
  return e;
};
PM.rasterStats = () => ({ size: cache.size, bytes: cacheBytes, maxBytes: MAX_BYTES });
PM.rasterClear = () => {
  for (const [key, entry] of [...cache]) release(key, entry);
  PM.GL && PM.GL.dropTextures && PM.GL.dropTextures('r:');
};
PM.Memory?.register?.('raster', {
  bytes: () => cacheBytes,
  entries: () => cache.size,
  trim: (target: number) => evict(target),
});

/* ── assets ────────────────────────────────────────────── */
function assetKind(file: any) {
  const mime = String(file.type || '').toLowerCase();
  if (mime.startsWith('image/')) return 'image';
  if (mime.startsWith('video/')) return 'video';
  if (PM.Audio && PM.Audio.accepts(file)) return 'audio';
  const ext = (String(file.name || '').split('.').pop() as any).toLowerCase();
  if (ext === 'obj' || mime === 'model/obj' || mime === 'text/plain+obj') return 'model';
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp'].includes(ext)) return 'image';
  if (['mp4', 'mov', 'm4v', 'webm'].includes(ext)) return 'video';
  return null;
}
PM.assetKind = assetKind;
function disposeAsset(a: any) {
  if (!a) return;
  if ((a.kind === 'audio' || a.audioBlob) && PM.Audio) PM.Audio.disposeAsset(a);
  try { if (a.el && a.el.pause) a.el.pause(); } catch (e) { }
  try { if (a.el && a.el.close) a.el.close(); } catch (e) { }
  if (a.url && String(a.url).startsWith('blob:')) window.URL.revokeObjectURL(a.url);
}
function waitForVideoMetadata(el: any, fileName: any, timeout: any = 15000) {
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
    const failed = () => finish(new Error(videoImportFailureMessage(fileName, el.error)));
    const timer = window.setTimeout(() => finish(new Error('Timed out reading video metadata')), timeout);
    el.addEventListener('loadedmetadata', loaded);
    el.addEventListener('durationchange', loaded);
    el.addEventListener('error', failed);
    loaded();
  });
}
async function playbackProxy(file: any, name: string): Promise<any> {
  const media = window.powermove?.media;
  if (!media?.createPlaybackProxy) {
    throw new Error(videoImportFailureMessage(name, { code: 4 }));
  }
  PM.toast(`Optimizing “${name}” for smooth playback…`, 30_000);
  const result = await media.createPlaybackProxy(file);
  if (!result.ok) throw new Error(result.error);
  try {
    const parts: ArrayBuffer[] = [];
    const chunkSize = 4 * 1024 * 1024;
    for (let offset = 0; offset < result.size; offset += chunkSize) {
      const chunk = await media.readPlaybackProxy(result.token, offset, Math.min(chunkSize, result.size - offset));
      if (!chunk.byteLength) throw new Error('The optimized playback file ended unexpectedly');
      const owned = new Uint8Array(chunk.byteLength);
      owned.set(chunk);
      parts.push(owned.buffer);
    }
    return new window.File(parts, name, {
      type: result.type,
      lastModified: Number(file.lastModified) || Date.now(),
    });
  } finally {
    await media.releasePlaybackProxy(result.token).catch(() => undefined);
  }
}
async function prepareAsset({ id, name, kind, blob, meta = {} }: any) {
  if (kind === 'audio') return PM.Audio.prepareAsset({ id, name, blob, meta });
  if (kind === 'model') {
    if (Number(blob?.size) > 64 * 1024 * 1024) throw new Error('OBJ files larger than 64 MB are not supported');
    const sourceText = await blob.text();
    const mesh = parseObj(sourceText);
    return {
      id, name, kind, format: 'obj', blob, sourceText, mesh,
      size: Number(blob.size) || Number(meta.size) || 0,
      vertices: mesh.sourceVertexCount,
      triangles: mesh.triangleCount,
    };
  }
  let sourceBlob = blob;
  let url = window.URL.createObjectURL(sourceBlob);
  let el: any, w = 0, hh = 0, dur = 0;
  let playbackProxyUsed = meta.playbackProxy === true;
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
      const openVideo = async () => {
        el = window.document.createElement('video');
        el.preload = 'metadata'; el.muted = true; el.playsInline = true;
        const ready = waitForVideoMetadata(el, name);
        el.src = url;
        try { el.load(); } catch (e) { }
        await ready;
        w = el.videoWidth || 0; hh = el.videoHeight || 0; dur = el.duration || 0;
      };
      await openVideo();
      /* A MOV can expose valid dimensions and advance its audio clock even when
         Chromium cannot decode a single video frame (notably Apple ProRes).
         Probe one presented frame, then stream a macOS-native H.264 proxy only
         when that proof fails. Known proxies skip the probe on project restore. */
      if (!playbackProxyUsed && /\.mov$/i.test(String(name || ''))
        && !await waitForPresentedVideoFrame(el)) {
        try { el.pause(); } catch (e) { }
        window.URL.revokeObjectURL(url);
        sourceBlob = await playbackProxy(blob, name);
        playbackProxyUsed = true;
        url = window.URL.createObjectURL(sourceBlob);
        await openVideo();
        if (!await waitForPresentedVideoFrame(el)) {
          throw new Error('The optimized video did not produce a playable frame');
        }
      }
    } else throw new Error('Unsupported media kind');
    const asset: any = {
      id, name, kind, url, el,
      w: w || meta.w || 0, h: hh || meta.h || 0,
      dur: dur || meta.dur || 0, size: sourceBlob.size || meta.size || 0,
      playbackProxy: playbackProxyUsed,
      persistBlob: sourceBlob,
    };
    /* A video's soundtrack stays attached to the video asset until the user
       separates it. Sharing the durable blob avoids storing the whole movie a
       second time. New imports decode once to prove an audio stream exists;
       restored videos use that saved result and decode lazily. */
    if (kind === 'video' && (meta.hasAudio === true || meta.hasAudio == null)) {
      try {
        const audio: any = await PM.Audio.prepareAsset({
          id, name, blob: sourceBlob,
          meta: meta.hasAudio === true ? { dur: meta.audioDur || meta.dur || dur } : {},
        });
        Object.assign(asset, {
          hasAudio: true,
          audioDur: audio.audioBuffer?.duration || audio.dur || dur || 0,
          audioBlob: audio.audioBlob,
          audioBuffer: audio.audioBuffer,
          audioDecoding: audio.audioDecoding,
          audioDisposed: audio.audioDisposed,
          audioToken: audio.audioToken,
          audioDecodeError: audio.audioDecodeError,
          audioRetryAt: audio.audioRetryAt,
          audioUsedAt: audio.audioUsedAt,
          peaks: audio.peaks,
          channels: audio.channels,
          sampleRate: audio.sampleRate,
        });
      } catch (error) {
        asset.hasAudio = false;
      }
    } else if (kind === 'video') asset.hasAudio = false;
    return asset;
  } catch (error) {
    try { if (el && el.close) el.close(); } catch (e) { }
    window.URL.revokeObjectURL(url);
    throw error;
  }
}
let assetEpoch = 0;
async function ingestAsset(file: any, { silent = false, layerDefinition }: any = {}) {
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
  const preparation = prepareAsset({ id: provisionalId, name: file.name, kind, blob: file });
  /* Images/audio can persist while they decode. Video preparation may replace
     an unsupported source with a much smaller playback proxy, so wait for that
     decision before writing any video bytes to durable storage. */
  const mayNeedPlaybackProxy = kind === 'video' && /\.mov$/i.test(String(file.name || ''));
  const eagerPersist = mayNeedPlaybackProxy ? null
    : PM.MediaStore.put(storageKey, file, { storageKey, fingerprint, type: file.type });
  const preparedResult: any = await Promise.resolve(preparation).then(
    value => ({ status: 'fulfilled', value }),
    reason => ({ status: 'rejected', reason })
  );
  const prepared = preparedResult.status === 'fulfilled' ? preparedResult.value : null;
  const persistBlob = prepared?.persistBlob || file;
  if (prepared) delete prepared.persistBlob;
  const persistedResult: any = preparedResult.status === 'fulfilled'
    ? await Promise.resolve(eagerPersist || PM.MediaStore.put(storageKey, persistBlob, {
      storageKey, fingerprint, type: persistBlob.type || file.type,
    })).then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason }))
    : eagerPersist
      ? await Promise.resolve(eagerPersist).then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason }))
      : { status: 'fulfilled', value: false };
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
    playbackProxy: prepared.playbackProxy === true,
    ...(kind === 'video' ? {
      hasAudio: prepared.hasAudio === true,
      audioDur: prepared.hasAudio ? prepared.audioDur || prepared.dur || 0 : 0,
    } : {}),
    ...(kind === 'model' ? {
      format: prepared.format || 'obj',
      vertices: prepared.vertices || 0,
      triangles: prepared.triangles || 0,
      layerDefinition: typeof layerDefinition === 'string' ? layerDefinition : undefined,
    } : {}),
  };
  const plan = PM.MediaImport.match(PM.proj, PM.assets.map, identity);
  const existingMeta = plan.canonicalId && PM.proj.assets[plan.canonicalId];
  const existingLive = plan.canonicalId && PM.assets.map.get(plan.canonicalId);
  if (existingLive?.playbackProxy) identity.playbackProxy = true;
  const wasMissing = !!(existingMeta && !existingLive);
  const id = plan.canonicalId || provisionalId;
  let asset = prepared;
  const upgradesPlayback = !!(existingLive && prepared.playbackProxy && !existingLive.playbackProxy);
  if (existingLive && !upgradesPlayback) {
    disposeAsset(prepared);
    asset = existingLive;
  } else {
    if (existingLive) disposeAsset(existingLive);
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
    status: relinked || upgradesPlayback ? 'relinked' : existingLive ? 'reused' : 'created',
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
      .filter((meta: any) => meta && meta.id && ['image', 'video', 'audio', 'model'].includes(meta.kind));
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
