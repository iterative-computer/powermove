// Path mask coverage reference page (docs/engine/composite.md): runs the
// coverage half of today's `applyPathMasks` (legacy/gl/compositor.ts, sliced
// verbatim and transpiled by run.ts) for every case of cases.ts, uploads the
// coverage canvas to a WebGL2 texture as the compositor's `texFor` does, and
// reads the texture back.
import { pathValues, tracePath } from '../../../../../apps/desktop/src/renderer/src/legacy/core/vector-paths';
import { evaluatedValue } from '../../../../../apps/desktop/src/renderer/src/legacy/core/content-properties';
import { cases, type Case } from './cases';

declare const COVERAGE_SOURCE: string;

// Values are given already evaluated: `PM.evP` returns them as they are.
const PM = { evP: (_layer: unknown, value: unknown) => value };

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

// coverage(L, T, W, H, masks) -> the coverage canvas.
const coverage = new Function('PM', 'evaluatedValue', 'scaledWorld', 'pathValues', 'tracePath', COVERAGE_SOURCE);

function legacyMask(m: Case['masks'][number], index: number) {
  return {
    id: `m${index}`, on: true, mode: m.mode, shape: m.shape, p: m.p,
    path: m.path && {
      id: `mp${index}`, name: 'Mask path', parent: null,
      p: { x: m.path.x, y: m.path.y, rotation: m.path.rotation, scaleX: m.path.scaleX, scaleY: m.path.scaleY, closed: true, trimStart: 0, trimEnd: 100, trimOffset: 0 },
      vertices: m.path.vertices.map((vertex, i) => ({ id: `v${i}`, p: vertex })),
    },
  };
}

(async () => {
  try {
    const list = cases();
    for (const c of list) {
      let cv: HTMLCanvasElement;
      try {
        const run = coverage(PM, evaluatedValue, () => c.world, pathValues, tracePath);
        cv = run({ id: 'layer' }, 0, c.width, c.height, c.masks.map(legacyMask));
      } catch (e) {
        await post('case', JSON.stringify({ id: c.id, error: String((e as Error)?.message || e) }), { id: c.id, kind: 'error' });
        continue;
      }
      const px = upload(cv);
      await post('case', px, { id: c.id, kind: 'rgba', w: cv.width, h: cv.height });
    }
    await post('done', String(list.length));
  } catch (e) {
    await post('failed', String((e as Error)?.stack || e));
  }
})();
