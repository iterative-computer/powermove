import type { StudioScene } from '../studio/extract';
import type { StudioView } from '../studio/gpu';

/**
 * Renderer boundary shared by the page and the native Studio process.
 * JSON field names and vectors match pm_studio::scene::SceneDocument.
 * This is evaluated render data; the project's layers/channels stay unchanged.
 */
export type StudioVec3 = { x: number; y: number; z: number };
export type StudioMesh = {
  positions: StudioVec3[]; normals: StudioVec3[];
  uvs: [number, number][]; indices: [number, number, number][];
};
export type StudioMaterial = {
  base: StudioVec3; metallic: number; roughness: number; coat: number;
  coat_roughness: number; transmission: number; ior: number; specular: number;
  emission: StudioVec3; screen: boolean; sheen: number; film_nm: number;
  dispersion: number; droplets: number; anisotropy: number; subsurface: number;
};
export type StudioCameraDescription = {
  position: StudioVec3; target: StudioVec3; fov: number; f_stop: number;
  focus: StudioVec3 | null; focal_mm: number;
};
export type StudioSoftbox = { center: StudioVec3; u: StudioVec3; v: StudioVec3; radiance: StudioVec3 };
export type StudioAnalyticLight = { pos: number[]; u: number[]; v: number[]; radiance: number[] };
export type StudioObjectIdentity = {
  layer: string; part: string | null; cast_shadow: boolean; receive_shadow: boolean;
  primary_visible?: boolean;
  vertex_colors?: [number,number,number,number][];
};
export type StudioMaterialOptions = {
  opacity: number; ambient_response: number; double_sided: boolean; alpha_test: number; unlit?: boolean;
  /** Texture indices: color, emissive, roughness, metalness, normal, AO. -1 = none. */
  maps: [number, number, number, number, number, number];
};
export type StudioTexture = {
  id: string; width: number; height: number;
  /** RGBA8, straight alpha. First row is UV v=0. Color/emission are sRGB. */
  rgba: number[];
};
export type StudioCameraOptions = {
  projection: 'perspective' | 'orthographic'; up: StudioVec3;
  ortho_height: number; near: number; far: number;
};
export type StudioEnvironmentOptions = {
  backdrop_opaque: boolean; shadows: boolean; floor_y: number; ground: StudioVec3;
};
export type StudioLightOptions = { cast_shadow: boolean; distance: number; decay: number };
export type StudioSceneDescription = {
  version: 1;
  /** Direct serialized Rust Scene. Mesh positions/normals are evaluated world space. */
  scene: {
    objects: { mesh: StudioMesh; material: number }[];
    materials: StudioMaterial[]; lights: StudioSoftbox[];
    camera: StudioCameraDescription; backdrop: StudioVec3;
    environment: [StudioVec3, StudioVec3]; exposure: number; shadow_catcher: boolean;
    extra_lights: StudioAnalyticLight[]; layers: number[];
  };
  /** layers[] indexes this list; object_ids[] parallels scene.objects[]. */
  layer_ids: string[]; object_ids: StudioObjectIdentity[];
  textures: StudioTexture[]; material_options: StudioMaterialOptions[];
  camera_options: StudioCameraOptions; environment_options: StudioEnvironmentOptions;
  /** Softboxes first, then extra_lights, matching Rust's prepared light order. */
  light_options: StudioLightOptions[];
};
export type StudioScenePatch = Partial<Omit<StudioSceneDescription, 'version' | 'scene'>> & {
  /** Top-level scene fields are replaced, never recursively merged. */
  scene?: Partial<StudioSceneDescription['scene']>;
};
export type StudioTier = 'live' | 'refine' | 'final';
export type StudioRenderRequest = {
  tier: StudioTier; size: { width: number; height: number }; samples: number;
  /** Omit for combined output. A layer image includes scene occlusion/reflections. */
  layer?: string; background?: boolean;
  /** Accumulate from the previous refine request; scene/camera/time edits reset it. */
  accumulate?: boolean; denoise?: boolean; signal?: AbortSignal;
};
export type StudioFrame = {
  source: CanvasImageSource; width: number; height: number;
  samples: number; version: string; release?: () => void;
};
export type StudioPick = { layer: string; part: string | null; point: StudioVec3; normal: StudioVec3 };
export type StudioBounds = { min: StudioVec3; max: StudioVec3 };
export type StudioResult<T> = T | Promise<T>;
export interface StudioBackend {
  readonly id: string;
  initialize(): Promise<void>;
  /** Optional binary fast path. Same evaluated data as description(), without JSON mesh copies.
   * Maps use the Studio texture-array layout. Native hosts can retain meshes/BLAS by geometryKey.
   */
  setPreparedScene?(scene: StudioScene, maps: StudioTextureSource[], description: () => StudioSceneDescription): StudioResult<void>;
  setView?(view: StudioView): StudioResult<void>;
  setScene(scene: StudioSceneDescription): StudioResult<void>;
  patchScene(patch: StudioScenePatch): StudioResult<void>;
  setTime(seconds: number): StudioResult<void>;
  setCamera(camera: StudioCameraDescription, options: StudioCameraOptions): StudioResult<void>;
  render(request: StudioRenderRequest): StudioResult<StudioFrame>;
  /** Normalized frame coordinates, top-left (0,0), bottom-right (1,1). */
  pick?(x: number, y: number): StudioResult<StudioPick | null>;
  bounds?(layer?: string): StudioResult<StudioBounds | null>;
  dispose(): void;
}
export type StudioTextureSource = HTMLCanvasElement | OffscreenCanvas | Uint8Array;
export type StudioBackendContext = { invalidate: () => void };
export type StudioBackendFactory = (context: StudioBackendContext) => StudioBackend;
let backendFactory: StudioBackendFactory | null = null;
let factoryRevision = 0;
/** Native integration registers before the first frame; replacing it releases old jobs. */
export function registerStudioBackend(factory: StudioBackendFactory | null): void {
  backendFactory = factory; factoryRevision++;
}
export function studioBackendFactory(): StudioBackendFactory | null { return backendFactory; }
export function studioBackendRevision(): number { return factoryRevision; }
export function patchStudioScene(scene: StudioSceneDescription, patch: StudioScenePatch): StudioSceneDescription {
  return { ...scene, ...patch, version: 1, scene: { ...scene.scene, ...patch.scene } };
}
