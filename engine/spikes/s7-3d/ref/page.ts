// Reference page: renders corpus scenes with the production SceneRuntime (three.js r180)
// the way the compositor does (one render per 3D object layer with the shared depth
// prepass, plus a whole-scene render), records the WebGL2 command stream, and produces
// classification masks (object ids, MSAA-dependent pixels, shadow penumbrae).
import * as THREE from '../../../../apps/desktop/node_modules/three/build/three.module.js';
import { SceneRuntime } from '../../../../apps/desktop/src/renderer/src/core/scene3d/runtime';
import { loadGltf } from '../../../../apps/desktop/src/renderer/src/core/scene3d/import-model';
import { parseScene } from '../../../../apps/desktop/src/renderer/src/core/scene3d/schema';
import { recorder, shaderDebug } from './recorder';

type SceneDef = { id: string; width: number; height: number; time: number; scene: any; assets: Record<string, { kind: string; file: string }>; sets: string[][]; opacity?: Record<string, number> };

async function post(path: string, body: BodyInit, query: Record<string, string | number> = {}) {
  const q = new URLSearchParams(Object.entries(query).map(([k, v]) => [k, String(v)])).toString();
  const response = await fetch(`/out/${path}${q ? `?${q}` : ''}`, { method: 'POST', body });
  if (!response.ok) throw new Error(`POST ${path} failed: ${response.status}`);
}

/** Reads the default framebuffer. Never rebinds on the recorded context, so three.js's
 *  cached GL state stays true (it leaves the canvas bound after rendering to it). */
function readCanvas(gl: WebGL2RenderingContext, w: number, h: number, rebind = false): Uint8Array {
  const pixels = new Uint8Array(w * h * 4);
  if (rebind) gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
  return pixels;
}

async function loadAssets(def: SceneDef) {
  const assets = new Map<string, any>();
  for (const [id, a] of Object.entries(def.assets)) {
    const blob = await (await fetch(`/corpus/${a.file}`)).blob();
    // Production: images are createImageBitmap(blob); models go through loadGltf.
    if (a.kind === 'image') assets.set(id, { id, kind: 'image', el: await createImageBitmap(blob) });
    else assets.set(id, { id, kind: 'model', format: 'glb', blob, ...(await loadGltf(new Blob([blob], { type: 'model/gltf-binary' }))) });
  }
  return assets;
}

function cameraDepth(runtime: SceneRuntime, id: string): number {
  const object = runtime.objects.get(id);
  if (!object) return 0;
  return object.getWorldPosition(new THREE.Vector3()).applyMatrix4(runtime.camera.matrixWorldInverse).z;
}

/** Drawables in traversal order; index + 1 is the id written into the id mask. */
function drawables(scene: THREE.Scene): { object: any; owner: string; name: string; kind: string }[] {
  const list: { object: any; owner: string; name: string; kind: string }[] = [];
  scene.traverse((o: any) => {
    if (!(o.isMesh || o.isPoints || o.isLine) || !o.visible) return;
    let visible = true;
    for (let p = o.parent; p; p = p.parent) if (!p.visible) visible = false;
    if (!visible) return;
    list.push({ object: o, owner: o.userData.sceneId, name: o.name || o.parent?.name || '', kind: o.isMesh ? (o.isSkinnedMesh ? 'skinned' : o.isInstancedMesh ? 'instanced' : 'mesh') : o.isPoints ? 'points' : 'line' });
  });
  return list;
}

let maskRenderer: THREE.WebGLRenderer | null = null;
function masks(runtime: SceneRuntime, def: SceneDef, exposure: number, shadows: boolean) {
  const { width: w, height: h } = def;
  maskRenderer ??= new THREE.WebGLRenderer({ alpha: true, antialias: false, premultipliedAlpha: true, preserveDrawingBuffer: true });
  const r = maskRenderer;
  r.setSize(w, h, false);
  r.setClearColor(0, 0);
  const scene = runtime.scene, camera = runtime.camera;
  runtime.resizeCamera(w, h);
  const gl = r.getContext() as WebGL2RenderingContext;

  // 1. Whole scene without MSAA, same settings: the difference to the MSAA render marks
  //    every pixel whose value depends on multisampling.
  r.outputColorSpace = THREE.SRGBColorSpace; r.toneMapping = THREE.ACESFilmicToneMapping; r.toneMappingExposure = exposure;
  r.shadowMap.enabled = shadows; r.shadowMap.type = THREE.PCFSoftShadowMap;
  r.render(scene, camera);
  const noAA = readCanvas(gl, w, h, true);

  // 1b. The same per-layer passes as the output, without MSAA: each layer output gets its
  //     own MSAA-edge mask (a layer's visible edges differ from the whole scene's).
  const layers: { name: string; pixels: Uint8Array }[] = [];
  {
    const own = (runtime as any).renderer;
    (runtime as any).renderer = r;
    try {
      runtime.render(w, h); // a whole-scene pass resets the shared depth-prepass cache
      for (const set of def.sets) {
        const ids = new Set(set);
        const ordered = [...set].sort((a, b) => cameraDepth(runtime, b) - cameraDepth(runtime, a));
        const bottom = ordered.at(-1);
        for (const objectId of [...ordered].reverse()) {
          runtime.render(w, h, objectId, ids, objectId === bottom);
          layers.push({ name: `layer-${objectId}`, pixels: readCanvas(gl, w, h, true) });
        }
      }
    } finally {
      (runtime as any).renderer = own;
    }
  }
  r.setClearColor(0, 0); r.autoClear = true; r.shadowMap.autoUpdate = true;

  // 2. Shadow factor: ShadowMaterial alpha = 1 - shadow mask; 0 < alpha < 1 is penumbra.
  const background = scene.background;
  scene.background = null;
  const shadowMaterial = new THREE.ShadowMaterial({ color: 0x000000, opacity: 1 });
  scene.overrideMaterial = shadowMaterial;
  r.toneMapping = THREE.NoToneMapping;
  r.render(scene, camera);
  const shadow = readCanvas(gl, w, h, true);
  scene.overrideMaterial = null;
  shadowMaterial.dispose();

  // 3. Drawable ids, flat colors without MSAA or tone mapping.
  const list = drawables(scene);
  const saved = list.map(d => d.object.material);
  const made: THREE.Material[] = [];
  list.forEach((d, i) => {
    const k = i + 1, color = new THREE.Color().setRGB((k & 255) / 255, ((k >> 8) & 255) / 255, ((k >> 16) & 255) / 255, THREE.LinearSRGBColorSpace);
    const m = d.kind === 'points' ? new THREE.PointsMaterial({ color, size: 1, sizeAttenuation: false })
      : d.kind === 'line' ? new THREE.LineBasicMaterial({ color })
      : new THREE.MeshBasicMaterial({ color, side: THREE.DoubleSide, wireframe: !!d.object.material?.wireframe });
    if (Array.isArray(d.object.material)) { d.object.material = d.object.material.map(() => m); } else d.object.material = m;
    made.push(m);
  });
  r.outputColorSpace = THREE.LinearSRGBColorSpace;
  r.shadowMap.enabled = false;
  r.render(scene, camera);
  const ids = readCanvas(gl, w, h, true);
  list.forEach((d, i) => { d.object.material = saved[i]; });
  for (const m of made) m.dispose();
  scene.background = background;
  return { noAA, layers, shadow, ids, list: list.map((d, i) => ({ id: i + 1, owner: d.owner, name: d.name, kind: d.kind })) };
}

