// Reference page for the 3D sweep (tools/three-ref/cases.ts): renders each
// case with the production SceneRuntime (three.js r180) exactly as the
// compositor renders 3D layers — the scene as `compositionRuntime` places it
// (world matrices flattened under the scene, layer opacity and masks in
// `userData`, a camera layer's pose), then every output in order through
// `runtime.render(w, h, objectId, depthIds, background, camera, look)` (the
// shared depth prepass cached between outputs of one set, as in the app), or
// the whole scene for a legacy scene layer — with and without MSAA, recording
// every linked program's sources and every draw's uniform values.
import * as THREE from '../../../../../apps/desktop/node_modules/three/build/three.module.js';
import { SceneRuntime } from '../../../../../apps/desktop/src/renderer/src/core/scene3d/runtime';

type Uniforms = Record<string, number[]>;
type Program = { id: number; vertex: string; fragment: string };
type Draw = { program: number; colorMask: boolean[]; depthMask: boolean; mode: number; count: number; uniforms: Uniforms };

const programs = new Map<WebGLProgram, Program>();
const shaders = new Map<WebGLShader, string>();
const attached = new Map<WebGLProgram, WebGLShader[]>();
const locations = new Map<WebGLUniformLocation, { program: WebGLProgram; name: string }>();
const values = new Map<WebGLProgram, Uniforms>();
let current: WebGLProgram | null = null;
let colorMask = [true, true, true, true];
let depthMask = true;
let draws: Draw[] = [];
let nextProgram = 1;

function hook() {
  const P = WebGL2RenderingContext.prototype as any;
  const wrap = (name: string, after: (args: any[], result: any) => void) => {
    const original = P[name];
    P[name] = function (this: WebGL2RenderingContext, ...args: any[]) {
      const result = original.apply(this, args);
      after(args, result);
      return result;
    };
  };
  wrap('shaderSource', ([s, src]) => shaders.set(s, src));
  wrap('attachShader', ([p, s]) => { const list = attached.get(p) || []; list.push(s); attached.set(p, list); });
  wrap('linkProgram', ([p]) => {
    const [a, b] = (attached.get(p) || []).map(s => shaders.get(s) || '');
    const vertex = a!.includes('gl_Position') ? a! : b!, fragment = vertex === a ? b! : a!;
    programs.set(p, { id: nextProgram++, vertex, fragment });
  });
  wrap('getUniformLocation', ([p, name], loc) => { if (loc) locations.set(loc, { program: p, name }); });
  wrap('useProgram', ([p]) => { current = p; });
  wrap('colorMask', (m) => { colorMask = m.map(Boolean); });
  wrap('depthMask', ([m]) => { depthMask = !!m; });
  const set = (args: any[], count: number, matrix = false) => {
    const loc = args[0];
    if (!loc) return;
    const at = locations.get(loc);
    if (!at) return;
    let v: number[];
    if (matrix) v = Array.from(args[2] as ArrayLike<number>);
    else if (count === 0) v = Array.from(args[1] as ArrayLike<number>);
    else v = args.slice(1, 1 + count).map(Number);
    const map = values.get(at.program) || {};
    map[at.name] = v;
    values.set(at.program, map);
  };
  for (const [name, n] of [['uniform1f', 1], ['uniform2f', 2], ['uniform3f', 3], ['uniform4f', 4], ['uniform1i', 1], ['uniform2i', 2], ['uniform3i', 3], ['uniform4i', 4]] as const)
    wrap(name, (args) => set(args, n));
  for (const name of ['uniform1fv', 'uniform2fv', 'uniform3fv', 'uniform4fv', 'uniform1iv', 'uniform2iv', 'uniform3iv', 'uniform4iv'])
    wrap(name, (args) => set(args, 0));
  for (const name of ['uniformMatrix2fv', 'uniformMatrix3fv', 'uniformMatrix4fv'])
    wrap(name, (args) => set(args, 0, true));
  const draw = (args: any[]) => {
    if (!current) return;
    draws.push({ program: programs.get(current)!.id, colorMask: [...colorMask], depthMask, mode: args[0], count: args[1],
      uniforms: JSON.parse(JSON.stringify(values.get(current) || {})) });
  };
  wrap('drawElements', draw);
  wrap('drawArrays', (args) => draw([args[0], args[2]]));
}
hook();

async function post(path: string, body: BodyInit, query: Record<string, string | number> = {}) {
  const q = new URLSearchParams(Object.entries(query).map(([k, v]) => [k, String(v)])).toString();
  const response = await fetch(`/out/${path}${q ? `?${q}` : ''}`, { method: 'POST', body });
  if (!response.ok) throw new Error(`POST ${path}: ${response.status}`);
}

