import * as THREE from "three";
import { sourceTime } from "../../legacy/core/retiming";
import { bridge } from "../../kernel/bridge";
import {
  render3DSchema,
  type Render3DSettings,
  type BlenderStatus,
  type BlenderAsset,
} from "../../../../shared/blender";
import {
  compositionRuntime,
  compositionDepthIds,
  compositionOrderedLayers,
} from "./service";
import { compositionScene, layer3DRole } from "./layers";
import { evaluatedRecipe } from "./modeling";
import { viewportCamera, getViewportMode, editorCamera } from "./viewport";

/** Blender's viewport shading: Wireframe and Solid draw natively; Material Preview (EEVEE) and Rendered (the composition's engine) use Blender. */
export type ViewportShading = "wireframe" | "solid" | "material" | "rendered";
export const VIEWPORT_SHADINGS: ViewportShading[] = ["wireframe", "solid", "material", "rendered"];
export type RenderPreviewState = {
  mode: "draft" | "rendered";
  busy: boolean;
  error: string | null;
  engine: "eevee" | "cycles";
  shading: ViewportShading;
};
type Surface = { surface: ImageBitmap; rect: [number, number, number, number] };
type Frame = {
  revision: number;
  camera: string;
  surfaces: Map<string, Surface>;
  version: string;
  preview: boolean;
  width: number;
  height: number;
};
type State = {
  mode: RenderPreviewState["mode"];
  shading: ViewportShading;
  busy: boolean;
  error: string | null;
  listeners: Set<(state: RenderPreviewState) => void>;
  frames: Map<string, Frame>;
  captureKeys: Set<string>;
  timer: ReturnType<typeof setTimeout> | null;
  request: number;
  running: Promise<void> | null;
};
const states = new WeakMap<object, State>();
const state = (PM: any) => {
  let s = states.get(PM);
  if (!s) {
    s = {
      mode: "draft",
      shading: "solid",
      busy: false,
      error: null,
      listeners: new Set(),
      frames: new Map(),
      captureKeys: new Set(),
      timer: null,
      request: 0,
      running: null,
    };
    states.set(PM, s);
  }
  return s;
};
export const renderSettings = (comp: any): Render3DSettings =>
  render3DSchema.parse(comp?.render3d || {});
export const renderState = (PM: any): RenderPreviewState => {
  const s = state(PM);
  const native = PM.EngineConnection?.preview?.state?.();
  return {
    mode: s.mode,
    busy: native ? native.busy || s.busy : s.busy,
    error: native ? native.error ?? s.error : s.error,
    engine: previewEngine(PM) ?? renderSettings(PM.curComp?.() || PM.proj).engine,
    shading: s.shading,
  };
};
/** Material Preview always uses EEVEE; Rendered uses the composition's engine. */
export const previewEngine = (PM: any): "eevee" | null =>
  state(PM).shading === "material" ? "eevee" : null;
export const wireframePreview = (PM: any) => state(PM).shading === "wireframe";
export const blenderRevision = (PM: any) => PM._blenderRevision || 0;
const emit = (PM: any) => {
  PM._blenderRevision = (PM._blenderRevision || 0) + 1;
  for (const fn of state(PM).listeners) fn(renderState(PM));
  PM.invalidate?.("render");
};
/** Mirrors prepareBlenderFrame's controls state while the native engine prepares
 * an async capture. Pixels and Blender supervision stay in the engine. */
