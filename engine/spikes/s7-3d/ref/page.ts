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

/** Pick reference for `pm-geom3d` (S7 deliverable 3). For each time: the production
 *  `runtime.pick` over an NDC grid (with its bounding-volume caches carried across time
 *  changes, as in the editor), and three.js raycasts with the skinned bounding volumes
 *  recomputed for the current pose (the engine's per-frame semantics). Also exports the
 *  camera, each model's glTF-scene-to-world matrix, and skinned bone world matrices. */
async function pickScene(id: string, options: { times: number[]; cols: number; rows: number; ids?: string[] }) {
  const def: SceneDef = await (await fetch(`/corpus/scenes/${id}.json`)).json();
  const assets = await loadAssets(def);
  const runtime = new SceneRuntime();
  const scene = parseScene(def.scene);
  const ids = new Set(options.ids || Object.values(def.scene.objects).filter((o: any) => o.source?.assetId).map((o: any) => o.id));
  const rays: [number, number][] = [];
  for (let j = 0; j < options.rows; j++) for (let i = 0; i < options.cols; i++) rays.push([((i + 0.5) / options.cols) * 2 - 1, 1 - ((j + 0.5) / options.rows) * 2]);
  const frames: any[] = [];
  const now = () => performance.now();
  for (const time of options.times) {
    runtime.sync(scene, (assetId: string) => assets.get(assetId), (prop: any) => prop.v, time);
    runtime.resizeCamera(def.width, def.height);
    runtime.scene.updateMatrixWorld(true);
    const camera = runtime.camera;
    // Production pick: the first call after the time change, then the median of the rest.
    const prod: (string | null)[] = [];
    const t0 = now();
    prod.push(runtime.pick(rays[0]![0], rays[0]![1], ids));
    const first = now() - t0;
    const times: number[] = [], hitTimes: number[] = [];
    for (const [x, y] of rays.slice(1)) { const s = now(); const r = runtime.pick(x, y, ids); const d = now() - s; prod.push(r); times.push(d); if (r) hitTimes.push(d); }
    times.sort((a, b) => a - b);
    hitTimes.sort((a, b) => a - b);
    // Fresh bounding volumes for the current pose.
    const layers: any[] = [];
    for (const layerId of ids) {
      const object = runtime.objects.get(layerId);
      if (!object) continue;
      const gltfScene = object.children[0]?.children[0];
      const skins: any[] = [];
      object.traverse((o: any) => {
        if (o.isSkinnedMesh) { o.boundingSphere = null; o.boundingBox = null; skins.push({ name: o.name, bones: o.skeleton.bones.map((b: any) => [b.name, Array.from(b.matrixWorld.elements)]), bindMatrix: Array.from(o.bindMatrix.elements) }); }
        if (o.isMesh && o.morphTargetInfluences) skins.push({ name: o.name, morph: Array.from(o.morphTargetInfluences) });
      });
      layers.push({ id: layerId, gltfToWorld: gltfScene ? Array.from(gltfScene.matrixWorld.elements) : null, skins });
    }
    const ray = new THREE.Raycaster();
    const hits = rays.map(([x, y]) => {
      ray.setFromCamera(new THREE.Vector2(x, y), camera);
      for (const hit of ray.intersectObjects([...runtime.objects.values()], true)) {
        const o: any = hit.object;
        if (!o.visible || !o.userData.sceneId || !ids.has(o.userData.sceneId)) continue;
        const materials = Array.isArray(o.material) ? o.material : [o.material];
        if (materials.every((m: any) => m.opacity <= 0)) continue;
        let visible = true; for (let p = o; p; p = p.parent) if (!p.visible) visible = false;
        if (!visible) continue;
        return { layer: o.userData.sceneId, object: o.name, kind: o.isPoints ? 'points' : o.isLine ? 'line' : 'mesh', distance: hit.distance, face: hit.faceIndex ?? null, index: hit.index ?? null, instance: hit.instanceId ?? null, point: hit.point.toArray() };
      }
      return null;
    });
    frames.push({
      time, rays, hits, prod, prodFirstMs: first, prodMedianMs: times[times.length >> 1] ?? 0, prodMaxMs: times.at(-1) ?? 0, prodHitMedianMs: hitTimes[hitTimes.length >> 1] ?? 0,
      camera: { projection: Array.from(camera.projectionMatrix.elements), world: Array.from(camera.matrixWorld.elements), near: camera.near, far: camera.far, isOrthographic: (camera as any).isOrthographicCamera === true },
      layers,
    });
  }
  runtime.dispose();
  return { id, width: def.width, height: def.height, ids: [...ids], frames };
}

