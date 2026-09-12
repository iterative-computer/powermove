import type { ExtensionLayerDefinition } from 'powermove';

export const OBJ_MODEL_ID = 'powermove.3d.obj-model';

export const OBJ_MODEL: ExtensionLayerDefinition = {
  id: OBJ_MODEL_ID,
  label: 'OBJ Model',
  version: 1,
  icon: 'grid',
  color: '#78A8FF',
  params: [
    { k: 'yaw', label: 'Camera yaw', def: 28, min: -180, max: 180, step: 1, unit: '°' },
    { k: 'pitch', label: 'Camera pitch', def: 18, min: -70, max: 70, step: 1, unit: '°' },
    { k: 'distance', label: 'Camera distance', def: 4.2, min: 1.5, max: 15, step: 0.05 },
    { k: 'rotationX', label: 'Rotate X', def: 0, min: -360, max: 360, step: 1, unit: '°' },
    { k: 'rotationY', label: 'Rotate Y', def: 25, min: -360, max: 360, step: 1, unit: '°' },
    { k: 'rotationZ', label: 'Rotate Z', def: 0, min: -360, max: 360, step: 1, unit: '°' },
    { k: 'size', label: 'Model size', def: 1.35, min: 0.05, max: 8, step: 0.01 },
    { k: 'roughness', label: 'Roughness', def: 0.25, min: 0.02, max: 1, step: 0.01 },
    { k: 'metalness', label: 'Metalness', def: 0.7, min: 0, max: 1, step: 0.01 },
    { k: 'objectColor', label: 'Material color', def: '#C7C4FF', type: 'color' },
    { k: 'lightColor', label: 'Key light', def: '#FFB36B', type: 'color' },
    { k: 'background', label: 'Background', def: '#0C0D12', type: 'color' },
    { k: 'autoRotate', label: 'Auto rotate', def: false, type: 'toggle' },
    { k: 'speed', label: 'Rotation speed', def: 25, min: -180, max: 180, step: 1, unit: '°/s' }
  ],
  defaults: {
    assetId: '',
    objects: [],
    camera: { id: 'camera', projection: 'perspective' },
    lights: [{ id: 'key', type: 'directional' }, { id: 'rim', type: 'directional' }]
  },
  renderer: { kind: 'mesh', assetField: 'assetId' }
};
