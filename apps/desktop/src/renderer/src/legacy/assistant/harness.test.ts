import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './harness';

function harnessRegistry(): PMRegistry {
  const PM: PMRegistry = {
    proj: { dur: 6, revision: 0, work: [0, 6] },
    time: 0,
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    round: (value: number, places = 0) => Number(Number(value).toFixed(places)),
    selLayers: () => [],
  };
  install(PM);
  return PM;
}

describe('legacy assistant harness install', () => {
  it('preserves hand edits, bounds keyframes, and rejects unknown commands', () => {
    const PM = harnessRegistry();
    const cleaned = PM.AgentHarness.cleanCommand({
      type: 'set_property', target: 'title', path: 'position.x', value: 42,
      preserveHandEdits: false, unknownField: 'strip me',
    });
    const replaced = PM.AgentHarness.cleanCommand({
      type: 'replace_keyframes', target: 'title', path: 'opacity',
      keyframes: Array.from({ length: 85 }, (_, index) => ({ time: index / 30, value: index })),
      preserveHandEdits: false, unknownField: 'strip me too',
    });
    const easing = PM.AgentHarness.cleanCommand({
      type: 'set_easing',
      keyframes: Array.from({ length: 85 }, (_, index) => `key-${index}`),
      curve: [.2, .1, .8, .9], overrideLock: true,
    });
    const added = PM.AgentHarness.cleanCommand({ type: 'add_layer', layerType: 'text', name: 'Title' });

    expect(cleaned.preserveHandEdits).toBe(false);
    expect(replaced.preserveHandEdits).toBe(false);
    expect(PM.AgentHarness.cleanCommand({ type: 'set_property', target: 'title', path: 'position.x', value: 12 }).preserveHandEdits).toBe(true);
    expect(Object.hasOwn(cleaned, 'unknownField')).toBe(false);
    expect(replaced.keyframes).toHaveLength(80);
    expect(Object.hasOwn(replaced, 'unknownField')).toBe(false);
    expect(easing.keyframes).toHaveLength(80);
    expect(Object.hasOwn(easing, 'overrideLock')).toBe(false);
    expect(added.from).toBe(0);
    expect(PM.AgentHarness.cleanCommand({
      type: 'create_section', section: { id: 'section-1', layers: [{ id: 'layer-1' }] },
    })).toEqual({ type: 'create_section', section: { id: 'section-1', layers: [{ id: 'layer-1' }] } });
    expect(PM.AgentHarness.cleanCommand({ type: 'run_shell', command: 'whoami' })).toBeNull();
  });
  it('keeps group commands available to live and returned agent edits', () => {
    const PM = harnessRegistry();
    for (const command of [
      {type:'group_layers',targets:['a','b'],name:'Hero'},
      {type:'ungroup_layers',targets:['g']},
      {type:'move_to_group',targets:['a'],group:'g'},
    ]) expect(PM.AgentHarness.cleanCommand(command)).toEqual(command);
  });

});
