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
import { loadGltf } from '../../../../../apps/desktop/src/renderer/src/core/scene3d/import-model';

type Uniforms = Record<string, number[]>;
type Program = { id: number; vertex: string; fragment: string };
type Draw = { program: number; colorMask: boolean[]; depthMask: boolean; mode: number; count: number; uniforms: Uniforms };

const programs = new Map<WebGLProgram, Program>();
const shaders = new Map<WebGLShader, string>();
const attached = new Map<WebGLProgram, WebGLShader[]>();
const locations = new Map<WebGLUniformLocation, { program: WebGLProgram; name: string }>();
const values = new Map<WebGLProgram, Uniforms>();
// Bound program and masks per context: three.js's state cache skips
// `useProgram` and mask calls a context already has, and two contexts (the
// MSAA canvas and the plain one) interleave.
type ContextState = { current: WebGLProgram | null; colorMask: boolean[]; depthMask: boolean };
const contexts = new WeakMap<WebGL2RenderingContext, ContextState>();
const ctx = (gl: WebGL2RenderingContext) => {
  let s = contexts.get(gl);
  if (!s) { s = { current: null, colorMask: [true, true, true, true], depthMask: true }; contexts.set(gl, s); }
  return s;
};
let draws: Draw[] = [];
let nextProgram = 1;

