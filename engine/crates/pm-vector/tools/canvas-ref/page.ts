// Canvas2D reference page (docs/engine/vector.md): rasterizes every case of
// cases.ts with today's code (legacy/gl/raster.ts `rasterShape`, copied
// verbatim below and checked against the source by run.ts; legacy
// `rasterPaths`, imported), uploads each canvas to a WebGL2 texture exactly as
// the compositor's `texFor` does (UNPACK_PREMULTIPLY_ALPHA_WEBGL), and reads
// the texture back: the premultiplied bytes the compositor samples.
import { shapeRasterGeometry, type RasterWindow } from '../../../../../apps/desktop/src/renderer/src/legacy/gl/shape-raster-window';
import { isUniformCircular, roundedRectPath } from '../../../../../apps/desktop/src/renderer/src/legacy/gl/corner-geometry';
import { rasterPaths } from '../../../../../apps/desktop/src/renderer/src/legacy/core/vector-paths';
import { cases, type Case } from './cases';

function getCanvas(w: any, h: any) {
  const cv = window.document.createElement('canvas');
  cv.width = Math.max(1, Math.ceil(w)); cv.height = Math.max(1, Math.ceil(h));
  return cv;
}

// BEGIN VERBATIM raster.ts
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
  } else if (isUniformCircular(d)) rr(c, 0, 0, W, H, d.radius || 0);
  else {
    const outline = new Path2D(roundedRectPath(d));
    c.fill(outline);
    if (d.stroke > 0) c.stroke(outline);
    return { cv, w, h: hh, anchorX: pad, anchorY: pad, selection, uv };
  }
  c.fill();
  if (d.stroke > 0) c.stroke();
  return { cv, w, h: hh, anchorX: pad, anchorY: pad, selection, uv };
}
// END VERBATIM raster.ts

const PM = {
  mul(m: any, n: any) {
    return [
      m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1],
      m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3],
      m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5],
    ];
  },
  clamp: (v: any, a: any, b: any) => (v < a ? a : v > b ? b : v),
  evP: (_layer: any, p: any) => p.v,
};

const gl = document.createElement('canvas').getContext('webgl2', { premultipliedAlpha: true, antialias: false })!;
function upload(cv: HTMLCanvasElement): Uint8Array {
  const w = cv.width, h = cv.height;
  const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, cv);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  const fb = gl.createFramebuffer(); gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t, 0);
  const px = new Uint8Array(w * h * 4);
  gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null); gl.deleteFramebuffer(fb); gl.deleteTexture(t);
  return px;
}

async function post(path: string, body: BodyInit, params: Record<string, string | number> = {}) {
  const q = new URLSearchParams(Object.entries(params).map(([k, v]) => [k, String(v)]));
  const r = await fetch(`/${path}?${q}`, { method: 'POST', body });
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
}

async function rasterize(c: Case): Promise<{ cv: HTMLCanvasElement; meta: Record<string, unknown> }> {
  if (c.kind === 'raw') {
    const cv = getCanvas(c.width, c.height);
    const ctx = cv.getContext('2d') as any;
    for (const [op, ...args] of c.ops) {
      if (String(op).startsWith('=')) ctx[String(op).slice(1)] = args[0];
      else ctx[op as string](...args);
    }
    return { cv, meta: {} };
  }
  if (c.kind === 'svg') {
    // legacy/gl/raster.ts rasterSvgAsset: an <img> of the SVG blob, drawn
    // into a canvas of the bucket size.
    const url = URL.createObjectURL(new Blob([c.svg], { type: 'image/svg+xml' }));
    const img = new Image();
    img.src = url;
    await img.decode();
    const cv = getCanvas(c.width, c.height);
    const context = cv.getContext('2d')!;
    context.clearRect(0, 0, c.width, c.height);
    context.drawImage(img, 0, 0, c.width, c.height);
    URL.revokeObjectURL(url);
    return { cv, meta: { naturalWidth: img.naturalWidth, naturalHeight: img.naturalHeight } };
  }
  if (c.kind === 'shape') {
    const r = rasterShape(c.d, c.scale);
    return { cv: r.cv, meta: { w: r.w, h: r.h } };
  }
  const layer = { d: { paths: c.paths.map(p => ({ id: p.id, name: p.id, parent: p.parent, p: p.p, vertices: p.vertices.map((v, i) => ({ id: `v${i}`, p: { inX: 0, inY: 0, outX: 0, outY: 0, ...v } })) })) } };
  const r = rasterPaths(PM, layer, 0, c.scale);
  return { cv: r.cv, meta: { w: r.w, h: r.h, anchorX: r.anchorX, anchorY: r.anchorY } };
}

(async () => {
  try {
    const list = cases();
    for (const c of list) {
      let result;
      try { result = await rasterize(c); } catch (e) {
        await post('case', JSON.stringify({ id: c.id, error: String((e as Error)?.message || e) }), { id: c.id, kind: 'error' });
        continue;
      }
      const px = upload(result.cv);
      await post('case', px, { id: c.id, kind: 'rgba', w: result.cv.width, h: result.cv.height, meta: JSON.stringify(result.meta) });
    }
    await post('done', String(list.length));
  } catch (e) {
    await post('failed', String((e as Error)?.stack || e));
  }
})();