export async function engineBlenderCapture<T>(PM: any, capture: () => Promise<T>): Promise<T> {
  const s = state(PM), project = PM.proj;
  s.busy = true;
  s.error = null;
  emit(PM);
  try {
    return await capture();
  } catch (error) {
    if (PM.proj === project) s.error = error instanceof Error ? error.message : String(error);
    throw error;
  } finally {
    s.busy = false;
    emit(PM);
  }
}
export function onRenderState(
  PM: any,
  listener: (value: RenderPreviewState) => void,
) {
  state(PM).listeners.add(listener);
  const offNative = PM.EngineConnection?.preview?.subscribe?.(() => listener(renderState(PM)));
  return () => { state(PM).listeners.delete(listener); offNative?.(); };
}
export function setRenderSettings(
  PM: any,
  patch: Partial<Render3DSettings>,
  meta: any = {},
) {
  const settings = render3DSchema.parse({
    ...renderSettings(PM.proj),
    ...patch,
  });
  return PM.Edit.apply(
    { type: "set_composition", patch: { render3d: settings } },
    { label: "3D render settings", origin: "inspector", ...meta },
  );
}
export async function blenderStatus(): Promise<BlenderStatus> {
  return (
    bridge()?.blender?.status() || {
      available: false,
      error: "Blender rendering requires the desktop app",
    }
  );
}
export function setPreviewMode(PM: any, mode: RenderPreviewState["mode"]) {
  const s = state(PM);
  setShading(PM, mode === "rendered" ? "rendered" : s.shading === "wireframe" ? "wireframe" : "solid");
}
export function setShading(PM: any, shading: ViewportShading) {
  if (!VIEWPORT_SHADINGS.includes(shading)) throw new Error(`Unknown viewport shading "${String(shading)}"`);
  const s = state(PM),
    mode = shading === "material" || shading === "rendered" ? "rendered" : "draft";
  s.shading = shading;
  s.mode = mode;
  s.error = null;
  s.request++;
  if (s.timer) {
    clearTimeout(s.timer);
    s.timer = null;
  }
  emit(PM);
  // The native viewer-state update starts or cancels automatic Blender
  // previews. A toolbar shading change must reach it before any explicit
  // renderFrame request, including Material Preview's EEVEE choice.
  if (!nativePreviewEnabled(PM) && mode === "rendered") schedulePreview(PM);
}
const frameKey = (
  PM: any,
  comp: any,
  time: number,
  preview: boolean,
  width?: number,
  height?: number,
) =>
  `${PM.proj.id}:${comp.compId || comp.id || "root"}:${time}:${preview ? `preview:${previewEngine(PM) || "scene"}` : `output:${width || comp.w}x${height || comp.h}`}`;
const cameraKey = (camera: THREE.Camera) =>
  JSON.stringify([
    camera.matrixWorld.elements,
    camera.projectionMatrix.elements,
  ]);
function chosenCamera(PM: any, time: number, comp: any, preview: boolean) {
  PM.scope.push(comp);
  try {
    const runtime = compositionRuntime(PM, time, comp);
    return preview && comp === PM.proj && getViewportMode(PM) === "editor"
      ? viewportCamera(PM, runtime.camera)
      : runtime.camera;
  } finally {
    PM.scope.pop();
  }
}
export function blenderSurface(
  PM: any,
  layer: any,
  time: number,
  width?: number,
  height?: number,
): {
  surface: ImageBitmap;
  rect: [number, number, number, number];
  version: string;
} | null {
  const s = state(PM),
    comp = PM._renderComposition3D || PM.curComp?.() || PM.proj;
  const preview = !!PM._renderBlenderPreview && !PM.agentFrameCapture;
  const frame = s.frames.get(frameKey(PM, comp, time, preview, width, height));
  if (
    !frame ||
    frame.revision !== (PM._scene3dRevision || 0) ||
    frame.preview !== preview ||
    frame.camera !== cameraKey(chosenCamera(PM, time, comp, preview))
  ) {
    if (preview && s.mode === "rendered" && !s.error) schedulePreview(PM);
    return null;
  }
  if (preview && s.mode !== "rendered") return null;
  const surface = frame.surfaces.get(layer.id);
  return surface ? { ...surface, version: frame.version } : null;
}
/** The new engine draws the viewer (Use New Engine): it renders the paused
 *  Blender preview itself (docs/engine/blender.md), so this one would only
 *  run Blender a second time. */
