import type { LightType, MapSlot, Primitive } from './scene-types';

/* Field definitions shared by the native inspector and layer commands. */

export type FieldKind = 'num' | 'color' | 'toggle';
export interface FieldSpec { key: string; label: string; kind: FieldKind; step?: number; min?: number; max?: number; unit?: string; prefix?: string }

export const PRIMITIVES: ReadonlyArray<{ id: Primitive; label: string }> = [
  { id: 'box', label: 'Cube' },
  { id: 'sphere', label: 'Sphere' },
  { id: 'plane', label: 'Plane' },
  { id: 'cylinder', label: 'Cylinder' },
  { id: 'cone', label: 'Cone' },
  { id: 'torus', label: 'Torus' },
  { id: 'capsule', label: 'Capsule' },
  { id: 'icosahedron', label: 'Icosphere' }
];

export const LIGHT_TYPES: ReadonlyArray<{ id: LightType; label: string }> = [
  { id: 'sun', label: 'Sun' },
  { id: 'point', label: 'Point' },
  { id: 'spot', label: 'Spot' },
  { id: 'area', label: 'Area' }
];

export const MAP_SLOTS: ReadonlyArray<{ id: MapSlot; label: string }> = [
  { id: 'color', label: 'Color map' },
  { id: 'normal', label: 'Normal map' },
  { id: 'roughness', label: 'Roughness map' },
  { id: 'metalness', label: 'Metalness map' },
  { id: 'emissive', label: 'Emissive map' },
  { id: 'ao', label: 'Occlusion map' }
];

/** Channel keys are `x`, `rx`, `sx` for transforms and camel-cased `targetX` for targets. */
const xyz = (prefix: string, label: string, step: number, unit?: string): FieldSpec[] =>
  (['x', 'y', 'z'] as const).map((axis) => ({
    key: prefix.length > 1 ? `${prefix}${axis.toUpperCase()}` : `${prefix}${axis}`,
    label: `${label} ${axis.toUpperCase()}`,
    kind: 'num' as const,
    step,
    unit,
    prefix: axis.toUpperCase()
  }));

export const MATERIAL_FIELDS: FieldSpec[] = [
  { key: 'color', label: 'Color', kind: 'color' },
  { key: 'roughness', label: 'Roughness', kind: 'num', step: 0.01, min: 0.02, max: 1 },
  { key: 'metalness', label: 'Metalness', kind: 'num', step: 0.01, min: 0, max: 1 },
  { key: 'emissive', label: 'Emission', kind: 'color' },
  { key: 'emissiveIntensity', label: 'Emission strength', kind: 'num', step: 0.05, min: 0 },
  { key: 'opacity', label: 'Opacity', kind: 'num', step: 0.01, min: 0, max: 1 }
];

export function lightFields(type: LightType): FieldSpec[] {
  const fields: FieldSpec[] = [
    { key: 'color', label: 'Color', kind: 'color' },
    { key: 'intensity', label: 'Intensity', kind: 'num', step: 0.05, min: 0 },
    ...xyz('', 'Position', 0.01)
  ];
  if (type !== 'point') fields.push(...xyz('target', 'Target', 0.01));
  if (type === 'area') {
    fields.push(
      { key: 'width', label: 'Size X', kind: 'num', step: 0.05, min: 0.001 },
      { key: 'height', label: 'Size Y', kind: 'num', step: 0.05, min: 0.001 }
    );
  } else if (type !== 'sun') {
    fields.push(
      { key: 'distance', label: 'Range', kind: 'num', step: 0.1, min: 0 },
      { key: 'decay', label: 'Falloff', kind: 'num', step: 0.05, min: 0 }
    );
  }
  if (type === 'spot') {
    fields.push(
      { key: 'angle', label: 'Cone angle', kind: 'num', step: 1, min: 1, max: 89, unit: '°' },
      { key: 'penumbra', label: 'Softness', kind: 'num', step: 0.01, min: 0, max: 1 }
    );
  }
  return fields;
}

export function cameraFields(projection: 'perspective' | 'orthographic'): FieldSpec[] {
  return [
    ...xyz('', 'Position', 0.01),
    ...xyz('target', 'Target', 0.01),
    projection === 'perspective'
      ? { key: 'fov', label: 'Field of view', kind: 'num', step: 0.5, min: 1, max: 150, unit: '°' }
      : { key: 'zoom', label: 'Zoom', kind: 'num', step: 0.01, min: 0.01 },
    { key: 'near', label: 'Clip start', kind: 'num', step: 0.01, min: 0.001 },
    { key: 'far', label: 'Clip end', kind: 'num', step: 1, min: 0.01 }
  ];
}

export const ENVIRONMENT_FIELDS: FieldSpec[] = [
  { key: 'ambient', label: 'Ambient', kind: 'num', step: 0.01, min: 0 },
  { key: 'ambientColor', label: 'Ambient color', kind: 'color' },
  { key: 'exposure', label: 'Exposure', kind: 'num', step: 0.01, min: 0 }
];
