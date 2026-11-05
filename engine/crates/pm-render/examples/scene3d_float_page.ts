// Independent r180 reference for the native scene pass. The production
// SceneRuntime supplies authored objects, lights, cameras and layer grouping;
// this path renders them into an RGBA32F target at eight samples per axis.
import * as THREE from '../../../../apps/desktop/node_modules/three/build/three.module.js';
import { SceneRuntime } from '../../../../apps/desktop/src/renderer/src/core/scene3d/runtime';
import { loadGltf } from '../../../../apps/desktop/src/renderer/src/core/scene3d/import-model';

function editorCamera(e: any) {
  const camera = e.projection === 'orthographic'
    ? new THREE.OrthographicCamera(-3, 3, 3, -3, e.near, e.far)
    : new THREE.PerspectiveCamera(e.fov, 16 / 9, e.near, e.far);
  camera.zoom = e.zoom;
  camera.position.fromArray(e.position);
  camera.up.fromArray(e.up);
  camera.lookAt(new THREE.Vector3().fromArray(e.target));
  camera.updateProjectionMatrix();
  camera.updateMatrixWorld(true);
  return camera;
}

// The pinned capture predates the current SceneRuntime's seventh `look`
// parameter. Apply that authored Editor look on Three.js materials explicitly
// instead of relying on an ignored argument. Restore before the next output.
function applyLook(scene: THREE.Scene, look: { wireframe?: boolean; xray?: boolean }) {
  const saved = new Map<THREE.Material, { wireframe?: boolean; transparent: boolean; opacity: number; depthWrite: boolean }>();
  const triangles: { mesh: THREE.Mesh; geometry: THREE.BufferGeometry; material: THREE.Material | THREE.Material[] }[] = [];
  scene.traverse((child: any) => {
    if (!child.isMesh) return;
    if (look.wireframe) {
      const mesh = child as THREE.Mesh;
      const original = mesh.geometry;
      const geometry = original.index ? original.toNonIndexed() : original.clone();
      const barycentric = new Float32Array(geometry.getAttribute('position').count * 3);
      for (let i = 0; i < barycentric.length / 3; i++) barycentric[i * 3 + i % 3] = 1;
      geometry.setAttribute('referenceBarycentric', new THREE.BufferAttribute(barycentric, 3));
      triangles.push({ mesh, geometry: original, material: mesh.material });
      mesh.geometry = geometry;
      mesh.userData.floatWireframeReference = true;
      const wireframeMaterial = (material: THREE.Material) => {
        const copy = material.clone();
        // Line primitives have undefined screen-space derivatives on Metal.
        // Shade the original triangles with r180's material formula, then
        // retain their one-pixel edges using analytic barycentric coverage.
        // Discard after shading so normal, tangent and bump derivatives have
        // valid neighboring triangle fragments.
        copy.wireframe = false;
        const originalCompile = copy.onBeforeCompile;
        copy.onBeforeCompile = (shader, renderer) => {
          originalCompile.call(copy, shader, renderer);
          shader.vertexShader = shader.vertexShader.replace('void main() {',
            'attribute vec3 referenceBarycentric; varying vec3 vReferenceBarycentric; void main() { vReferenceBarycentric = referenceBarycentric;');
          shader.fragmentShader = shader.fragmentShader.replace('void main() {',
            'varying vec3 vReferenceBarycentric; void main() {');
          if (!shader.fragmentShader.includes('#include <opaque_fragment>'))
            throw new Error(`wireframe reference cannot shade ${material.type}`);
          shader.fragmentShader = shader.fragmentShader.replace('#include <opaque_fragment>',
            'if (all(greaterThan(vReferenceBarycentric, 0.5 * fwidth(vReferenceBarycentric)))) discard;\n#include <opaque_fragment>');
        };
        return copy;
      };
      mesh.material = Array.isArray(mesh.material)
        ? mesh.material.map(wireframeMaterial) : wireframeMaterial(mesh.material);
    }
    for (const material of Array.isArray(child.material) ? child.material : [child.material]) {
      if (!material || saved.has(material)) continue;
      saved.set(material, { wireframe: material.wireframe, transparent: material.transparent,
        opacity: material.opacity, depthWrite: material.depthWrite });
      if (look.xray) { material.transparent = true; material.opacity *= .45; material.depthWrite = false; }
    }
  });
  return () => {
    for (const [material, previous] of saved) Object.assign(material, previous);
    for (const { mesh, geometry, material } of triangles) {
      const temporary = mesh.geometry;
      const temporaryMaterials = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
      mesh.geometry = geometry;
      mesh.material = material;
      delete mesh.userData.floatWireframeReference;
      temporary.dispose();
      for (const m of temporaryMaterials) m.dispose();
    }
  };
}