const nativePreviewEnabled = (PM: any): boolean => !!PM.EngineDoc?.enabled || !!PM.EngineConnection;
const engineDrawsViewer = nativePreviewEnabled;
export function schedulePreview(PM: any) {
  const s = state(PM);
  if (
    s.mode !== "rendered" ||
    s.timer ||
    s.busy ||
    PM.playing ||
    PM.Export?.busy ||
    engineDrawsViewer(PM)
  )
    return;
  s.timer = setTimeout(() => {
    s.timer = null;
    if (s.mode !== "rendered" || s.busy || s.running || engineDrawsViewer(PM)) return;
    const request = ++s.request;
    s.running = renderBlenderPreview(PM)
      .catch((e) => {
        if (request === s.request) {
          s.error = e instanceof Error ? e.message : String(e);
          emit(PM);
        }
      })
      .finally(() => {
        s.running = null;
      });
  }, 250);
}
async function assetBytes(PM: any, id: string): Promise<BlenderAsset> {
  const meta = PM.proj.assets[id];
  if (!meta) throw new Error(`Missing 3D asset: ${id}`);
  const blob = await PM.MediaStore.get(meta);
  if (!blob) throw new Error(`Missing media for ${meta.name}`);
  if (meta.kind === "image") {
    const types: Record<string, string> = {
      "image/png": "png",
      "image/jpeg": "jpg",
      "image/webp": "webp",
    };
    const extension = types[blob.type];
    if (extension)
      return {
        id,
        name: `${id}.${extension}`,
        data: new Uint8Array(await blob.arrayBuffer()),
      };
    const asset = PM.assets.get(id),
      image = asset?.el || (await createImageBitmap(blob));
    const canvas = document.createElement("canvas");
    canvas.width = image.width || image.naturalWidth;
    canvas.height = image.height || image.naturalHeight;
    const ctx = canvas.getContext("2d");
    if (!ctx || !canvas.width || !canvas.height)
      throw new Error(`Could not convert texture ${meta.name}`);
    ctx.drawImage(image, 0, 0);
    const converted = await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob(
        (b) =>
          b ? resolve(b) : reject(new Error("Could not convert texture")),
        "image/png",
      ),
    );
    if (!asset?.el) image.close?.();
    return {
      id,
      name: `${id}.png`,
      data: new Uint8Array(await converted.arrayBuffer()),
    };
  }
  return {
    id,
    name: meta.name,
    data: new Uint8Array(await blob.arrayBuffer()),
  };
}
function colorHex(c: THREE.Color) {
  return "#" + c.getHexString(THREE.SRGBColorSpace);
}

/** A model's own material as the Blender worker's Principled surface takes it.
 *  glTF loads as MeshStandardMaterial or MeshPhysicalMaterial; unlit glTF
 *  (KHR_materials_unlit) loads as MeshBasicMaterial, which has no emissive,
 *  roughness, or metalness. An unlit surface shows its colour whatever the
 *  lighting, so it becomes a black, fully rough base with that colour (or its
 *  colour map, which then wins over a tint) as emission at strength 1. Any
 *  other material type keeps the fields it has, with the worker's defaults
 *  for the rest. */
