import { createCloudMedia, cloudSourcePaths, readLocalMediaSource } from '../core/cloud-media';
import { prepareVideoPreview } from '../core/video-preview';
import { disposeVideoInstances } from '../core/video-instances';
import { cancelPreviewVideoSeek } from '../core/video-seek';
import { importedSequences } from '../core/image-sequence';
import { convertAnimatedImage, convertStillImage, isAnimatedImage, mayAnimate, readProxyFile } from '../core/media-conversion';
import {
  isImageExtension, isVideoExtension, mayNeedVideoProxy, mediaExtension, needsImageConversion, needsVideoProxy,
} from '../../../../shared/media-formats';
import { fontAnchorOffset } from '../core/font-anchor';
import { animatedGlyphs, textControlValues } from '../core/text-animation';
import { rasterPaths, pathValues, groupMatrix } from '../core/vector-paths';
import { createVariableFontRenderer, variationEntries, variationSettings } from '../../typography/font-renderer';
export { variationEntries as textVariationEntries, variationSettings as formatFontVariationSettings } from '../../typography/font-renderer';
import { resolveContent } from '../core/content-properties';
import { shapeRasterGeometry, type RasterWindow } from './shape-raster-window';
/* Ported from js/gl/raster.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { parseObj } from '../../kernel/obj';
import { parseSvg } from '../core/svg-import';
import { inspectorService, viewerService } from '../core/services';
import { capturePoster } from '../core/poster';

const posterUrls = new Map<string, string>();

function revokePoster(id: any) {
  const previous = posterUrls.get(String(id));
  posterUrls.delete(String(id));
  if (!previous) return;
  try { window.URL?.revokeObjectURL?.(previous); } catch (error) { }
}

function setPoster(id: any, blob: Blob) {
  revokePoster(id);
  try {
    if (typeof window.URL?.createObjectURL !== 'function') return;
    posterUrls.set(String(id), window.URL.createObjectURL(blob));
  } catch (error) { }
}

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
// Small icons and waveform bars should be limited by bytes, not discarded
// every frame merely because a composition has more than 96 unique sources.
const MAX = 4096;
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
  // Uploaded textures own their storage and are evicted by the GPU budget.
  // Dropping one here turns CPU cache pressure into repeated GPU uploads.
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
const variationFontFamily = createVariableFontRenderer(() => { PM.rasterClear?.(); PM.invalidate?.(); }, PM.fontSource);
PM.rasterDispose = () => { variationFontFamily.dispose(); PM.rasterClear?.(); };

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
interface SourceLine { text: string; start: number }

/** Wrapped display lines with the source offset each one starts at. Caret
    placement maps a source index to the line whose range contains it, so
    every branch below records where its line begins in the source string. */
function textSourceLines(d: any, context: any, lineHeight: number): SourceLine[] {
  const text = String(d.text == null ? '' : d.text);
  const paragraphs = text.split('\n');
  const boxWidth = Number(d.boxWidth);
  const lines: SourceLine[] = [];
  let offset = 0;
  if (!d.paragraph || !Number.isFinite(boxWidth) || boxWidth <= 0) {
    for (const paragraph of paragraphs) { lines.push({ text: paragraph, start: offset }); offset += paragraph.length + 1; }
    return lines;
  }
  const width = (value: string) => context.measureText(value).width;
  for (const paragraph of paragraphs) {
    const paragraphStart = offset;
    offset += paragraph.length + 1;
    if (!paragraph) { lines.push({ text: '', start: paragraphStart }); continue; }
    let line = '', lineStart = paragraphStart, tokenStart = paragraphStart;
    for (const token of paragraph.split(/(\s+)/u).filter(Boolean)) {
      const candidate = line + token;
      if (line && width(candidate) > boxWidth) {
        lines.push({ text: line.trimEnd(), start: lineStart });
        line = token.trimStart();
        lineStart = tokenStart + (token.length - line.length);
      } else {
        if (!line) lineStart = tokenStart;
        line = candidate;
      }
      tokenStart += token.length;
      while (line && width(line) > boxWidth) {
        let cut = 1;
        while (cut < line.length && width(line.slice(0, cut + 1)) <= boxWidth) cut++;
        lines.push({ text: line.slice(0, cut), start: lineStart });
        line = line.slice(cut);
        lineStart += cut;
      }
    }
    lines.push({ text: line.trimEnd(), start: lineStart });
  }
  const boxHeight = Number(d.boxHeight);
  if (!Number.isFinite(boxHeight) || boxHeight <= 0) return lines;
  return lines.slice(0, Math.max(1, Math.floor(boxHeight / Math.max(1, lineHeight))));
}

function textLines(d: any, context: any, lineHeight: number): string[] {
  return textSourceLines(d, context, lineHeight).map(line => line.text);
}

function graphemesOf(value: string): string[] {
  if (typeof Intl !== 'undefined' && Intl.Segmenter) {
    return [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(value)].map(item => item.segment);
  }
  return Array.from(value);
}

interface CaretLine { text: string; start: number; x: number; y: number; baseline: number; width: number; boundaries: { index: number; x: number }[] }
interface CaretLayout { lines: CaretLine[]; lineHeight: number; size: number; length: number; align: 'left' | 'center' | 'right'; boxWidth: number; boxHeight: number }

/* Caret and selection geometry for the on-canvas text editor. Every x comes
   from the same measuring context and wrapping as the rasterizer, so the
   caret sits between the painted glyphs at any zoom. Coordinates are in the
   layer's local space: the first baseline is at size * .82 below the origin
   and x is measured from the alignment origin. */
