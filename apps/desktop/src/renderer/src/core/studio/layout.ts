/** Shared with pm_studio::scene::{GpuTri,GpuMaterial} and trace.wgsl. */
export const STUDIO_TRIANGLE_FLOATS = 36;
export const STUDIO_MATERIAL_FLOATS = 272;
export const IMPORT_SCALAR_FLOATS = 24;
export const IMPORT_MAP_FLOATS = 12;
export const IMPORT_MAP_SLOTS = [
  'map','emissiveMap','roughnessMap','metalnessMap','normalMap','aoMap',
  'clearcoatMap','clearcoatRoughnessMap','clearcoatNormalMap','transmissionMap',
  'thicknessMap','specularIntensityMap','specularColorMap','sheenColorMap',
  'sheenRoughnessMap','iridescenceMap','iridescenceThicknessMap','anisotropyMap',
] as const;