function read(gl: WebGL2RenderingContext, w: number, h: number): Uint8Array {
  const pixels = new Uint8Array(w * h * 4);
  gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
  return pixels;
}

function editorCamera(e: any) {
  const camera = e.projection === 'orthographic' ? new THREE.OrthographicCamera(-3, 3, 3, -3, e.near, e.far) : new THREE.PerspectiveCamera(e.fov, 16 / 9, e.near, e.far);
  camera.zoom = e.zoom;
  camera.position.fromArray(e.position);
  camera.up.fromArray(e.up);
  camera.lookAt(new THREE.Vector3().fromArray(e.target));
  camera.updateProjectionMatrix();
  camera.updateMatrixWorld(true);
  return camera;
}

let plain: THREE.WebGLRenderer | null = null;

async function renderCase(c: any) {
  const assets = new Map<string, any>();
  for (const [id, name] of Object.entries<string>(c.textures || {})) {
    const blob = await (await fetch(`/tex/${name}.png`)).blob();
    // The asset store decodes images with `createImageBitmap(blob)`.
    assets.set(id, { kind: 'image', bitmap: await createImageBitmap(blob) });
  }
  const runtime = new SceneRuntime();
  runtime.sync(c.scene, (id: string) => assets.get(id) ?? null, (p: any) => p.v, 0, true);
  if (c.matrices) {
    for (const [id, m] of Object.entries<number[]>(c.matrices)) {
      const object = runtime.objects.get(id)!;
      runtime.scene.add(object);
      object.matrixAutoUpdate = false;
      object.matrix.fromArray(m);
      object.matrix.decompose(object.position, object.quaternion, object.scale);
      object.rotation.order = 'ZYX';
      object.userData.layerOpacity = c.layers?.[id]?.opacity ?? 1;
      object.userData.layerOccludes = c.layers?.[id]?.occludes ?? true;
    }
  }
  if (c.cameraPose) {
    const camera = runtime.camera;
    camera.position.fromArray(c.cameraPose.position);
    camera.up.fromArray(c.cameraPose.up);
    camera.lookAt(new THREE.Vector3().fromArray(c.cameraPose.target));
    camera.updateMatrixWorld(true);
  }
  runtime.scene.updateMatrixWorld(true);
  const { width: w, height: h } = c;
  const outputs: any[] = [];
  for (let k = 0; k < c.outputs.length; k++) {
    const o = c.outputs[k];
    const camera = o.editorCamera ? editorCamera(o.editorCamera) : runtime.camera;
    const render = () => o.object
      ? runtime.render(w, h, o.object, new Set(o.depthIds), !!o.background, camera, o.look || {})
      : runtime.render(w, h);
    draws = [];
    render();
    const gl = (runtime as any).renderer.getContext() as WebGL2RenderingContext;
    const msaa = read(gl, w, h);
    const recorded = draws;
    draws = [];
    // The same output without MSAA (classifies MSAA edge pixels).
    plain ??= new THREE.WebGLRenderer({ alpha: true, antialias: false, premultipliedAlpha: true, preserveDrawingBuffer: true });
    plain.outputColorSpace = THREE.SRGBColorSpace; plain.toneMapping = THREE.ACESFilmicToneMapping; plain.shadowMap.type = THREE.PCFSoftShadowMap;
    const own = (runtime as any).renderer;
    (runtime as any).renderer = plain;
    let noaa: Uint8Array;
    try {
      plain.setSize(w, h, false);
      runtime.render(w, h); // resets the shared depth-prepass cache
      render();
      noaa = read(plain.getContext() as WebGL2RenderingContext, w, h);
      // The MSAA renderer's depth buffer and shadow maps still hold this
      // set's prepass, so the next output of the set reuses it as in the app.
    } finally { (runtime as any).renderer = own; }
    await post(`x/${c.id}/${k}.msaa.rgba`, msaa, { w, h });
    await post(`x/${c.id}/${k}.noaa.rgba`, noaa, { w, h });
    const used = new Set(recorded.map(d => d.program));
    await post(`x/${c.id}/${k}.json`, JSON.stringify({
      id: c.id, output: k, samples: (gl.getContextAttributes() as any)?.antialias ? gl.getParameter(gl.SAMPLES) : 0,
      programs: [...programs.values()].filter(p => used.has(p.id)), draws: recorded,
    }));
    outputs.push({ draws: recorded.length });
  }
  runtime.dispose();
  return { id: c.id, outputs };
}

(globalThis as any).ThreeRef = { renderCase, version: THREE.REVISION };