function hook() {
  const P = WebGL2RenderingContext.prototype as any;
  const wrap = (name: string, after: (args: any[], result: any, gl: WebGL2RenderingContext) => void) => {
    const original = P[name];
    P[name] = function (this: WebGL2RenderingContext, ...args: any[]) {
      const result = original.apply(this, args);
      after(args, result, this);
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
  wrap('useProgram', ([p], _, gl) => { ctx(gl).current = p; });
  wrap('colorMask', (m, _, gl) => { ctx(gl).colorMask = m.map(Boolean); });
  wrap('depthMask', ([m], _, gl) => { ctx(gl).depthMask = !!m; });
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
  const draw = (args: any[], gl: WebGL2RenderingContext) => {
    const s = ctx(gl);
    if (!s.current) return;
    draws.push({ program: programs.get(s.current)!.id, colorMask: [...s.colorMask], depthMask: s.depthMask, mode: args[0], count: args[1],
      uniforms: JSON.parse(JSON.stringify(values.get(s.current) || {})) });
  };
  wrap('drawElements', (args, _, gl) => draw(args, gl));
  wrap('drawArrays', (args, _, gl) => draw([args[0], args[2]], gl));
  wrap('drawElementsInstanced', (args, _, gl) => draw(args, gl));
  wrap('drawArraysInstanced', (args, _, gl) => draw([args[0], args[2]], gl));
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
  // Model assets through the production importer (glTF cases).
  for (const [id, file] of Object.entries<string>(c.models || {})) {
    const blob = await (await fetch(`/models/${encodeURIComponent(file)}`)).blob();
    assets.set(id, { id, kind: 'model', format: /\.gltf$/i.test(file) ? 'gltf' : 'glb', blob, ...(await loadGltf(blob)) });
  }
  const runtime = new SceneRuntime();
  const time = c.time ?? 0;
  const animationTime = c.animationTimes ? (id: string) => c.animationTimes[id] ?? time : undefined;
  runtime.sync(c.scene, (id: string) => assets.get(id) ?? null, (p: any) => p.v, time, true, animationTime);
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
  if (c.models) await post(`x/${c.id}/pose.json`, JSON.stringify(poses(runtime, c)));
  if (c.picks) await post(`x/${c.id}/picks.json`, JSON.stringify(picks(runtime, c)));
  runtime.dispose();
  return { id: c.id, outputs };
}

/**
 * The posed drawables of every model layer (traverse order): kind, world
 * matrix, morph influences, skinning (`bindMatrixInverse`, `boneMatrices`),
 * instance matrices, and embedded lights with their targets.
 */
function poses(runtime: SceneRuntime, c: any) {
  runtime.scene.updateMatrixWorld(true);
  const out: any = {};
  for (const [id, object] of runtime.objects) {
    const node = c.scene.objects?.find((o: any) => o.id === id);
    if (!node || !('assetId' in node.source)) continue;
    const meshes: any[] = [], lights: any[] = [];
    object.traverse((o: any) => {
      if (o.isMesh || o.isPoints || o.isLine) {
        const kind = o.isSkinnedMesh ? 'skinned' : o.isInstancedMesh ? 'instanced' : o.isPoints ? 'points' : o.isLineLoop ? 'lineLoop'
          : o.isLineSegments ? 'lineSegments' : o.isLine ? 'line' : 'mesh';
        const m: any = { name: o.name, kind, matrixWorld: Array.from(o.matrixWorld.elements), influences: o.morphTargetInfluences ? Array.from(o.morphTargetInfluences) : null };
        if (o.isSkinnedMesh && o.skeleton) {
          o.skeleton.update();
          m.bindMatrixInverse = Array.from(o.bindMatrixInverse.elements);
          m.boneMatrices = Array.from(o.skeleton.boneMatrices);
        }
        if (o.isInstancedMesh) m.instanceMatrix = Array.from(o.instanceMatrix.array);
        meshes.push(m);
      }
      if (o.isLight) lights.push({ name: o.name, matrixWorld: Array.from(o.matrixWorld.elements), target: o.target ? Array.from(o.target.matrixWorld.elements) : null,
        color: o.color.toArray(), intensity: o.intensity, distance: o.distance ?? null, angle: o.angle ?? null, penumbra: o.penumbra ?? null });
    });
    out[id] = { meshes, lights };
  }
  return out;
}

/**
 * three.js raycasts against the model layers on a grid of rays (the
 * engine's picks are compared with these): skinned bounding volumes are
 * recomputed for the current pose first (production keeps the first
 * pose's; spike S7 defect 5); hits as `runtime.pick` filters them.
 */
function picks(runtime: SceneRuntime, c: any) {
  runtime.resizeCamera(c.width, c.height);
  runtime.scene.updateMatrixWorld(true);
  const models: any[] = [];
  for (const [id, object] of runtime.objects) {
    const node = c.scene.objects?.find((o: any) => o.id === id);
    if (node && 'assetId' in node.source) models.push(object);
  }
  for (const m of models) m.traverse((o: any) => { if (o.isSkinnedMesh) { o.computeBoundingSphere(); o.computeBoundingBox(); } });
  const ray = new THREE.Raycaster(), rays: any[] = [];
  for (let j = 0; j < c.picks.rows; j++) for (let i = 0; i < c.picks.cols; i++) {
    const x = ((i + 0.5) / c.picks.cols) * 2 - 1, y = 1 - ((j + 0.5) / c.picks.rows) * 2;
    ray.setFromCamera(new THREE.Vector2(x, y), runtime.camera);
    let hit: any = null;
    for (const h of ray.intersectObjects(models, true)) {
      if (!h.object.visible || !h.object.userData.sceneId) continue;
      const materials = (h.object as any).material;
      if (materials && (Array.isArray(materials) ? materials : [materials]).every((m: any) => m.opacity <= 0)) continue;
      let visible = true, parent: any = h.object;
      while (parent) { if (!parent.visible) visible = false; parent = parent.parent; }
      if (visible) { hit = h; break; }
    }
    rays.push({ x, y, origin: ray.ray.origin.toArray(), direction: ray.ray.direction.toArray(),
      hit: hit && { layer: hit.object.userData.sceneId, object: hit.object.name, distance: hit.distance, faceIndex: hit.faceIndex ?? null,
        index: hit.index ?? null, instanceId: hit.instanceId ?? null } });
  }
  return { near: ray.near, far: ray.far, rays };
}

// A legacy OBJ mesh layer as `meshExtensionQuad` draws it: the compositor's
// own shader sources (passed in from compositor.ts), the kernel's `parseObj`,
// a half-float target with a 16-bit depth buffer cleared to the background,
// LEQUAL depth, no blending; or, without a mesh, the missing-definition
// checkerboard through `PM.VERT` and `PM.SHADER_HEADER`.
import { parseObj } from '../../../../../apps/desktop/src/renderer/src/kernel/obj';

const hex2rgb = (hex: string) => { const s = hex.replace('#', ''); return [0, 2, 4].map(i => (parseInt(s.slice(i, i + 2), 16) || 0) / 255); };

function compile(gl: WebGL2RenderingContext, vs: string, fs: string) {
  const sh = (type: number, src: string) => {
    const s = gl.createShader(type)!; gl.shaderSource(s, src); gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) || 'compile');
    return s;
  };
  const p = gl.createProgram()!;
  gl.attachShader(p, sh(gl.VERTEX_SHADER, vs)); gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs)); gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) || 'link');
  return p;
}

