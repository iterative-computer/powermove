import type { PowermoveAPI, EffectParamDefinition } from 'powermove';
import { installTypingRaster } from './raster';

// Keep the contribution id and parameter keys: saved colors, expressions and
// Progress keyframes remain the only editable source of the effect's values.
const params: EffectParamDefinition[] = [
  { k: 'progress', label: 'Progress', def: 100, min: 0, max: 100, step: 0.1, unit: '%' },
  { k: 'cursor', label: 'Show Cursor', def: true, type: 'toggle' },
  { k: 'cursorWidth', label: 'Cursor Width', def: 8, min: 1, max: 200, step: 1, unit: '% em' },
  { k: 'cursorHeight', label: 'Cursor Height', def: 100, min: 1, max: 300, step: 1, unit: '%' },
  { k: 'cursorOffsetX', label: 'Cursor Gap', def: 4, min: 0, max: 200, step: 1, unit: '% em' },
  { k: 'cursorOffsetY', label: 'Cursor Offset Y', def: 0, min: -200, max: 200, step: 1, unit: '% em' },
  { k: 'cursorRound', label: 'Cursor Roundness', def: 0, min: 0, max: 100, step: 1, unit: '%' },
  { k: 'blinkRate', label: 'Blink Rate', def: 2, min: 0, max: 10, step: 0.05, unit: 'Hz' },
  { k: 'gradStart', label: 'Cursor Gradient Start', def: '#7CE7FF', type: 'color' },
  { k: 'gradMid', label: 'Cursor Gradient Middle', def: '#A67CFF', type: 'color' },
  { k: 'gradEnd', label: 'Cursor Gradient End', def: '#FF6FD8', type: 'color' },
  { k: 'gradMidPos', label: 'Middle Position', def: 50, min: 1, max: 99, step: 1, unit: '%' },
  { k: 'gradAngle', label: 'Gradient Angle', def: 90, min: -360, max: 360, step: 1, unit: '°' },
  { k: 'gradOffset', label: 'Gradient Offset', def: 0, min: -200, max: 200, step: 1, unit: '%' },
  { k: 'gradCycle', label: 'Gradient Cycle', def: 0, min: -10, max: 10, step: 0.05, unit: 'Hz' }
];

export default function activate(api: PowermoveAPI): void {
  installTypingRaster(api, params);
  api.effects.register({
    id: 'typewriter', label: 'Typewriter', group: 'Text', params,
    // The text stage renders the typed prefix and cursor together. Subsequent
    // effects, layer transforms, masks and export use the same native surface.
    frag: 'o = texture(u_tex, v_st);'
  });
  api.transport.invalidate();
}
