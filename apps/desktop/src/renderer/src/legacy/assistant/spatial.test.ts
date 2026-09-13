import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './spatial';

function spatialRegistry(): PMRegistry {
  const PM: PMRegistry = {
    h() {},
    uid: () => 'id',
    clamp: (value: any, min: any, max: any) => Math.max(min, Math.min(max, value)),
    round: (value: any, places = 0) => Number(Number(value).toFixed(places)),
    snapF: (time: any, fps: any) => Math.round(time * fps) / fps,
    TYPE_META: { text: {} },
    Ease: {
      PRESETS: {
        linear: [0, 0, 1, 1],
        easeInOut: [.42, 0, .58, 1],
        power: [.62, .05, 0, 1],
        backOut: [.34, 1.56, .64, 1],
      },
      bezier: () => (value: any) => value,
      nameOf: () => 'custom',
    },
    commands: { fitView: { id: 'fitView', label: 'Fit view' } },
    PANELS: {
      viewer: { title: 'Composition' },
      timeline: { title: 'Timeline' },
      inspector: { title: 'Inspector' },
      assets: { title: 'Project' },
    },
    registerPanel(id: any, definition: any) { this.PANELS[id] = definition; },
    firstSel: () => null,
    L: () => null,
    byName: () => null,
    findProp: () => null,
    AgentHarness: {
      sceneSchema: () => ({ type: 'object' }),
      sanitizeProposal: (value: any) => ({ commands: value?.commands || [] }),
      promptContext: () => '',
      describeCommand: (command: any) => command.type,
    },
  };
  install(PM);
  return PM;
}

describe('legacy spatial assistant install', () => {
  it('resolves rectangular selections in either direction', () => {
    const math = spatialRegistry().SpatialAssistant.math;

    expect(math.selectionRect({ x: 310, y: 260 }, { x: 120, y: 80 })).toEqual(
      { x: 120, y: 80, width: 190, height: 180 },
    );
    expect(math.selectionRect({ x: 40, y: 50 }, { x: 40, y: 50 })).toEqual(
      { x: 40, y: 50, width: 0, height: 0 },
    );
  });

  it('distinguishes click cancellation from drag selection', () => {
    const math = spatialRegistry().SpatialAssistant.math;

    expect(math.isClickGesture([{ x: 100, y: 100 }, { x: 103, y: 102 }])).toBe(true);
    expect(math.isClickGesture([{ x: 100, y: 100 }, { x: 125, y: 102 }])).toBe(false);
    expect(math.overlayPointerAction('arming', 0, false)).toBe('cancel');
    expect(math.overlayPointerAction('selecting', 0, false)).toBe('select');
    expect(math.overlayPointerAction('composing', 0, true)).toBe('ignore');
    expect(math.overlayPointerAction('composing', 2, false)).toBe('ignore');
  });

  it('bounds large editable catalogs while prioritizing selected and named layers', () => {
    const math = spatialRegistry().SpatialAssistant.math;
    const controls = Array.from({ length: 80 }, (_, index) => ({
      path: `properties.control-${index}`,
      label: `Control ${index}`,
      control: 'slider',
      value: index,
      min: 0,
      max: 100,
    }));
    const layers = Array.from({ length: 700 }, (_, index) => ({
      id: `layer-${index}`,
      name: index === 699 ? 'Hero headline' : `Layer ${index}`,
      type: 'text',
      group: null,
      parent: null,
      controls,
    }));

    const result = math.boundedEditableSource(
      { target: '$composition', composition: [], operations: ['set_property'], layers },
      'Polish the Hero headline',
      ['layer-698'],
      72_000,
    );

    expect(JSON.stringify(result).length).toBeLessThanOrEqual(72_000);
    expect(result.truncated).toBe(true);
    expect(result.totalLayers).toBe(700);
    expect(result.layers.slice(0, 2).map((layer: any) => layer.id)).toEqual(['layer-698', 'layer-699']);
    expect(result.layerIndex[0]).toMatchObject({ id: 'layer-0', name: 'Layer 0', type: 'text' });
  });

  it('keeps oversized assembled prompts inside the IPC limit without dropping the user request', () => {
    const math = spatialRegistry().SpatialAssistant.math;
    const request = 'Explain what we can do after showing the Powermove window';
    const prompt = `AGENT INSTRUCTIONS\n${'source-property '.repeat(260_000)}\n\nUSER REQUEST\n${request}`;

    const result = math.boundedAgentPrompt(prompt, request, 200_000);

    expect(result.length).toBeLessThanOrEqual(200_000);
    expect(result).toContain('Earlier project context was clipped');
    expect(result.endsWith(`USER REQUEST\n${request}`)).toBe(true);
  });
});