async function renderScene(id: string, options: { trace?: boolean; tag?: string; fsTail?: string; fsMatch?: string } = {}) {
  shaderDebug.tail = options.fsTail || '';
  if (options.fsMatch) shaderDebug.match = options.fsMatch;
  const def: SceneDef = await (await fetch(`/corpus/scenes/${id}.json`)).json();
  const tag = options.tag || 'ref';
  const assets = await loadAssets(def);
  if (options.trace !== false) recorder.arm();
  const runtime = new SceneRuntime();
  const scene = parseScene(def.scene);
  runtime.sync(scene, (assetId: string) => assets.get(assetId), (prop: any) => prop.v, def.time);
  for (const [objectId, opacity] of Object.entries(def.opacity || {})) {
    const object = runtime.objects.get(objectId);
    if (object) object.userData.layerOpacity = opacity;
  }
  const { width: w, height: h } = def;
  runtime.resizeCamera(w, h);
  runtime.scene.updateMatrixWorld(true);
  const outputs: any[] = [];
  let gl: WebGL2RenderingContext | null = null;
  for (const set of def.sets) {
    const ids = new Set(set);
    // compositionOrderedLayers: models sort by camera depth; the bottom layer draws the background.
    const ordered = [...set].sort((a, b) => cameraDepth(runtime, b) - cameraDepth(runtime, a));
    const bottom = ordered.at(-1);
    for (const objectId of [...ordered].reverse()) {
      const name = `layer-${objectId}`;
      recorder.mark(name, { object: objectId, depthIds: set, background: objectId === bottom });
      const canvas = runtime.render(w, h, objectId, ids, objectId === bottom);
      gl ??= (runtime as any).renderer.getContext();
      const on = recorder.on; recorder.on = false;
      const pixels = readCanvas(gl!, w, h);
      recorder.on = on;
      await post(`${tag}/${id}/${name}.rgba`, pixels, { w, h });
      outputs.push({ name, object: objectId, depthIds: set, background: objectId === bottom, canvas: `${canvas.width}x${canvas.height}` });
    }
  }
  recorder.mark('full', {});
  runtime.render(w, h);
  {
    const on = recorder.on; recorder.on = false;
    gl ??= (runtime as any).renderer.getContext();
    await post(`${tag}/${id}/full.rgba`, readCanvas(gl!, w, h), { w, h });
    recorder.on = on;
  }
  outputs.push({ name: 'full' });
  if (options.trace !== false) {
    const { calls, blob } = recorder.take();
    await post(`${tag}/${id}/trace.json`, JSON.stringify(calls));
    await post(`${tag}/${id}/trace.bin`, blob);
  }
  const exposure = def.scene.environment?.p?.exposure ?? 1;
  const shadows = def.scene.environment?.shadows !== false;
  const m = masks(runtime, def, typeof exposure === 'object' ? exposure.v : exposure, shadows);
  await post(`${tag}/${id}/mask-noaa.rgba`, m.noAA, { w, h });
  for (const layer of m.layers) await post(`${tag}/${id}/mask-noaa-${layer.name}.rgba`, layer.pixels, { w, h });
  await post(`${tag}/${id}/mask-shadow.rgba`, m.shadow, { w, h });
  await post(`${tag}/${id}/mask-ids.rgba`, m.ids, { w, h });
  const info = {
    id, width: w, height: h, time: def.time, outputs, drawables: m.list, warnings: runtime.warnings,
    renderer: gl ? (() => { const e = gl!.getExtension('WEBGL_debug_renderer_info'); return e ? gl!.getParameter(e.UNMASKED_RENDERER_WEBGL) : 'unknown'; })() : 'none',
    samples: gl?.getParameter(gl.SAMPLES), three: THREE.REVISION,
  };
  await post(`${tag}/${id}/info.json`, JSON.stringify(info, null, 1));
  runtime.dispose();
  for (const a of assets.values()) if (a.el?.close) a.el.close();
  return info;
}

(globalThis as any).S7 = { renderScene, THREE };