function textCaretLayout(d: any): CaretLayout {
  const size = Math.max(1, Number(d.size) || 16);
  const meas = getCanvas(8, 8).getContext('2d') as any;
  const align = d.align === 'center' ? 'center' : d.align === 'right' ? 'right' : 'left';
  meas.font = fontStr(d);
  meas.textAlign = 'left';
  meas.textBaseline = 'alphabetic';
  if ('letterSpacing' in meas) meas.letterSpacing = (d.tracking || 0) + 'px';
  const lh = size * (d.leading || 1.15);
  const width = (value: string) => meas.measureText(value).width;
  const text = String(d.text == null ? '' : d.text);
  const lines = textSourceLines(d, meas, lh).map((line, row): CaretLine => {
    const lineWidth = width(line.text);
    const startX = align === 'center' ? -lineWidth / 2 : align === 'right' ? -lineWidth : 0;
    const boundaries = [{ index: line.start, x: startX }];
    let prefix = '';
    for (const segment of graphemesOf(line.text)) {
      prefix += segment;
      boundaries.push({ index: line.start + prefix.length, x: startX + width(prefix) });
    }
    return { text: line.text, start: line.start, x: startX, y: row * lh, baseline: row * lh + size * .82, width: lineWidth, boundaries };
  });
  return { lines, lineHeight: lh, size, length: text.length, align, boxWidth: Number(d.boxWidth) || 0, boxHeight: Number(d.boxHeight) || 0 };
}
PM.textCaretLayout = (input: any, time = PM.time) => textCaretLayout(resolvedTextContent(input, time));

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
    const lineSource=Math.max(sourceOffset,String(d.text||'').indexOf(line,sourceOffset));
    if (line.length) output.lines.push({ text: line, x: startX, y, line: row, sourceStart: lineSource, index: lineIndex++ });

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
      output.words.push({ text: match[0], x: segmentX(startX, prefix, match[0]), y, line: row, sourceStart: lineSource + match.index, index: wordIndex++ });
    }
  });
  return output;
}
PM.textLayout = (input: any, time = PM.time) => textLayout(resolvedTextContent(input, time));

