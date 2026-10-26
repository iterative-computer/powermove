import type { PowermoveAPI } from 'powermove';

import { EFFECTS } from './effects';

export default function activate(api: PowermoveAPI): void {
  if (api.effects.supportsTemporalInputs !== true || api.effects.supportsOwnerValidity !== true || api.effects.supportsBrushGeometry !== true || api.effects.supportsLutAssets !== true || api.effects.supportsTypedReferences !== true || api.effects.supportsSourceOwners !== true || api.effects.supportsSourceRasters !== true || api.effects.supportsInputSubframes !== true || api.effects.supportsLayerInputs !== true || api.effects.supportsVersions !== true || api.effects.supportsSpatialSampling !== true || typeof api.effects.parameterDefault !== 'function') throw new Error('These effects require an updated Powermove renderer.');
  for (const definition of EFFECTS) api.effects.register(definition);
}