// A WebGL line remains one framebuffer pixel wide when the target grows.
// For wireframe, points and lines, 8x dimensions would shrink that authored
// width to 1/8 after resolve. Integrate 8x8 subpixel camera jitter at the
// original dimensions instead; every sample is still independent RGBA32F.
function needsJitter(scene: THREE.Scene) {
  let result = false;
  scene.traverse((child: any) => {
    if (child.isLine || child.isPoints || child.isSprite) result = true;
    if (child.isMesh && child.userData.floatWireframeReference) result = true;
  });
  return result;
}

async function renderCase(c: any) {
  const assets = new Map<string, any>();
  for (const [id, name] of Object.entries<string>(c.textures || {})) {
    const bitmap = await createImageBitmap(await (await fetch(`/tex/${name}.png`)).blob());
    assets.set(id, { kind: 'image', bitmap });
  }
  for (const [id, file] of Object.entries<string>(c.models || {})) {
    const blob = await (await fetch(`/models/${encodeURIComponent(file)}`)).blob();
    assets.set(id, { id, kind: 'model', format: /\.gltf$/i.test(file) ? 'gltf' : 'glb',
      blob, ...(await loadGltf(blob)) });
  }
  const runtime = new SceneRuntime();
  const time = c.time ?? 0;
  runtime.sync(c.scene, (id: string) => assets.get(id) ?? null, (p: any) => p.v,
    time, true, c.animationTimes ? (id: string) => c.animationTimes[id] ?? time : undefined);
  if (c.matrices) for (const [id, matrix] of Object.entries<number[]>(c.matrices)) {
    const object = runtime.objects.get(id)!;
    runtime.scene.add(object);
    object.matrixAutoUpdate = false;
    object.matrix.fromArray(matrix);
    object.matrix.decompose(object.position, object.quaternion, object.scale);
    object.rotation.order = 'ZYX';
    object.userData.layerOpacity = c.layers?.[id]?.opacity ?? 1;
    object.userData.layerOccludes = c.layers?.[id]?.occludes ?? true;
  }
  if (c.cameraPose) {
    runtime.camera.position.fromArray(c.cameraPose.position);
    runtime.camera.up.fromArray(c.cameraPose.up);
    runtime.camera.lookAt(new THREE.Vector3().fromArray(c.cameraPose.target));
  }
  runtime.scene.updateMatrixWorld(true);
  // SceneOutput handles the authored display background separately; the
  // direct GPU contract under test is transparent premultiplied scene color.
  runtime.scene.background = null;
  const scale = 8, width = c.width * scale, height = c.height * scale;
  const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: false,
    premultipliedAlpha: true, preserveDrawingBuffer: true });
  renderer.outputColorSpace = THREE.LinearSRGBColorSpace;
  renderer.toneMapping = THREE.NoToneMapping;
  renderer.shadowMap.enabled = c.scene.environment.shadows;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  renderer.setSize(width, height, false);
  const gl = renderer.getContext();
  if (!gl.getExtension('EXT_color_buffer_float')) throw new Error('RGBA32F render target unsupported');
  const target = new THREE.WebGLRenderTarget(width, height, { type: THREE.FloatType,
    format: THREE.RGBAFormat, colorSpace: THREE.LinearSRGBColorSpace,
    depthBuffer: true, samples: 0 });
  const jitterTarget = new THREE.WebGLRenderTarget(c.width, c.height, { type: THREE.FloatType,
    format: THREE.RGBAFormat, colorSpace: THREE.LinearSRGBColorSpace,
    depthBuffer: true, samples: 0 });
  (runtime as any).renderer = renderer;
  try {
    for (let k = 0; k < c.outputs.length; k++) {
      const o = c.outputs[k];
      const camera = o.editorCamera ? editorCamera(o.editorCamera) : runtime.camera;
      // Each channel is averaged independently in premultiplied linear light.
      const averaged = new Float32Array(c.width * c.height * 4);
      const restore = applyLook(runtime.scene, o.look || {});
      const render = (w: number, h: number) => {
        // The scene color stays transparent (the gate composites the
        // authored background after the curve), but r180's transmission
        // target sees the canvas clear: with a background output that is the
        // opaque background, not its half-transparent white fallback.
        const background = o.background && c.scene.environment.background
          ? new THREE.Color(c.scene.environment.background) : null;
        const setRenderTarget = renderer.setRenderTarget;
        if (background) renderer.setRenderTarget = function(rt: any, ...rest: any[]) {
          // Only the transmission target (4× MSAA with mipmaps); shadow maps
          // keep their own clear.
          if (rt && rt.samples === 4 && rt.texture.generateMipmaps) renderer.setClearColor(background, 1);
          return setRenderTarget.call(renderer, rt, ...rest);
        };
        try { renderLayer(w, h); } finally { renderer.setRenderTarget = setRenderTarget; }
      };
      const renderLayer = (w: number, h: number) => {
        if (!o.object) {
          runtime.render(w, h, undefined, undefined, false, camera);
          return;
        }
        // The old runtime disables depth writes for every color draw. An
        // opaque target still needs its own depth test when layer opacity or
        // occlusion excluded it from the shared prepass. Correct that one
        // reference-model artifact without changing its material or shader.
        // Its color draw can now write depth, so rebuild the shared prepass
        // before the next object (and each jittered sample).
        runtime.revision++;
        const originalRender = renderer.render;
        renderer.render = function(scene, view) {
          if (!renderer.autoClear) scene.traverse((child: any) => {
            if (!child.isMesh || child.userData.sceneId !== o.object) return;
            for (const material of Array.isArray(child.material) ? child.material : [child.material]) {
              if (!material.transparent && material.opacity >= .999 && material.depthTest)
                material.depthWrite = true;
            }
          });
          return originalRender.call(renderer, scene, view);
        };
        try { runtime.render(w, h, o.object, new Set(o.depthIds), false, camera); }
        finally { renderer.render = originalRender; }
      };
      try {
        if (needsJitter(runtime.scene)) {
          if (renderer.domElement.width !== c.width || renderer.domElement.height !== c.height)
            renderer.setSize(c.width, c.height, false);
          const pixels = new Float32Array(c.width * c.height * 4);
          for (let j = 0; j < scale; j++) for (let i = 0; i < scale; i++) {
            camera.setViewOffset(c.width, c.height,
              (i + .5) / scale - .5, (j + .5) / scale - .5, c.width, c.height);
            // SceneRuntime owns the shared depth prepass and clears only
            // color between object layers. Clearing here would erase that
            // retained depth while its cache still considers it valid.
            renderer.setRenderTarget(jitterTarget);
            render(c.width, c.height);
            renderer.readRenderTargetPixels(jitterTarget, 0, 0, c.width, c.height, pixels);
            for (let y = 0; y < c.height; y++) for (let x = 0; x < c.width; x++) {
              const at = (y * c.width + x) * 4;
              const from = ((c.height - 1 - y) * c.width + x) * 4;
              for (let channel = 0; channel < 4; channel++) averaged[at + channel] += pixels[from + channel]! / (scale * scale);
            }
          }
          camera.clearViewOffset();
        } else {
          if (renderer.domElement.width !== width || renderer.domElement.height !== height)
            renderer.setSize(width, height, false);
          // r180 sizes its transmission target from the canvas: keep it at
          // the output's size while the shading is supersampled, so
          // transmissionSamplerSize and its roughness lod stay the 1x ones.
          renderer.transmissionResolutionScale = 1 / scale;
          renderer.setRenderTarget(target);
          try { render(width, height); } finally { renderer.transmissionResolutionScale = 1; }
          const pixels = new Float32Array(width * height * 4);
          renderer.readRenderTargetPixels(target, 0, 0, width, height, pixels);
          // WebGL readPixels starts at the bottom row; recorded PNGs start at
          // the top. The Rust gate flips native readback to that order.
          for (let y = 0; y < c.height; y++) for (let x = 0; x < c.width; x++) {
            const at = (y * c.width + x) * 4;
            for (let j = 0; j < scale; j++) for (let i = 0; i < scale; i++) {
              const from = ((height - 1 - y * scale - j) * width + x * scale + i) * 4;
              for (let channel = 0; channel < 4; channel++) averaged[at + channel] += pixels[from + channel]! / (scale * scale);
            }
          }
        }
      } finally {
        camera.clearViewOffset(); restore();
      }
      const nonfinite = averaged.findIndex(value => !Number.isFinite(value));
      if (nonfinite >= 0)
        throw new Error(`${c.id}/${k}: nonfinite RGBA32F reference component ${nonfinite}`);
      const response = await fetch(`/out/${encodeURIComponent(c.id)}/${k}.rgba32f`,
        { method: 'POST', body: averaged.buffer });
      if (!response.ok) throw new Error(`write ${c.id}/${k}: ${response.status}`);
    }
  } finally {
    target.dispose(); jitterTarget.dispose();
    runtime.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
    for (const asset of assets.values()) asset.bitmap?.close();
  }
}

(globalThis as any).Scene3dFloatReference = { renderCase };
