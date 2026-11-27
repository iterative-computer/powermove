// Reference page for the native 3D pass (docs/engine/pm-render.md): renders the
// scene cases of tests/fixtures/three-scenes.json with the production SceneRuntime
// (three.js r180) exactly as the compositor renders a 3D object layer (shared depth
// prepass, then the layer's color pass), with and without MSAA, and records every
// linked program's sources and the uniform values of every draw.
import * as THREE from '../../../../../apps/desktop/node_modules/three/build/three.module.js';
import { SceneRuntime } from '../../../../../apps/desktop/src/renderer/src/core/scene3d/runtime';

type Uniforms = Record<string, number[]>;
type Program = { id: number; vertex: string; fragment: string };
type Draw = { program: number; colorMask: boolean[]; depthMask: boolean; uniforms: Uniforms };

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
  const draw = () => {
    if (!current) return;
    draws.push({ program: programs.get(current)!.id, colorMask: [...colorMask], depthMask, uniforms: JSON.parse(JSON.stringify(values.get(current) || {})) });
  };
  wrap('drawElements', draw);
  wrap('drawArrays', draw);
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

const ch = (v: any) => ({ v, kf: [], expr: null });

/** The production scene for a case, as `compositionScene` would produce it. */
function sceneOf(c: any) {
  const m = c.object.material;
  return {
    objects: [{ id: 'obj', name: 'Object', source: { primitive: c.object.primitive, parameters: c.object.parameters },
      material: { p: { color: ch(m.color), roughness: ch(m.roughness), metalness: ch(m.metalness), emissive: ch(m.emissive), emissiveIntensity: ch(m.emissiveIntensity), opacity: ch(m.opacity) }, maps: {}, doubleSided: m.doubleSided },
      castShadow: true, receiveShadow: true }],
    lights: c.lights.map((l: any, i: number) => ({ id: `sun${i}`, name: 'Light', type: 'sun', castShadow: false,
      p: { x: ch(l.position[0]), y: ch(l.position[1]), z: ch(l.position[2]), targetX: ch(l.target[0]), targetY: ch(l.target[1]), targetZ: ch(l.target[2]), color: ch(l.color), intensity: ch(l.intensity), visible: ch(l.visible) } })),
    camera: { projection: c.camera.projection, p: { x: ch(c.camera.position[0]), y: ch(c.camera.position[1]), z: ch(c.camera.position[2]), targetX: ch(c.camera.target[0]), targetY: ch(c.camera.target[1]), targetZ: ch(c.camera.target[2]), fov: ch(c.camera.fov), zoom: ch(c.camera.zoom), near: ch(c.camera.near), far: ch(c.camera.far) } },
    environment: { p: { ambient: ch(c.environment.ambient), ambientColor: ch(c.environment.ambientColor), exposure: ch(c.environment.exposure) }, background: c.environment.background, shadows: false },
  };
}

let plain: THREE.WebGLRenderer | null = null;

async function renderCase(c: any, out: string) {
  const runtime = new SceneRuntime();
  runtime.sync(sceneOf(c) as any, () => null, (p: any) => p.v, 0);
  // compositionRuntime: the layer's world matrix as given, the camera's up and aim.
  const object = runtime.objects.get('obj')!;
  object.matrixAutoUpdate = false;
  object.matrix.fromArray(c.object.matrix);
  object.matrix.decompose(object.position, object.quaternion, object.scale);
  object.userData.layerOpacity = c.layerOpacity;
  object.userData.layerOccludes = true;
  const camera = runtime.camera;
  camera.position.fromArray(c.camera.position);
  camera.up.fromArray(c.camera.up);
  camera.lookAt(new THREE.Vector3().fromArray(c.camera.target));
  camera.updateMatrixWorld(true);
  runtime.scene.updateMatrixWorld(true);
  const { width: w, height: h } = c;
  const ids = new Set(['obj']);
  draws = [];
  const used = new Set<number>();
  const canvas = runtime.render(w, h, 'obj', ids, !!c.bottom);
  const gl = (runtime as any).renderer.getContext() as WebGL2RenderingContext;
  const msaa = read(gl, w, h);
  const recorded = draws;
  for (const d of recorded) used.add(d.program);
  // The same passes without MSAA (classifies MSAA edge pixels).
  plain ??= new THREE.WebGLRenderer({ alpha: true, antialias: false, premultipliedAlpha: true, preserveDrawingBuffer: true });
  plain.outputColorSpace = THREE.SRGBColorSpace; plain.toneMapping = THREE.ACESFilmicToneMapping; plain.shadowMap.type = THREE.PCFSoftShadowMap;
  const own = (runtime as any).renderer;
  (runtime as any).renderer = plain;
  let noaa: Uint8Array;
  try {
    plain.setSize(w, h, false);
    runtime.render(w, h); // resets the shared depth-prepass cache
    runtime.render(w, h, 'obj', ids, !!c.bottom);
    noaa = read(plain.getContext() as WebGL2RenderingContext, w, h);
  } finally { (runtime as any).renderer = own; }
  await post(`${out}/${c.id}/msaa.rgba`, msaa, { w, h });
  await post(`${out}/${c.id}/noaa.rgba`, noaa, { w, h });
  const progs = [...programs.values()].filter(p => used.has(p.id));
  await post(`${out}/${c.id}/record.json`, JSON.stringify({
    id: c.id, samples: (gl.getContextAttributes() as any)?.antialias ? gl.getParameter(gl.SAMPLES) : 0,
    camera: { matrixWorld: camera.matrixWorld.elements, projection: camera.projectionMatrix.elements },
    programs: progs, draws: recorded,
  }));
  runtime.dispose();
  void canvas;
  return { id: c.id, draws: recorded.length, programs: progs.map(p => p.id) };
}

(globalThis as any).ThreeRef = { renderCase, version: THREE.REVISION };
