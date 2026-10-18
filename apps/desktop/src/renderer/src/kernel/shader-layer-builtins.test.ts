import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { install as installShaders } from '../legacy/gl/shaders';
import { install as installTransitions } from '../legacy/gl/transitions';
import { STUDIO_CUBE } from '../../../extensions/layers-3d/studio-cube';
import { OBJ_MODEL } from '../../../extensions/layers-3d/obj-model';
import { layerSnapshot, type Preambles } from './definition-snapshots';

/*
 * pm-render renders shader layers and the built-in extension layers from an
 * embedded copy of the app's shader-layer runtime
 * (`engine/crates/pm-render/src/shader_layers/builtins.json`). It must stay
 * identical: run `bun engine/crates/pm-render/tools/shaders/builtins.ts`
 * after changing `SHADER_HEADER`, the presets, the missing-layer
 * checkerboard, Studio Cube, or the OBJ model layer.
 */
describe('the shader-layer runtime pm-render embeds', () => {
  const PM: any = { uid: () => 'fx', P: (v: unknown) => ({ v }) };
  installShaders(PM);
  installTransitions(PM);
  const preambles: Preambles = { effect: PM.GLSL_PRE, transition: PM.TRANSITION_PRE, layer: PM.SHADER_HEADER };
  const embedded = JSON.parse(readFileSync(join(__dirname, '../../../../../../engine/crates/pm-render/src/shader_layers/builtins.json'), 'utf8'));

  it('has the shader-layer header, template, and presets', () => {
    expect(embedded.header).toBe(PM.SHADER_HEADER);
    expect(embedded.template).toBe(PM.SHADER_TEMPLATE);
    expect(embedded.presets).toEqual(Object.entries(PM.SHADER_PRESETS).map(([name, code]) => ({ name, code })));
    expect(embedded.presets).toHaveLength(5);
  });

  it('has the missing-layer checkerboard the compositor draws', () => {
    const compositor = readFileSync(join(__dirname, '../legacy/gl/compositor.ts'), 'utf8');
    expect(/const missing = `([\s\S]*?)`;/.exec(compositor)?.[1]).toBe(embedded.missing);
  });

  it('has the built-in layer snapshots', () => {
    const layers = [STUDIO_CUBE, OBJ_MODEL].map((d) => layerSnapshot(d, 'builtIn', preambles));
    expect(embedded.layers).toEqual(JSON.parse(JSON.stringify(layers)));
  });
});
