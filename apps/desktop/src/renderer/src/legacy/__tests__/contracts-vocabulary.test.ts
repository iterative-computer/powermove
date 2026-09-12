import { describe, expect, it } from 'vitest';

import { AGENT_COMMAND_TYPES, EDIT_COMMAND_TYPES } from '../../../../shared/edit-vocabulary';
import { makePM } from './make-pm';

function harnessCommand(type: string): Record<string, unknown> {
  if (type === 'replace_keyframes' || type === 'set_easing') {
    return { type, keyframes: [] };
  }
  if (type === 'create_section') {
    return { type, section: { id: 'section-contract', layers: [] } };
  }
  if (type === 'update_section') {
    return { type, sectionId: 'section-contract', layers: [{ id: 'layer-contract' }] };
  }
  return { type };
}

describe('agent edit vocabulary contract', () => {
  it('keeps the harness, shared vocabulary, and editor on the same 22 commands', () => {
    const PM = makePM('core/editing', 'assistant/harness');
    const harnessOperations = EDIT_COMMAND_TYPES.map((type) => {
      const cleaned = PM.AgentHarness.cleanCommand(harnessCommand(type));
      expect(cleaned, `harness rejected ${type}`).not.toBeNull();
      if (!cleaned) throw new Error(`harness rejected ${type}`);
      return cleaned.type;
    });

    expect(new Set(EDIT_COMMAND_TYPES).size).toBe(22);
    expect(harnessOperations).toEqual([...AGENT_COMMAND_TYPES]);
    expect(Object.keys(PM.Edit.operations)).toEqual([...EDIT_COMMAND_TYPES]);
    expect(AGENT_COMMAND_TYPES).toEqual(EDIT_COMMAND_TYPES);
  });
});