export async function sourceMaterialSnapshot(
  source: THREE.Material | undefined,
  textureAsset: (texture: THREE.Texture | undefined) => Promise<string | undefined>,
) {
  const any = (source || {}) as any;
  const maps: Record<string, string> = {};
  const opacity = Number.isFinite(any.opacity) ? any.opacity : 1;
  if (any.isMeshBasicMaterial) {
    const map = await textureAsset(any.map || undefined);
    if (map) maps.emissive = map;
    return {
      p: {
        color: { v: "#000000" },
        roughness: { v: 1 },
        metalness: { v: 0 },
        emissive: { v: map ? "#FFFFFF" : any.color?.isColor ? colorHex(any.color) : "#FFFFFF" },
        emissiveIntensity: { v: 1 },
        opacity: { v: opacity },
      },
      maps,
    };
  }
  for (const [key, prop] of [
    ["color", "map"],
    ["normal", "normalMap"],
    ["roughness", "roughnessMap"],
    ["metalness", "metalnessMap"],
    ["emissive", "emissiveMap"],
    ["ao", "aoMap"],
  ] as const) {
    const id = await textureAsset(any[prop] || undefined);
    if (id) maps[key] = id;
  }
  const number = (value: unknown, fallback: number) =>
    typeof value === "number" && Number.isFinite(value) ? value : fallback;
  return {
    p: {
      color: { v: any.color?.isColor ? colorHex(any.color) : "#FFFFFF" },
      roughness: { v: number(any.roughness, 1) },
      metalness: { v: number(any.metalness, 0) },
      emissive: { v: any.emissive?.isColor ? colorHex(any.emissive) : "#000000" },
      emissiveIntensity: { v: number(any.emissiveIntensity, 1) },
      opacity: { v: opacity },
    },
    maps,
  };
}

