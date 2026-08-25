import { describe, expect, it } from 'vitest';
import { channelBinding, compositionBinding, contentBinding, layerFieldBinding } from './binding';

const command = (binding: ReturnType<typeof channelBinding>, value: unknown) => {
  if (binding.mode !== 'command') throw new Error('Expected command binding');
  return typeof binding.command === 'function' ? binding.command(value) : { ...binding.command, value };
};

describe('control bindings', () => {
  it('builds the inspector set_property command with live time and human intent', () => {
    const PM = { time: 1 };
    const binding = channelBinding(PM, 'L1', 'position.x', { label: 'X', time: () => PM.time });
    PM.time = 4.5;
    expect(command(binding, 24)).toEqual({
      type: 'set_property', target: 'L1', path: 'position.x', value: 24,
      time: 4.5, mode: 'auto', preserveHandEdits: false, markIntent: 'human'
    });
  });

  it('maps layer, content, and composition fields to their edit commands', () => {
    expect(command(layerFieldBinding({}, 'L1', 'layer.visible'), true)).toEqual({ type: 'set_layer', target: 'L1', patch: { visible: true } });
    expect(command(contentBinding({}, 'L1', 'content.text'), 'Hello')).toEqual({ type: 'set_content', target: 'L1', patch: { text: 'Hello' } });
    expect(command(compositionBinding({}, 'composition.background'), '#123456')).toEqual({ type: 'set_composition', patch: { background: '#123456' } });
  });

  it('clamps work-area endpoints with the same frame gap as sourceBinding', () => {
    const PM = { proj: { dur: 10, fps: 20, work: [2, 8] }, clamp: (n: number, min: number, max: number) => Math.max(min, Math.min(max, n)) };
    expect(command(compositionBinding(PM, 'workArea.start'), 9)).toEqual({ type: 'set_composition', patch: { workArea: [7.95, 8] } });
    expect(command(compositionBinding(PM, 'workArea.end'), 1)).toEqual({ type: 'set_composition', patch: { workArea: [2, 2.05] } });
  });
});