async function renderMesh(c: any, shaders: { meshVertex: string; meshFragment: string; vert: string; header: string; missing: string }) {
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl2', { alpha: false, antialias: false, premultipliedAlpha: true, preserveDrawingBuffer: true })!;
  gl.getExtension('EXT_color_buffer_float');
  const { width: w, height: h } = c;
  const tex = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
  const fb = gl.createFramebuffer(); gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
  const depth = gl.createRenderbuffer(); gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
  gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT16, w, h);
  gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
  gl.viewport(0, 0, w, h);
  if (c.obj) {
    const mesh = parseObj(c.obj);
    const p = compile(gl, shaders.meshVertex, shaders.meshFragment);
    const bg = hex2rgb(c.background);
    gl.enable(gl.DEPTH_TEST); gl.depthFunc(gl.LEQUAL); gl.depthMask(true); gl.disable(gl.BLEND);
    gl.clearColor(bg[0]!, bg[1]!, bg[2]!, 1); gl.clearDepth(1); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.useProgram(p);
    const buffer = (data: Float32Array, name: string) => {
      const b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      const loc = gl.getAttribLocation(p, name); gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, 3, gl.FLOAT, false, 0, 0);
    };
    buffer(mesh.positions, 'a_position'); buffer(mesh.normals, 'a_normal');
    const u = (name: string, v: number[]) => { const l = gl.getUniformLocation(p, name); if (v.length === 2) gl.uniform2fv(l, v); else if (v.length === 3) gl.uniform3fv(l, v); else gl.uniform1f(l, v[0]!); };
    u('u_resolution', [w, h]);
    ['u_yaw', 'u_pitch', 'u_distance', 'u_rotationX', 'u_rotationY', 'u_rotationZ', 'u_scale', 'u_roughness', 'u_metalness'].forEach((n, i) => u(n, [c.values[i]]));
    u('u_objectColor', hex2rgb(c.objectColor)); u('u_lightColor', hex2rgb(c.lightColor));
    gl.drawArrays(gl.TRIANGLES, 0, mesh.vertexCount);
  } else {
    const p = compile(gl, shaders.vert, shaders.header + '\n' + shaders.missing);
    gl.disable(gl.DEPTH_TEST); gl.disable(gl.BLEND);
    gl.clearColor(0, 0, 0, 0); gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(p);
    const quad = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(p, 'a_pos'); gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    gl.uniformMatrix3fv(gl.getUniformLocation(p, 'u_m'), false, new Float32Array([w, 0, 0, 0, h, 0, 0, 0, 1]));
    gl.uniform2f(gl.getUniformLocation(p, 'u_res'), w, h);
    gl.uniform4f(gl.getUniformLocation(p, 'u_uv'), 0, 0, 1, 1);
    gl.uniform2f(gl.getUniformLocation(p, 'iResolution'), w, h);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }
  const px = new Float32Array(w * h * 4);
  gl.readPixels(0, 0, w, h, gl.RGBA, gl.FLOAT, px);
  await post(`x/${c.id}/mesh.f32`, new Uint8Array(px.buffer));
  return { id: c.id };
}

(globalThis as any).ThreeRef = { renderCase, renderMesh, version: THREE.REVISION };