/** Snapshot evaluated geometry and world matrices; Blender never owns the project or its timeline. */
export async function renderSnapshot(
  PM: any,
  time: number,
  comp: any,
  options: { preview?: boolean; width?: number; height?: number } = {},
) {
  const collect = () => {
    PM.scope.push(comp);
    try {
      const preview = !!options.preview,
        runtime = compositionRuntime(PM, time, comp),
        scene = compositionScene(PM, time, comp),
        camera = chosenCamera(PM, time, comp, preview),
        settings = {
          ...renderSettings(comp),
          ...(preview && previewEngine(PM) ? { engine: previewEngine(PM)! } : {}),
        };
      const ids = new Set<string>(
        scene.objects.flatMap((node) => {
          const materials = node.slots.length
            ? node.slots.map((slot) => slot.material)
            : [node.material];
          return materials.flatMap((material) =>
            [
              ...Object.values(material.maps),
              material.shader?.source?.assetId,
              ...(material.shader?.graph?.nodes || []).map(
                (node) => node.image,
              ),
            ].filter((id): id is string => !!id),
          );
        }),
      );
      const assets: BlenderAsset[] = [],
        pendingMaterials: Promise<void>[] = [];
      const textureIds = new Map<THREE.Texture, string>();
      const textureAsset = async (
        texture: THREE.Texture | undefined,
      ): Promise<string | undefined> => {
        if (!texture?.image) return;
        const existing = textureIds.get(texture);
        if (existing) return existing;
        const id = `texture_${textureIds.size}`,
          image = texture.image,
          canvas = document.createElement("canvas");
        canvas.width = image.width;
        canvas.height = image.height;
        const ctx = canvas.getContext("2d");
        if (!ctx || !canvas.width || !canvas.height)
          throw new Error("Could not snapshot the model texture");
        ctx.drawImage(image, 0, 0);
        textureIds.set(texture, id);
        const blob = await new Promise<Blob>((resolve, reject) =>
          canvas.toBlob(
            (b) =>
              b ? resolve(b) : reject(new Error("Could not snapshot texture")),
            "image/png",
          ),
        );
        assets.push({
          id,
          name: id + ".png",
          data: new Uint8Array(await blob.arrayBuffer()),
        });
        textureIds.set(texture, id);
        return id;
      };
      const objects: any[] = [];
      for (const node of scene.objects) {
        const root = runtime.objects.get(node.id),
          layer = comp.layers.find((l: any) => l.id === node.id);
        if (!root || !layer) continue;
        if (PM.worldOpacity && PM.worldOpacity(layer, time) <= 0.001) continue;
        const meshes: any[] = [],
          pending: Promise<void>[] = [];
        root.traverse((mesh: any) => {
          if (!mesh.isMesh || !mesh.visible) return;
          const g = mesh.geometry,
            position = g.attributes.position,
            normal = g.attributes.normal,
            uv = g.attributes.uv;
          if (!position) return;
          const groups = g.groups.length
            ? g.groups
            : [
                {
                  start: 0,
                  count: g.index?.count || position.count,
                  materialIndex: 0,
                },
              ];
          const point = new THREE.Vector3(),
            direction = new THREE.Vector3();
          for (const group of groups) {
            const positions: number[] = [],
              normals: number[] = [],
              uvs: number[] = [];
            const end = Math.min(
              group.start + group.count,
              g.index?.count || position.count,
            );
            for (let i = group.start; i < end; i++) {
              const index = g.index ? g.index.getX(i) : i;
              mesh.getVertexPosition(index, point);
              positions.push(point.x, point.y, point.z);
              if (normal) {
                direction.fromBufferAttribute(normal, index);
                if (mesh.isSkinnedMesh) {
                  const origin = new THREE.Vector3().fromBufferAttribute(
                    position,
                    index,
                  );
                  direction.add(origin);
                  mesh.applyBoneTransform(index, direction);
                  direction.sub(point);
                }
                direction.normalize();
                normals.push(direction.x, direction.y, direction.z);
              }
              uvs.push(uv?.getX(index) || 0, uv?.getY(index) || 0);
            }
            const slot = node.slots?.[group.materialIndex || 0];
            let material: any = slot?.material || node.material;
            const matrix = mesh.matrixWorld.elements.slice();
            pending.push(
              (async () => {
                if (node.useSourceMaterials && !slot) {
                  const source = (
                    Array.isArray(mesh.material)
                      ? mesh.material[group.materialIndex || 0]
                      : mesh.material
                  ) as THREE.Material;
                  material = await sourceMaterialSnapshot(source, textureAsset);
                }
                meshes.push({
                  positions,
                  normals,
                  uvs,
                  materialIndex: group.materialIndex || 0,
                  matrix,
                  material,
                  smooth: true,
                });
              })(),
            );
          }
        });
        pendingMaterials.push(...pending);
        const generation = layer.d?.data?.object?.generation,
          group = generation ? PM.L(generation.groupId) : null;
        const recipe = group?.d?.modeling
          ? evaluatedRecipe(PM, group, time)
          : null;
        const generated =
          recipe && (recipe.kind !== "mesh" || recipe.mesh)
            ? {
                recipe,
                part: layer.d.data.object.blender?.object || generation.partId,
              }
            : undefined;
        objects.push({
          id: node.id,
          meshes,
          castShadow: node.castShadow && scene.environment.shadows,
          receiveShadow: node.receiveShadow,
          generated,
        });
      }
      const lights = scene.lights.map((node) => {
        const light = runtime.objects.get(node.id) as any;
        return {
          id: node.id,
          type: node.type,
          position: light?.position.toArray() || [
            node.p.x!.v,
            node.p.y!.v,
            node.p.z!.v,
          ],
          target: light?.target?.position.toArray() || [0, 0, 0],
          color: node.p.color!.v,
          intensity: node.p.intensity!.v,
          distance: node.p.distance!.v,
          angle: node.p.angle!.v,
          penumbra: node.p.penumbra!.v,
          width: node.p.width?.v ?? 1,
          height: node.p.height?.v ?? 1,
          castShadow: node.castShadow && scene.environment.shadows,
        };
      });
      const previous = PM._renderComposition3D;
      PM._renderComposition3D = comp;
      const blocks: any = {},
        backgroundFor: string[] = [];
      try {
        for (const object of objects) {
          const layer = comp.layers.find((l: any) => l.id === object.id),
            ids = compositionDepthIds(PM, layer, time);
          blocks[object.id] = [...ids];
          const bottom = compositionOrderedLayers(
            PM,
            comp.layers.filter((l: any) => l.type !== "group"),
            time,
          )
            .filter((l) => ids.has(l.id))
            .at(-1)?.id;
          if (bottom === object.id) backgroundFor.push(object.id);
        }
      } finally {
        PM._renderComposition3D = previous;
      }
      const c = camera as THREE.PerspectiveCamera & THREE.OrthographicCamera;
      const data = {
        assets,
        snapshot: {
          width: Math.max(
            2,
            Math.round(
              (options.width || comp.w) * (preview ? settings.previewScale : 1),
            ),
          ),
          height: Math.max(
            2,
            Math.round(
              (options.height || comp.h) *
                (preview ? settings.previewScale : 1),
            ),
          ),
          settings: {
            ...settings,
            samples: preview ? settings.previewSamples : settings.samples,
          },
          objects,
          lights,
          blocks,
          backgroundFor,
          environment: Object.fromEntries(
            Object.entries<any>(scene.environment.p).map(([key, prop]) => [
              key,
              prop.v,
            ]),
          ),
          background: scene.environment.background,
          camera: {
            matrix: camera.matrixWorld.elements.slice(),
            orthographic: !!c.isOrthographicCamera,
            orthoHeight: c.isOrthographicCamera ? c.top - c.bottom : 4,
            fov: c.isPerspectiveCamera ? c.fov : 45,
            zoom: c.zoom,
            near: c.near,
            far: c.far,
          },
        },
        camera: cameraKey(camera),
      };
      return { data, ids, pendingMaterials };
    } finally {
      PM.scope.pop();
    }
  };
  const { data, ids, pendingMaterials } = collect();
  const assetsPromise = Promise.all([...ids].map((id) => assetBytes(PM, id)));
  const [assets] = await Promise.all([
    assetsPromise,
    Promise.all(pendingMaterials),
  ]);
  data.assets.push(...assets);
  return data;
}
// Cache only the occupied rectangle: twenty shutter samples should not retain
// twenty full composition-sized images for each small model part.
async function cropSurface(blob: Blob): Promise<Surface> {
  const source = await createImageBitmap(blob, {
    premultiplyAlpha: "none",
    colorSpaceConversion: "none",
  });
  try {
    const canvas = new OffscreenCanvas(source.width, source.height),
      ctx = canvas.getContext("2d", { willReadFrequently: true })!;
    ctx.drawImage(source, 0, 0);
    const pixels = ctx.getImageData(0, 0, source.width, source.height).data;
    let x0 = source.width,
      y0 = source.height,
      x1 = -1,
      y1 = -1;
    for (let y = 0; y < source.height; y++)
      for (let x = 0; x < source.width; x++)
        if (pixels[(y * source.width + x) * 4 + 3]) {
          x0 = Math.min(x0, x);
          x1 = Math.max(x1, x);
          y0 = Math.min(y0, y);
          y1 = Math.max(y1, y);
        }
    if (x1 < 0)
      return {
        surface: await createImageBitmap(source, 0, 0, 1, 1),
        rect: [0, 0, 1 / source.width, 1 / source.height],
      };
    x0 = Math.max(0, x0 - 2);
    y0 = Math.max(0, y0 - 2);
    x1 = Math.min(source.width - 1, x1 + 2);
    y1 = Math.min(source.height - 1, y1 + 2);
    return {
      surface: await createImageBitmap(
        source,
        x0,
        y0,
        x1 - x0 + 1,
        y1 - y0 + 1,
        { premultiplyAlpha: "none", colorSpaceConversion: "none" },
      ),
      rect: [
        x0 / source.width,
        y0 / source.height,
        (x1 - x0 + 1) / source.width,
        (y1 - y0 + 1) / source.height,
      ],
    };
  } finally {
    source.close();
  }
}

