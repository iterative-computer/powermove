import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './scripting';

function scriptingRegistry(): PMRegistry {
  let id = 0;
  const allowed = new Set(['set_layer', 'set_content', 'add_layer', 'delete_layers', 'set_expression', 'replace_keyframes']);
  const PM: PMRegistry = {
    uid: (prefix: string) => `${prefix}-${++id}`,
    AgentHarness: {
      cleanCommand(command: any) {
        return command && allowed.has(command.type) ? JSON.parse(JSON.stringify(command)) : null;
      },
      describeCommand(command: any) { return `${command.type}:${command.target || command.name || ''}`; },
    },
    selLayers: () => [],
  };
  install(PM);
  return PM;
}

const state = {
  composition: { revision: 4 }, selection: { layers: ['text-1'] },
  layers: [
    { id: 'text-1', name: 'Title', type: 'text', locked: false },
    { id: 'locked-1', name: 'Locked', type: 'text', locked: true },
  ],
};

describe('legacy isolated scripting install', () => {
  it('bounds typed results, protects locked layers, and exposes explicit limits', () => {
    const PM = scriptingRegistry();
    const commands = PM.Script.test.normalizeCommands([
      { type: 'add_layer', layerType: 'text', name: 'A' },
      { type: 'set_layer', target: 'text-1', patch: { visible: false } },
    ], state);

    expect(commands).toHaveLength(2);
    expect(() => PM.Script.test.normalizeCommands([
      { type: 'set_layer', target: 'locked-1', patch: { visible: false } },
    ], state)).toThrow(/locked layers/);
    expect(() => PM.Script.test.normalizeCommands([
      { type: 'run_shell', command: 'whoami' },
    ], state)).toThrow(/unsupported/);
    expect(() => PM.Script.test.normalizeCommands([
      { type: 'set_expression', target: 'text-1', path: 'opacity', expression: 'window.location' },
    ], state)).toThrow(/cannot install executable project expressions/);
    expect(PM.Script.sourceCode('return [];')).toBe('return [];');
    expect(PM.Script.sourceCode('x'.repeat(PM.Script.LIMITS.source + 1))).toBe('');
    expect(PM.Script.catalog().timeoutMs).toBe(900);
    expect(PM.Script.catalog().sdk).toContain('addLayer');
  });
});