(globalThis as any).S7.pickScene = pickScene;

/** Blender render snapshot for a corpus scene: a port of `renderSnapshot`'s collection
 *  (core/scene3d/rendering.ts) over the production SceneRuntime, so the spike drives the
 *  shipped Blender worker with the geometry, UVs, materials, lights, camera, depth sets,
 *  and textures the app sends today. Texture and asset bytes come back base64-encoded. */
async function blenderSnapshot(id: string, settings: Record<string, unknown> = {}) {
  const def: SceneDef = await (await fetch(`/corpus/scenes/${id}.json`)).json();
  const loaded = await loadAssets(def);
  const runtime = new SceneRuntime();
  const scene = parseScene(def.scene) as any;
  runtime.sync(scene, (assetId: string) => loaded.get(assetId), (prop: any) => prop.v, def.time);
  runtime.resizeCamera(def.width, def.height);
  runtime.scene.updateMatrixWorld(true);
  const toBase64 = async (blob: Blob) => { const b = new Uint8Array(await blob.arrayBuffer()); let s = ''; for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000)); return btoa(s); };
  const colorHex = (c: any) => '#' + c.getHexString(THREE.SRGBColorSpace);
  const assets: { id: string; name: string; data: string }[] = [];
  const textureIds = new Map<any, string>();
  const textureAsset = async (texture: any): Promise<string | undefined> => {
    if (!texture?.image) return;
    const existing = textureIds.get(texture);
    if (existing) return existing;
    const tid = `texture_${textureIds.size}`, image = texture.image, canvas = document.createElement('canvas');
    canvas.width = image.width; canvas.height = image.height;
    canvas.getContext('2d')!.drawImage(image, 0, 0);
    textureIds.set(texture, tid);
    const blob = await new Promise<Blob>((resolve, reject) => canvas.toBlob(b => (b ? resolve(b) : reject(new Error('toBlob'))), 'image/png'));
    assets.push({ id: tid, name: tid + '.png', data: await toBase64(blob) });
    return tid;
  };
  // Corpus image assets referenced by primitive material maps.
  const used = new Set<string>();
  for (const node of scene.objects) for (const m of [node.material, ...(node.slots || []).map((s: any) => s.material)]) if (m) for (const v of Object.values<string>(m.maps || {})) used.add(v);
  for (const assetId of used) {
    const a = def.assets[assetId];
    if (!a) continue;
    assets.push({ id: assetId, name: `${assetId}.${a.file.split('.').pop()}`, data: await toBase64(await (await fetch(`/corpus/${a.file}`)).blob()) });
  }
  const objects: any[] = [];
  const pending: Promise<void>[] = [];
  for (const node of scene.objects) {
    const root = runtime.objects.get(node.id);
    if (!root) continue;
    const meshes: any[] = [];
    root.traverse((mesh: any) => {
      if (!mesh.isMesh || !mesh.visible) return;
      const g = mesh.geometry, position = g.attributes.position, normal = g.attributes.normal, uv = g.attributes.uv;
      if (!position) return;
      const groups = g.groups.length ? g.groups : [{ start: 0, count: g.index?.count || position.count, materialIndex: 0 }];
      const point = new THREE.Vector3(), direction = new THREE.Vector3();
      for (const group of groups) {
        const positions: number[] = [], normals: number[] = [], uvs: number[] = [];
        const end = Math.min(group.start + group.count, g.index?.count || position.count);
        for (let i = group.start; i < end; i++) {
          const index = g.index ? g.index.getX(i) : i;
          mesh.getVertexPosition(index, point);
          positions.push(point.x, point.y, point.z);
          if (normal) {
            direction.fromBufferAttribute(normal, index);
            if (mesh.isSkinnedMesh) { const origin = new THREE.Vector3().fromBufferAttribute(position, index); direction.add(origin); mesh.applyBoneTransform(index, direction); direction.sub(point); }
            direction.normalize();
            normals.push(direction.x, direction.y, direction.z);
          }
          uvs.push(uv?.getX(index) || 0, uv?.getY(index) || 0);
        }
        const slot = node.slots?.[group.materialIndex || 0];
        let material: any = slot?.material || node.material;
        const matrix = mesh.matrixWorld.elements.slice();
        pending.push((async () => {
          if (node.useSourceMaterials && !slot) {
            const source = (Array.isArray(mesh.material) ? mesh.material[group.materialIndex || 0] : mesh.material);
            const maps: any = {};
            for (const [key, prop] of [['color', 'map'], ['normal', 'normalMap'], ['roughness', 'roughnessMap'], ['metalness', 'metalnessMap'], ['emissive', 'emissiveMap'], ['ao', 'aoMap']] as const) {
              const tid = await textureAsset(source[prop] || undefined);
              if (tid) maps[key] = tid;
            }
            material = { p: { color: { v: colorHex(source.color) }, roughness: { v: source.roughness }, metalness: { v: source.metalness }, emissive: { v: colorHex(source.emissive) }, emissiveIntensity: { v: source.emissiveIntensity }, opacity: { v: source.opacity } }, maps };
          }
          meshes.push({ positions, normals, uvs, materialIndex: group.materialIndex || 0, matrix, material, smooth: true });
        })());
      }
    });
    objects.push({ id: node.id, meshes, castShadow: node.castShadow && scene.environment.shadows, receiveShadow: node.receiveShadow });
  }
  await Promise.all(pending);
  const lights = scene.lights.map((node: any) => {
    const light = runtime.objects.get(node.id) as any;
    return {
      id: node.id, type: node.type, position: light?.position.toArray() || [node.p.x.v, node.p.y.v, node.p.z.v], target: light?.target?.position.toArray() || [0, 0, 0],
      color: node.p.color.v, intensity: node.p.intensity.v, distance: node.p.distance.v, angle: node.p.angle.v, penumbra: node.p.penumbra.v,
      width: node.p.width?.v ?? 1, height: node.p.height?.v ?? 1, castShadow: node.castShadow && scene.environment.shadows,
    };
  });
  const blocks: Record<string, string[]> = {}, backgroundFor: string[] = [];
  for (const set of def.sets) {
    const ordered = [...set].sort((a, b) => cameraDepth(runtime, b) - cameraDepth(runtime, a));
    for (const objectId of set) blocks[objectId] = [...set];
    backgroundFor.push(ordered.at(-1)!);
  }
  const c: any = runtime.camera;
  const snapshot = {
    width: def.width, height: def.height,
    settings: { enabled: true, engine: 'eevee', samples: 64, previewSamples: 16, previewScale: 0.5, denoise: true, device: 'auto', ...settings },
    objects, lights, blocks, backgroundFor,
    environment: Object.fromEntries(Object.entries<any>(scene.environment.p).map(([key, prop]) => [key, prop.v])),
    background: scene.environment.background,
    camera: { matrix: c.matrixWorld.elements.slice(), orthographic: !!c.isOrthographicCamera, orthoHeight: c.isOrthographicCamera ? c.top - c.bottom : 4, fov: c.isPerspectiveCamera ? c.fov : 45, zoom: c.zoom, near: c.near, far: c.far },
  };
  runtime.dispose();
  return { snapshot, assets };
}

(globalThis as any).S7.blenderSnapshot = blenderSnapshot;
