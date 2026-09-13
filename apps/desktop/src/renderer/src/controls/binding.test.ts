import { describe, expect, it } from 'vitest';
import { channelBinding, compositionBinding, contentBinding, layerFieldBinding, type ControlBindingAPI } from './binding';

const command = (binding: ReturnType<typeof channelBinding>, value: unknown) => {
  if (binding.mode !== 'command') throw new Error('Expected command binding');
  return typeof binding.command === 'function' ? binding.command(value) : { ...binding.command, value };
};

const fakeAPI = (overrides: Partial<ControlBindingAPI> = {}): ControlBindingAPI => ({
  project: { get: () => ({ dur: 10, fps: 20, work: [2, 8] }) },
  transport: { time: () => 1 },
  model: { layer: () => null },
  anim: { ev: () => null },
  util: { clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)) },
  ...overrides
} as ControlBindingAPI);

describe('control bindings', () => {
  it('reads a live API time source for channel edits', () => {
    let time = 1;
    const api = fakeAPI({ transport: { time: () => time } as ControlBindingAPI['transport'] });
    const binding = channelBinding(api, 'L1', 'position.x', { label: 'X' });
    time = 4.5;
    expect(command(binding, 20)).toEqual(expect.objectContaining({
      type: 'set_property', target: 'L1', path: 'position.x', value: 20, time: 4.5
    }));
  });

  it('maps layer and content fields to typed commands', () => {
    const api = fakeAPI();
    expect(command(layerFieldBinding(api, 'L1', 'name'), 'Title')).toEqual({ type: 'set_layer', target: 'L1', patch: { name: 'Title' } });
    expect(command(contentBinding(api, 'L1', 'text'), 'Hello')).toEqual({ type: 'set_content', target: 'L1', patch: { text: 'Hello' } });
    expect(command(compositionBinding(api, 'fps'), 60)).toEqual({ type: 'set_composition', patch: { fps: 60 } });
  });

  it('clamps work-area endpoints through project and utility APIs', () => {
    const api = fakeAPI();
    expect(command(compositionBinding(api, 'workArea.start'), 9)).toEqual({ type: 'set_composition', patch: { workArea: [7.95, 8] } });
    expect(command(compositionBinding(api, 'workArea.end'), 1)).toEqual({ type: 'set_composition', patch: { workArea: [2, 2.05] } });
  });
});