/** A paused rendered preview includes nested compositions through their own cameras. */
export async function renderBlenderPreview(PM: any) {
  if (nativePreviewEnabled(PM)) {
    if (!PM.EngineConnection?.preview) throw Object.assign(new Error("The native preview controller is unavailable"), { name: "EnginePreviewUnavailable" });
    state(PM).error = null;
    const session = PM.EngineConnection.session?.();
    // A nested 3D composition can render with Blender while the open
    // composition has no direct 3D layers (and thus no 3D viewer override).
    const comp = PM.curComp?.() || PM.proj;
    const cacheOnly = !!PM.playing || state(PM).mode === "draft" ||
      !comp?.layers?.some((layer: any) => layer3DRole(layer)) ||
      !session || session.headless || session.route !== "engine";
    if (!cacheOnly) return PM.EngineConnection.preview.render();
    const camera = getViewportMode(PM) === "editor" ? editorCamera(PM) : null;
    camera?.updateMatrixWorld?.(true);
    const engineCamera = camera ? {
      view: Array.from(camera.matrixWorldInverse.elements as ArrayLike<number>),
      projection: camera instanceof THREE.OrthographicCamera
        ? { kind: "orthographic" as const, halfHeight: 3 / camera.zoom, near: camera.near, far: camera.far }
        : { kind: "perspective" as const, fovYDegrees: camera.fov, near: camera.near, far: camera.far },
    } : null;
    return PM.EngineConnection.preview.render({
      cacheOnly: true,
      size: { width: Math.max(1, Math.round(Number(comp?.w) || 1)), height: Math.max(1, Math.round(Number(comp?.h) || 1)) },
      camera: engineCamera,
    });
  }
  const visited = new Set<string>(),
    project = PM.proj;
  const visit = async (comp: any, time: number, depth: number) => {
    if (depth > 8) throw new Error("Composition nesting is too deep");
    const key = `${comp.compId || comp.id}:${time}`;
    if (visited.has(key)) return;
    visited.add(key);
    let children: Array<{ comp: any; time: number }>;
    PM.scope.push(comp);
    try {
      children = comp.layers
        .filter(
          (layer: any) => layer.type === "precomp" && PM.active(layer, time),
        )
        .flatMap((layer: any) => {
          const sub =
            PM.compOf?.(layer) ||
            comp.comps?.[layer.d.comp] ||
            project.comps?.[layer.d.comp];
          return sub ? [{ comp: sub, time: sourceTime(PM, layer, time) }] : [];
        });
    } finally {
      PM.scope.pop();
    }
    await prepareBlenderFrame(PM, time, comp, { preview: true });
    if (PM.proj !== project) return;
    for (const child of children!)
      await visit(child.comp, child.time, depth + 1);
  };
  await visit(PM.curComp?.() || project, PM.time, 0);
}
export function beginBlenderCapture(PM: any) {
  state(PM).captureKeys.clear();
}
export async function prepareBlenderFrame(
  PM: any,
  time: number,
  comp = PM.proj,
  options: {
    preview?: boolean;
    width?: number;
    height?: number;
    retain?: boolean;
  } = {},
): Promise<void> {
  if (!comp.layers.some((l: any) => layer3DRole(l) === "object")) return;
  const settings = renderSettings(comp);
  if (!settings.enabled && !options.preview) return;
  const s = state(PM),
    revision = PM._scene3dRevision || 0,
    project = PM.proj,
    key = frameKey(
      PM,
      comp,
      time,
      !!options.preview,
      options.width,
      options.height,
    ),
    camera = cameraKey(chosenCamera(PM, time, comp, !!options.preview));
  if (options.retain) s.captureKeys.add(key);
  const cached = s.frames.get(key);
  if (
    cached &&
    cached.revision === revision &&
    cached.camera === camera &&
    cached.preview === !!options.preview
  )
    return;
  const host = bridge()?.blender;
  if (!host) throw new Error("Blender rendering requires the desktop app");
  s.busy = true;
  s.error = null;
  emit(PM);
  try {
    const data = await renderSnapshot(PM, time, comp, options),
      result = await host.run({
        id: `render_${crypto.randomUUID().replaceAll("-", "")}`,
        operation: "render",
        snapshot: data.snapshot,
        assets: data.assets,
      });
    const surfaces = new Map<string, Surface>();
    try {
      let bytes = 0;
      for (const file of result.files) {
        const image = await cropSurface(
          new Blob([new Uint8Array(file.data)], { type: "image/png" }),
        );
        surfaces.set(file.id, image);
        bytes += image.surface.width * image.surface.height * 4;
        if (bytes > 512 * 1024 * 1024)
          throw new Error(
            "Rendered layers exceed the memory budget. Lower output resolution or simplify the composition.",
          );
      }
    } catch (error) {
      for (const image of surfaces.values()) image.surface.close();
      throw error;
    }
    if (
      PM.proj !== project ||
      (PM._scene3dRevision || 0) !== revision ||
      (options.preview &&
        cameraKey(chosenCamera(PM, time, comp, true)) !== data.camera)
    ) {
      for (const image of surfaces.values()) image.surface.close();
      return;
    }
    for (const image of cached?.surfaces.values() || []) image.surface.close();
    s.frames.set(key, {
      revision,
      camera: data.camera,
      surfaces,
      version: `blender_${crypto.randomUUID()}`,
      preview: !!options.preview,
      width: data.snapshot.width,
      height: data.snapshot.height,
    });
    const bytes = () =>
      [...s.frames.values()].reduce(
        (sum, f) =>
          sum +
          [...f.surfaces.values()].reduce(
            (n, img) => n + img.surface.width * img.surface.height * 4,
            0,
          ),
        0,
      );
    while (
      s.frames.size > 96 ||
      (bytes() > 512 * 1024 * 1024 && s.frames.size > 1)
    ) {
      const oldest = [...s.frames.keys()].find(
        (key) => !s.captureKeys.has(key),
      );
      if (!oldest) {
        if (bytes() > 512 * 1024 * 1024)
          throw new Error(
            "Motion blur frames exceed the memory budget. Lower output resolution or shutter samples.",
          );
        break;
      }
      const frame = s.frames.get(oldest)!;
      for (const image of frame.surfaces.values()) image.surface.close();
      s.frames.delete(oldest);
    }
  } catch (error) {
    if (PM.proj === project) {
      s.error = error instanceof Error ? error.message : String(error);
    }
    throw error;
  } finally {
    s.busy = false;
    emit(PM);
  }
}
export async function cancelBlender(PM: any) {
  const s = state(PM);
  s.request++;
  s.mode = "draft";
  s.shading = s.shading === "wireframe" ? "wireframe" : "solid";
  s.error = null;
  if (s.timer) clearTimeout(s.timer);
  s.timer = null;
  emit(PM);
  // The window's Blender worker runs scene3d.model jobs (Regenerate, .blend
  // import) with the engine on too, so cancelling always stops it, as today.
  const worker = bridge()?.blender?.cancel();
  if (nativePreviewEnabled(PM)) {
    await Promise.all([worker, PM.EngineConnection?.preview ? cancelNativePreview(PM.EngineConnection.preview) : Promise.resolve()]);
  } else {
    await worker;
  }
}
/** Cancels the interactive engine's preview. Without a native viewer
    session (no mounted viewer, a route still pending) there is no native
    preview to stop, so the cancel resolves as today's does. */
async function cancelNativePreview(preview: { cancel(): Promise<void> }) {
  try {
    await preview.cancel();
  } catch (error) {
    const name = (error as Error | null)?.name;
    // Unavailable: no session to cancel. Cancelled: the session restarted
    // or a newer control replaced this one, which also ends the preview.
    if (name !== "EnginePreviewUnavailable" && name !== "EnginePreviewCancelled") throw error;
  }
}
export function disposeBlenderFrames(PM: any) {
  const s = states.get(PM);
  if (!s) return;
  if (s.timer) clearTimeout(s.timer);
  for (const frame of s.frames.values())
    for (const image of frame.surfaces.values()) image.surface.close();
  s.frames.clear();
  s.captureKeys.clear();
  s.mode = "draft";
  s.shading = "solid";
  s.error = null;
  s.timer = null;
  s.request++;
  emit(PM);
}