// Font measurements are independent of zoom. Keep this small metadata cache
// separate from bitmap eviction so panning does not repeatedly measure text.
const textGeometryCache = new Map<string, any>();
function textGeometry(d: any) {
  const key = JSON.stringify([d.text, d.boxWidth, d.boxHeight, d.paragraph, d.font, d.weight, d.size, d.tracking, d.leading, d.align, d.italic, variationSettings(d)]);
  const cached = textGeometryCache.get(key);
  if (cached) { textGeometryCache.delete(key); textGeometryCache.set(key, cached); return cached; }
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
  const x = d.align === 'center' ? w / 2 : d.align === 'right' ? w - pad : pad;
  const anchorX = d.align === 'center' ? w / 2 : d.align === 'right' ? w - pad : pad;
  const anchorY = pad;
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  lines.forEach((line, i) => {
    const baseline = pad + lh * i + size * .82;
    const measured = metrics[i];

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
    selection.y1 = pad + (Number(d.boxHeight) > 0 ? Math.max(paragraphHeight, lh) : lh * lines.length) - anchorY + interactionPad;
  }
  selection.w = selection.x1 - selection.x0;
  selection.h = selection.y1 - selection.y0;
  const geometry = { w, h: hh, anchorX, anchorY, selection, lines, lh, size, pad, x, align };
  textGeometryCache.set(key, geometry);
  if (textGeometryCache.size > 512) textGeometryCache.delete(textGeometryCache.keys().next().value!);
  return geometry;
}

function textRasterGeometry(d: any, scale: number) {
  const geometry = textGeometry(d);
  const density = Math.min(scale, 8192 / geometry.w, 8192 / geometry.h);
  return { ...geometry, density, width: Math.max(1, Math.ceil(geometry.w * density)), height: Math.max(1, Math.ceil(geometry.h * density)) };
}
// The compositor uses this only for ordinary text. Animated glyphs and font
// anchoring continue through their existing complete-source rendering path.
PM.textRasterGeometry = (layer: any, scale: number, time = PM.time) => textRasterGeometry(resolvedTextContent(layer, time), scale);

/* A text raster can come out empty although the layer has visible text: a
   font that is still loading paints nothing on a 2D canvas, and a canvas the
   browser refused to allocate reads back as transparent. Cached as-is, that
   blank bitmap would stay on screen until the app restarts. Sample a small
   downscale of the result so such a raster can be retried instead. */
const BLANK_RETRY_MS = 250;
const warnedBlank = new Set<string>();
function rasterLooksBlank(cv: HTMLCanvasElement): boolean {
  try {
    const probe = getCanvas(24, 24), ctx = probe.getContext('2d') as any;
    if (!ctx?.drawImage || !ctx.getImageData) return false;
    ctx.drawImage(cv, 0, 0, 24, 24);
    const data = ctx.getImageData(0, 0, 24, 24)?.data;
    if (!data) return false;
    for (let i = 3; i < data.length; i += 4) if (data[i]! > 0) return false;
    return true;
  } catch { return false; }
}

function rasterText(d: any, scale: number) {
  const g = textRasterGeometry(d, scale);
  const cv = getCanvas(g.width, g.height);
  const c = cv.getContext('2d') as any;
  c.scale(g.density, g.density);
  c.font = fontStr(d);
  if ('letterSpacing' in c) c.letterSpacing = (d.tracking || 0) + 'px';
  c.textBaseline = 'alphabetic'; c.textAlign = g.align; c.fillStyle = d.color || '#fff';
  g.lines.forEach((line: string, i: number) => c.fillText(line, g.x, g.pad + g.lh * i + g.size * .82));
  const visible = g.lines.some((line: string) => line.trim().length) && !/^(transparent|rgba?\(.*,\s*0\s*\))$/i.test(String(d.color || ''));
  const blank = visible && rasterLooksBlank(cv);
  if (blank && !warnedBlank.has(c.font)) {
    warnedBlank.add(c.font);
    console.warn('[raster] text painted nothing; retrying shortly', { font: c.font, size: g.width + 'x' + g.height, text: String(d.text).slice(0, 40) });
  }
  return { cv, w: g.w, h: g.h, anchorX: g.anchorX, anchorY: g.anchorY, selection: g.selection, blank };
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

function rasterShape(d: any, scale: any, crop?: RasterWindow) {
  const { pad, w, h: hh, density, width, height, selection } = shapeRasterGeometry(d, scale);
  const uv = crop ? [-crop.x / crop.width, -crop.y / crop.height, width / crop.width, height / crop.height] : undefined;
  const cv = getCanvas(crop?.width ?? width, crop?.height ?? height);
  const c = cv.getContext('2d') as any;
  if (crop) c.translate(-crop.x, -crop.y);
  c.scale(density, density);
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
    return { cv, w, h: hh, anchorX: pad, anchorY: pad, selection, uv };
  } else rr(c, 0, 0, W, H, d.radius || 0);
  c.fill();
  if (d.stroke > 0) c.stroke();
  return { cv, w, h: hh, anchorX: pad, anchorY: pad, selection, uv };
}

/** Get (and cache) a rasterized bitmap for a layer. `scale` = render supersample. */
PM.raster = (L: any, scale: any = 1, time: any = PM.time, uploaded?: (key: string) => any, crop?: RasterWindow) => {
  const d = L.type === 'text' ? resolvedTextContent(L, time) : resolveContent(PM, L, time);
  const controls=L.type==='text'?textControlValues(PM,L,time):null;
  // Above the bitmap dimension limit, different requested zoom densities
  // produce identical pixels. Key those sources by their effective density
  // so pinch gestures do not rebuild/upload the same capped bitmap repeatedly.
  const rasterScale = L.type === 'shape' && !L.d.paths?.length ? shapeRasterGeometry(d, scale).density
    : L.type === 'text' && !L.d.animators?.length && !L.d.styles?.length ? textRasterGeometry(d, scale).density
    : scale;
  const key = (L.d.paths?.length ? 'paths|'+JSON.stringify(L.d.paths.map((path:any)=>({values:pathValues(PM,L,path,time),matrix:groupMatrix(PM,L,path,time)})))+'|'+scale : L.type === 'text'
    ? 't|' + [d.text, d.boxWidth, d.boxHeight, d.font, d.weight, d.size, d.tracking, d.leading, d.color, d.align, d.italic, variationSettings(d), JSON.stringify(controls), rasterScale].join('|')
    : 's|' + [d.shape, d.color, d.w, d.h, d.radius, d.stroke, d.strokeColor, d.points, rasterScale].join('|'))
    + (crop ? `|crop:${crop.x},${crop.y},${crop.width},${crop.height}` : '');
  // Typography anchoring may need a separate, unanimated CPU measurement.
  // Keep that path intact; otherwise the renderer can reuse uploaded pixels
  // and their geometry without retaining a second canvas in memory.
  let e = cache.get(key);
  if (e?.blank) {
    // Give a loading font a moment, then paint again instead of keeping the blank bitmap.
    if (Date.now() >= e.retryAt) { release(key, e); e = null; }
  }
  if (!e && !L.d.fontAnchorBounds) { const stub = uploaded?.(key); if (stub && !stub.blank) e = stub; }
  if (!e) {
    e = L.d.paths?.length ? rasterPaths(PM,L,time,scale) : L.type === 'text' ? (L.d.animators?.length||L.d.styles?.length ? rasterAnimatedText(L,d,time,scale) : rasterText(d, scale)) : rasterShape(d, scale, crop);
    e.dirty = true;
    e.used = ++tick;
    e.bytes = Math.max(0, Number(e.cv?.width || 0) * Number(e.cv?.height || 0) * 4);
    if (e.blank) {
      e.retryAt = Date.now() + BLANK_RETRY_MS;
      window.setTimeout(() => PM.invalidate?.('render'), BLANK_RETRY_MS + 16);
      (window.document as any)?.fonts?.ready?.then?.(() => PM.invalidate?.('render'));
    }
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
  textGeometryCache.clear();
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
  const ext = mediaExtension(file.name);
  if (ext === 'obj' || mime === 'model/obj' || mime === 'text/plain+obj') return 'model';
  if (isImageExtension(ext)) return 'image';
  if (isVideoExtension(ext)) return 'video';
  return null;
}
PM.assetKind = assetKind;
/* An animated GIF, APNG or WebP is a still to `assetKind` and a clip to the
   editor. Only the decoder can tell the two apart, so import and replacement
   both settle the question before anything else looks at the file. */
async function resolveAssetKind(file: any) {
  const kind = assetKind(file);
  if (kind !== 'image') return { kind, animated: false };
  const animated = mayAnimate(file) && await isAnimatedImage(file);
  return { kind: animated ? 'video' : 'image', animated };
}
PM.resolveAssetKind = resolveAssetKind;
const disposedAssets = new WeakSet<object>();
const historyAssetRetains = new WeakMap<object, number>();
function disposeAsset(a: any) {
  if (!a) return;
  if (typeof a === 'object') {
    if (disposedAssets.has(a)) return;
    disposedAssets.add(a);
  }
  disposeVideoInstances(a);
  if (a.preview) {
    cancelPreviewVideoSeek(a.preview.el);
    a.preview.el.pause(); a.preview.el.removeAttribute('src'); a.preview.el.load();
    window.URL.revokeObjectURL(a.preview.url); delete a.preview;
  }
  if ((a.kind === 'audio' || a.audioBlob) && PM.Audio) PM.Audio.disposeAsset(a);
  try { if (a.el && a.el.pause) a.el.pause(); } catch (e) { }
  if (a.kind === 'video' && a.el) {
    cancelPreviewVideoSeek(a.el);
    // Revoking a Blob URL alone leaves the element's decoder and decoded
    // frames alive. Only unload after the final live/history owner is gone.
    try { a.el.removeAttribute('src'); a.el.load(); } catch (e) { }
  }
  try { if (a.el && a.el.close) a.el.close(); } catch (e) { }
  if (a.url && String(a.url).startsWith('blob:')) window.URL.revokeObjectURL(a.url);
}
function retainHistoryAsset(asset: any) {
  if (!asset || typeof asset !== 'object') return;
  historyAssetRetains.set(asset, (historyAssetRetains.get(asset) || 0) + 1);
}
function releaseHistoryAsset(asset: any, id: any) {
  if (!asset || typeof asset !== 'object') return;
  const next = Math.max(0, (historyAssetRetains.get(asset) || 0) - 1);
  if (next) historyAssetRetains.set(asset, next);
  else {
    historyAssetRetains.delete(asset);
    if (PM.assets.map.get(id) !== asset) disposeAsset(asset);
  }
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
async function playbackProxy(file: any, name: string, onStage?: (label: string) => void): Promise<any> {
  const media = window.powermove?.media;
  if (!media?.createPlaybackProxy) {
    throw new Error(videoImportFailureMessage(name, { code: 4 }));
  }
  if (onStage) onStage('Optimizing video for playback');
  else PM.toast(`Optimizing “${name}” for smooth playback…`, 30_000, { error: false });
  const result = await media.createPlaybackProxy(file, cloudSourcePaths.get(file));
  if (!result.ok) throw new Error(result.error);
  onStage?.('Loading optimized video');
  return readProxyFile(result, name, file.lastModified);
}
async function prepareAsset({ id, name, kind, blob, meta = {}, onStage }: any) {
  const imageSequence = importedSequences.get(blob) || meta.imageSequence;
  if (kind === 'audio') return PM.Audio.prepareAsset({ id, name, blob, meta });
  if (kind === 'model') {
    const sourceText = await blob.text();
    const mesh = parseObj(sourceText);
    return {
      id, name, kind, format: 'obj', blob, sourceText, mesh,
      size: Number(blob.size) || Number(meta.size) || 0,
      vertices: mesh.sourceVertexCount,
      triangles: mesh.triangleCount,
    };
  }
  const imageFormat = kind === 'image' && (meta.format === 'svg'
    || String(blob?.type || '').toLowerCase() === 'image/svg+xml'
    || /\.svg$/i.test(String(name || ''))) ? 'svg' : meta.format;
  /* macOS and drag/drop providers occasionally omit an SVG File's MIME type.
     Give Chromium the type it needs for decode without altering the bytes. */
  let sourceBlob = imageFormat === 'svg' && String(blob?.type || '').toLowerCase() !== 'image/svg+xml'
    ? new window.Blob([blob], { type: 'image/svg+xml' })
    : blob;
  const svg = imageFormat === 'svg' ? parseSvg(await sourceBlob.text()) : null;
  let url = window.URL.createObjectURL(sourceBlob);
  let el: any, w = 0, hh = 0, dur = 0;
  let playbackProxyUsed = meta.playbackProxy === true;
  let playbackProxyVersion = Number(meta.playbackProxyVersion) || 0;
  try {
    if (kind === 'image') {
      /* Keep an SVG as a live vector image. createImageBitmap bakes it at its
         intrinsic dimensions, which makes an otherwise vector layer soften as
         soon as the user scales it in the composition. */
      if (imageFormat !== 'svg' && window.createImageBitmap) {
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
      const extension = mediaExtension(name);
      const canProxy = !playbackProxyUsed && mayNeedVideoProxy(extension);
      /* Chromium opens none of MKV, AVI, MPEG-TS and friends, so those skip
         straight to conversion instead of waiting out a decode failure. */
      let needsProxy = canProxy && needsVideoProxy(extension);
      if (!needsProxy) {
        try { await openVideo(); }
        catch (error) {
          if (!canProxy) throw error;
          needsProxy = true;
        }
      }
      /* A MOV can expose valid dimensions and advance its audio clock even when
         Chromium cannot decode a single video frame (notably Apple ProRes).
         Probe one presented frame, then stream an alpha-preserving VP9 proxy
         if metadata or frame decoding fails. Known proxies skip the probe on restore. */
      if (canProxy && (needsProxy || !await waitForPresentedVideoFrame(el))) {
        if (el) {
          try { el.pause(); } catch (e) { }
          el.removeAttribute('src');
          el.load();
        }
        window.URL.revokeObjectURL(url);
        sourceBlob = await playbackProxy(blob, name, onStage);
        playbackProxyUsed = true;
        playbackProxyVersion = 3;
        url = window.URL.createObjectURL(sourceBlob);
        await openVideo();
        if (!await waitForPresentedVideoFrame(el)) {
          throw new Error('The optimized video did not produce a playable frame');
        }
      }
    } else throw new Error('Unsupported media kind');
    const asset: any = {
      id, name, kind, url, el, storageKey: meta.storageKey,
      w: w || meta.w || 0, h: hh || meta.h || 0,
      dur: dur || meta.dur || 0, size: sourceBlob.size || meta.size || 0,
      playbackProxy: playbackProxyUsed,
      playbackProxyVersion,
      persistBlob: sourceBlob,
      ...(imageSequence ? { imageSequence } : {}),
      ...(imageFormat ? { format: imageFormat } : {}),
      ...(svg ? { svg } : {}),
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
    if (kind === 'video') {
      asset.previewReady = prepareVideoPreview(PM, asset, sourceBlob, () => disposedAssets.has(asset));
      if (asset.playbackProxy && asset.playbackProxyVersion < 3) await asset.previewReady;
    }
    return asset;
  } catch (error) {
    try { if (el && el.close) el.close(); } catch (e) { }
    window.URL.revokeObjectURL(url);
    throw error;
  }
}

function assetIdentity(id: any, file: any, kind: any, prepared: any, fingerprint: any, storageKey: any, sourcePath: any, persisted: any, layerDefinition?: any) {
  return {
    id, name: file.name, kind, fingerprint, storageKey,
    ...(sourcePath ? { sourcePath } : {}),
    size: prepared.size, dur: prepared.dur, w: prepared.w, h: prepared.h,
    channels: prepared.channels || 0, sampleRate: prepared.sampleRate || 0,
    playbackProxy: prepared.playbackProxy === true,
    playbackProxyVersion: Number(prepared.playbackProxyVersion) || 0,
    persisted: persisted === true,
    ...(prepared.imageSequence ? { imageSequence: prepared.imageSequence } : {}),
    ...(prepared.poster === true ? { poster: true } : {}),
    ...(prepared.format ? { format: prepared.format } : {}),
    ...(prepared.svg ? { editablePaths: prepared.svg.paths.length } : {}),
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
}
let assetEpoch = 0;
function checkpointAssetMetadata(project: any) {
  if (!project || PM.proj !== project) return;
  /* Asset metadata is mutated outside typed history, so an existing recovery
     journal cannot replay it. Force the smallest full project checkpoint. */
  try { PM.Projects?.put?.(project); } catch (error) { }
}
/** Import and replacement share all reading, decoding, conversion, and storage. */
async function prepareImportedAsset(file: any, { id, assertCurrentProject, resolved: settled, onStage }: any) {
  const resolved = settled || await resolveAssetKind(file);
  const kind = resolved.kind;
  if (!kind) throw new Error('Unsupported media file');
  const sourcePath = cloudSourcePaths.get(file) || window.powermove?.media?.sourcePath?.(file) || '';
  onStage?.('Reading file');
  /* Fingerprint the file the user chose, not the conversion, so re-importing
     the same GIF still resolves to the media already in the project. */
  const fingerprint = await PM.MediaImport.fingerprint(file);
  assertCurrentProject();
  const storageKey = PM.MediaImport.storageKeyFor(fingerprint);
  /* Formats Chromium cannot show become ones it can before anything decodes,
     stores, or measures them. */
  const extension = mediaExtension(file.name);
  let source = file;
  let meta: any = {};
  if (resolved.animated) {
    const animation = await convertAnimatedImage(file, { onStage });
    assertCurrentProject();
    source = animation.file;
    meta = { format: extension, imageSequence: { fps: animation.fps, frames: animation.frames } };
  } else if (kind === 'image' && needsImageConversion(extension)) {
    source = await convertStillImage(file, { onStage });
    assertCurrentProject();
    meta = { format: extension };
  }
  onStage?.('Preparing media');
  const preparation = prepareAsset({ id, name: file.name, kind, blob: source, meta: { ...meta, storageKey }, onStage });
  /* Images/audio can persist while they decode. Video preparation may replace
     an unsupported source with a much smaller playback proxy, so wait for that
     decision before writing any video bytes to durable storage. */
  const mayNeedPlaybackProxy = kind === 'video' && !resolved.animated && mayNeedVideoProxy(extension);
  // Observe storage failures immediately, even while decoding is still pending.
  const persist = (blob: any) => Promise.resolve().then(() => PM.MediaStore.put(storageKey, blob, {
    storageKey, fingerprint, type: blob.type || file.type,
  })).then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason }));
  const eagerPersist = mayNeedPlaybackProxy ? null : persist(source);
  const preparedResult: any = await Promise.resolve(preparation).then(
    value => ({ status: 'fulfilled', value }),
    reason => ({ status: 'rejected', reason })
  );
  const prepared = preparedResult.status === 'fulfilled' ? preparedResult.value : null;
  const persistBlob = prepared?.persistBlob || source;
  if (prepared) delete prepared.persistBlob;
  if (prepared) onStage?.('Saving media');
  const persistedResult: any = await (eagerPersist || (prepared
    ? persist(persistBlob) : { status: 'fulfilled', value: false }));
  if (preparedResult.status === 'rejected' || persistedResult.status === 'rejected') {
    if (prepared) disposeAsset(prepared);
    throw preparedResult.status === 'rejected' ? preparedResult.reason : persistedResult.reason;
  }
  try { assertCurrentProject(); }
  catch (error) { disposeAsset(prepared); throw error; }
  const persisted = persistedResult.value;
  return { prepared, kind, fingerprint, storageKey, sourcePath, persisted };
}
async function ingestAsset(file: any, { silent = false, layerDefinition, onStage }: any = {}) {
  const targetProject = PM.proj;
  const targetEpoch = assetEpoch;
  const assertCurrentProject = () => {
    if (PM.proj !== targetProject || assetEpoch !== targetEpoch) {
      const error = new Error('Import stopped because you switched projects · import the file again in the intended project');
      (error as any).code = 'STALE_MEDIA_IMPORT';
      throw error;
    }
  };
  const provisionalId = PM.uid('a');
  const { prepared, kind, fingerprint, storageKey, sourcePath, persisted } = await prepareImportedAsset(file, {
    id: provisionalId, assertCurrentProject, onStage,
  });
  let posterBlob: Blob | null = null;
  try {
    posterBlob = await capturePoster(prepared);
    if (posterBlob) {
      const posterKey = PM.MediaImport.posterKeyFor(storageKey);
      const stored = await PM.MediaStore.put(posterKey, posterBlob, { storageKey: posterKey, type: posterBlob.type });
      if (stored) prepared.poster = true;
      else posterBlob = null;
    }
  } catch (error) { posterBlob = null; }
  try { assertCurrentProject(); }
  catch (error) { disposeAsset(prepared); throw error; }
  const { id: _identityId, persisted: _identityPersisted, ...identity } = assetIdentity(
    provisionalId, file, kind, prepared, fingerprint, storageKey, sourcePath, persisted, layerDefinition,
  );
  const plan = PM.MediaImport.match(PM.proj, PM.assets.map, identity);
  const existingMeta = plan.canonicalId && PM.proj.assets[plan.canonicalId];
  const existingLive = plan.canonicalId && PM.assets.map.get(plan.canonicalId);
  const wasMissing = !!(existingMeta && !existingLive);
  const id = plan.canonicalId || provisionalId;
  if (!identity.poster && existingMeta?.poster) identity.poster = true;
  let asset = prepared;
  const upgradesPlayback = !!(existingLive && prepared.playbackProxy
    && (!existingLive.playbackProxy || prepared.playbackProxyVersion > (existingLive.playbackProxyVersion || 0)));
  if (existingLive && !upgradesPlayback) {
    if (existingLive.playbackProxy) {
      identity.playbackProxy = true;
      identity.playbackProxyVersion = Number(existingLive.playbackProxyVersion) || 0;
    }
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
  if (posterBlob) setPoster(id, posterBlob);
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
    checkpointAssetMetadata(PM.proj);
    PM.bus.emit('assets');
    if (relinkedLayers) PM.bus.emit('layers');
  }
  return result;
}
async function recoverSourceAsset(meta: any, file: File, current: () => boolean) {
  const assertCurrentProject = () => { if (!current()) throw new Error('The project changed'); };
  if (meta.fingerprint && await PM.MediaImport.fingerprint(file) !== meta.fingerprint) {
    throw new Error('The source file has changed. Use Locate File to choose a replacement.');
  }
  assertCurrentProject();
  const imported = await prepareImportedAsset(file, { id: meta.id, assertCurrentProject });
  if (!current()) { disposeAsset(imported.prepared); return; }
  Object.assign(meta, assetIdentity(meta.id, file, imported.kind, imported.prepared,
    imported.fingerprint, imported.storageKey, imported.sourcePath, imported.persisted, meta.layerDefinition));
  Object.assign(imported.prepared, meta);
  PM.assets.map.set(meta.id, imported.prepared);
  PM.assets.errors.delete(meta.id);
  PM.touch();
  checkpointAssetMetadata(PM.proj);
  PM.Audio?.rebalanceCache?.();
}
const cloudMedia = createCloudMedia(PM, recoverSourceAsset);
PM.assets = {
  cloud: cloudMedia,
  map: new Map<any, any>(),
  loading: new Set<string>(),
  errors: new Map<string, string>(),
  async add(file: any, options: any) {
    const result = await ingestAsset(file, options);
    result.asset.importResult = result;
    return result.asset;
  },
  async importBatch(files: any, { concurrency, onProgress, onStage, replaceAssetId }: any = {}) {
    const list = Array.from(files || []);
    if (replaceAssetId != null && list.length !== 1) throw new Error('Choose one file or one image sequence to replace this media');
    const cores = Math.max(1, Number(window.navigator && window.navigator.hardwareConcurrency) || 4);
    const limit = concurrency == null ? Math.max(1, Math.min(3, Math.floor(cores / 2))) : concurrency;
    let completed = 0;
    const results = await PM.MediaImport.mapBounded(list, limit, async (file: any, index: any) => {
      let result: any;
      const options = { silent: true, onStage: onStage
        ? (label: string) => onStage({ file, index, label, completed, total: list.length }) : undefined };
      try {
        result = replaceAssetId != null
          ? { ...await PM.assets.replace(replaceAssetId, file, options), status: 'replaced' }
          : await ingestAsset(file, options);
      }
      catch (error) { result = { file, status: 'failed', error }; }
      completed++;
      if (onProgress) onProgress({ completed, total: list.length, index, file, result });
      return result;
    });
    if (results.some((result: any) => result.status !== 'failed' && result.status !== 'replaced')) {
      PM.touch();
      checkpointAssetMetadata(PM.proj);
      PM.bus.emit('assets');
      if (results.some((result: any) => result.relinkedLayers)) PM.bus.emit('layers');
    }
    return results;
  },
  async replace(id: any, file: any, { onStage }: any = {}) {
    const targetProject = PM.proj;
    const targetProjectId = targetProject?.id;
    const targetEpoch = assetEpoch;
    const currentMeta: any = targetProject?.assets?.[id];
    if (!currentMeta) throw new Error('This media item is no longer in the project');
    const resolved = await resolveAssetKind(file);
    const kind = resolved.kind;
    if (!kind) throw new Error('Unsupported media file');
    if (kind !== currentMeta.kind) {
      const article = currentMeta.kind === 'image' || currentMeta.kind === 'audio' ? 'an' : 'a';
      throw new Error(`Choose ${article} ${currentMeta.kind} file to replace this ${currentMeta.kind} media`);
    }
    const assertCurrentProject = () => {
      if (PM.proj !== targetProject || assetEpoch !== targetEpoch || PM.proj?.assets?.[id] !== currentMeta) {
        const error = new Error('Replacement stopped because you switched projects · replace the file again in the intended project');
        (error as any).code = 'STALE_MEDIA_REPLACEMENT';
        throw error;
      }
    };
    let prepared: any = null;
    try {
      const imported = await prepareImportedAsset(file, { id, assertCurrentProject, resolved, onStage });
      prepared = imported.prepared;
      const { fingerprint, storageKey, sourcePath, persisted } = imported;
      if (!persisted) throw new Error('Could not store the replacement media · the original file is unchanged');
      let nextPoster: Blob | null = null;
      try {
        nextPoster = await capturePoster(prepared);
        if (nextPoster) {
          const posterKey = PM.MediaImport.posterKeyFor(storageKey);
          const stored = await PM.MediaStore.put(posterKey, nextPoster, { storageKey: posterKey, type: nextPoster.type });
          if (stored) prepared.poster = true;
          else nextPoster = null;
        }
      } catch (error) { nextPoster = null; }
      assertCurrentProject();

      const previousRuntime = PM.assets.map.get(id) || null;
      const previousMeta = JSON.parse(JSON.stringify(currentMeta));
      let previousPoster: Blob | null = null;
      if (currentMeta.poster && currentMeta.storageKey && typeof PM.MediaStore.get === 'function') {
        try { previousPoster = await PM.MediaStore.get(PM.MediaImport.posterKeyFor(currentMeta.storageKey)); }
        catch (error) { }
      }
      assertCurrentProject();
      const nextMeta = assetIdentity(id, file, kind, prepared, fingerprint, storageKey, sourcePath, true, currentMeta.layerDefinition);
      Object.assign(prepared, nextMeta);

      const applyVersion = (meta: any, runtime: any, poster: Blob | null) => {
        if (!PM.proj || PM.proj.id !== targetProjectId || !PM.proj.assets?.[id]) return;
        viewerService(PM)?.preview?.clear();
        if (kind === 'audio' || kind === 'video') PM.Audio?.pause?.();
        PM.proj.assets[id] = JSON.parse(JSON.stringify(meta));
        if (runtime) PM.assets.map.set(id, runtime);
        else PM.assets.map.delete(id);
        if (poster) setPoster(id, poster);
        else revokePoster(id);
        PM.rasterClear?.();
        PM.GL?.dropTextures?.(`a:${id}`);
        PM.GL?.dropMesh?.(id);
        PM.preparedVideoFrames?.clear?.();
        PM.touch();
        checkpointAssetMetadata(PM.proj);
        PM.bus.emit('assets');
        PM.bus.emit('layers');
        PM.bus.emit('project');
        inspectorService(PM)?.refresh();
        PM.invalidate('all');
        PM.autosave?.();
      };

      applyVersion(nextMeta, prepared, nextPoster);
      const retainedRuntimes = [...new Set([previousRuntime, prepared].filter(Boolean))];
      retainedRuntimes.forEach(retainHistoryAsset);
      const historyId = PM.hist.external(
        `Replace ${currentMeta.name || 'media'}`,
        () => applyVersion(previousMeta, previousRuntime, previousPoster),
        () => applyVersion(nextMeta, prepared, nextPoster),
        {
          bytes: Number(previousMeta.size || 0) + Number(nextMeta.size || 0),
          cleanup: () => retainedRuntimes.forEach(runtime => releaseHistoryAsset(runtime, id)),
        },
      );
      if (!historyId) {
        applyVersion(previousMeta, previousRuntime, previousPoster);
        retainedRuntimes.forEach(runtime => releaseHistoryAsset(runtime, id));
        throw new Error('Could not add the replacement to Undo history · the original file is unchanged');
      }
      if (kind === 'audio' && PM.Audio) PM.Audio.rebalanceCache();
      return { asset: prepared, previous: previousRuntime, previousName: currentMeta.name, meta: nextMeta, persisted: true };
    } catch (error) {
      if (prepared && PM.assets.map.get(id) !== prepared) disposeAsset(prepared);
      throw error;
    }
  },
  get: (id: any) => PM.assets.map.get(id),
  poster: (id: any) => posterUrls.get(String(id)) || '',
  revokePoster,
  clear() {
    assetEpoch++;
    cloudMedia.clear();
    for (const [id, a] of PM.assets.map) {
      disposeAsset(a);
      PM.GL?.dropMesh?.(id);
    }
    PM.assets.map.clear();
    PM.assets.loading.clear();
    PM.assets.errors.clear();
    PM.GL?.dropTextures?.('a:');
    PM.GL?.dropTextures?.('video:');
    for (const id of [...posterUrls.keys()]) revokePoster(id);
  },
  async restoreProject(project: any) {
    const epoch = assetEpoch;
    const metas: any[] = Object.values(project && project.assets || {})
      .filter((meta: any) => meta && meta.id && ['image', 'video', 'audio', 'model'].includes(meta.kind));
    const restored: any[] = [], missing: any[] = [];
    const loading = new Set(metas.map(meta => String(meta.id)));
    PM.assets.loading = loading;
    PM.bus?.emit?.('assets');
    const results = await PM.MediaImport.mapBounded(metas, 3, async (meta: any) => {
      const current = () => epoch === assetEpoch && PM.proj === project && project.assets?.[meta.id] === meta;
      if (!current()) return { stale: true };
      PM.assets.errors.delete(meta.id);
      let posterBlob: any = null;
      let cacheError: unknown;
      try {
        const posterKey = meta.storageKey
          ? PM.MediaImport.posterKeyFor(meta.storageKey)
          : null;
        const [blob, storedPoster] = await Promise.all([
          Promise.resolve().then(() => PM.MediaStore.get(meta)).catch(error => { cacheError = error; return null; }),
          posterKey ? Promise.resolve(PM.MediaStore.get(posterKey)).catch(() => null) : Promise.resolve(null),
        ]);
        if (!current()) return { stale: true };
        posterBlob = storedPoster;
        if (blob) {
          try {
            const asset = await prepareAsset({ id: meta.id, name: meta.name, kind: meta.kind, blob, meta });
            Object.assign(asset, {
              fingerprint: meta.fingerprint, storageKey: meta.storageKey, persisted: true,
              channels: asset.channels || meta.channels || 0,
              sampleRate: asset.sampleRate || meta.sampleRate || 0,
            });
            if (!current()) { disposeAsset(asset); return { stale: true }; }
            // Publish each ready asset without waiting for unrelated conversions.
            PM.assets.map.set(asset.id, asset);
            return { asset, meta, posterBlob };
          } catch (error) { cacheError = error; }
        }
        const file = await readLocalMediaSource(meta, current);
        if (!current()) return { stale: true };
        if (file) {
          await recoverSourceAsset(meta, file, current);
          if (!current()) return { stale: true };
          return { asset: PM.assets.get(meta.id), meta, posterBlob };
        }
        if (cacheError) throw cacheError;
        return { meta, missing: true, posterBlob };
      } catch (error) {
        if (!current()) return { stale: true };
        PM.assets.errors.set(meta.id, error instanceof Error ? error.message : 'Could not read this media file');
        return { meta, missing: true, posterBlob, error };
      }
      finally {
        loading.delete(String(meta.id));
        if (epoch === assetEpoch && PM.proj === project) {
          PM.bus?.emit?.('assets');
          PM.invalidate?.('render');
        }
      }
    });
    results.forEach((result: any) => {
      if (result.asset) restored.push(result.asset);
      else if (result.missing) missing.push(result.meta);
    });
    if (epoch !== assetEpoch || PM.proj !== project) {
      restored.forEach(disposeAsset);
      return { restored: [], missing: [], stale: true };
    }
    let repairedPosterMetadata = false;
    results.forEach((result: any) => {
      if (!result.posterBlob || !result.meta?.id || project.assets?.[result.meta.id] !== result.meta) return;
      setPoster(result.meta.id, result.posterBlob);
      if (result.meta.poster !== true) {
        result.meta.poster = true;
        repairedPosterMetadata = true;
      }
    });
    if (repairedPosterMetadata) {
      PM.touch();
      checkpointAssetMetadata(project);
    }
    restored.forEach((a: any) => {
      /* Older projects already preserve the original .svg bytes and name but
         predate the explicit vector marker. Hydrate that marker in place so
         the editor UI and subsequent web exports also identify the asset. */
      const meta: any = project.assets?.[a.id];
      if (a.format && meta && meta.format !== a.format) meta.format = a.format;
    });
    const backfill = results.filter((result: any) => result.asset && !result.posterBlob);
    if (backfill.length) setTimeout(() => {
      void (async () => {
        if (epoch !== assetEpoch || PM.proj !== project) return;
        const completed = await PM.MediaImport.mapBounded(backfill, 3, async (result: any) => {
          if (epoch !== assetEpoch || PM.proj !== project) return null;
          const blob = await capturePoster(result.asset);
          if (!blob || epoch !== assetEpoch || PM.proj !== project) return null;
          const meta = project.assets?.[result.asset.id];
          if (!meta?.storageKey || meta !== result.meta || PM.assets.map.get(result.asset.id) !== result.asset) return null;
          const posterKey = PM.MediaImport.posterKeyFor(meta.storageKey);
          try {
            const stored = await PM.MediaStore.put(posterKey, blob, { storageKey: posterKey, type: blob.type });
            return stored ? { id: result.asset.id, blob, meta, asset: result.asset } : null;
          } catch (error) { return null; }
        });
        if (epoch !== assetEpoch || PM.proj !== project) return;
        const available = completed.filter(Boolean);
        if (!available.length) return;
        let applied = 0;
        available.forEach(({ id, blob, meta, asset }: any) => {
          if (project.assets?.[id] !== meta || PM.assets.map.get(id) !== asset) return;
          meta.poster = true;
          setPoster(id, blob);
          applied++;
        });
        if (applied) {
          PM.touch();
          checkpointAssetMetadata(project);
          PM.bus?.emit?.('assets');
        }
      })();
    }, 0);
    await cloudMedia.scan(project, missing);
    if (epoch !== assetEpoch || PM.proj !== project) return { restored: [], missing: [], stale: true };
    return { restored, missing: missing.filter(meta => !cloudMedia.get(meta.id) && !PM.assets.get(meta.id)) };
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

/* SVG source remains in the durable asset store and in the live <img>. This
   cache is only the final GPU sampling boundary, rebuilt in resolution buckets
   chosen by the compositor from the layer's current display transform. */
PM.rasterSvgAsset = (asset: any, requestedWidth: any, requestedHeight: any) => {
  const width = Math.max(1, Math.round(Number(requestedWidth) || 1));
  const height = Math.max(1, Math.round(Number(requestedHeight) || 1));
  const key = `svg:${asset?.id || 'missing'}:${width}x${height}`;
  let entry = cache.get(key);
  if (entry) { entry.used = ++tick; return entry; }
  const cv = getCanvas(width, height);
  const context = cv.getContext('2d');
  if (!context || !asset?.el) throw new Error('Could not rasterize this SVG file');
  context.clearRect(0, 0, width, height);
  context.drawImage(asset.el, 0, 0, width, height);
  entry = { cv, w: width, h: height, key, used: ++tick, bytes: width * height * 4 };
  cache.set(key, entry);
  cacheBytes += entry.bytes;
  evict();
  PM.Memory?.maintain?.('raster');
  return entry;
};
}
