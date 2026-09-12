import { expect, it, vi } from 'vitest';
import { createAgentCheckpoint } from './checkpoint';
import { install } from './harness';
import type { PMRegistry } from '../registry';

it('retains the original project for Undo without touching saved Takes or copying it into UI snapshots', () => {
  const proj = { id: 'project', layers: [{ id: 'layer', value: 42 }] };
  const PM: PMRegistry = {
    proj, uid: () => 'checkpoint',
    hist: { restoreSnapshot: vi.fn(() => true) },
    takes: { all: vi.fn(() => { throw new Error('Large archive must not be read'); }), save: vi.fn() }
  };
  install(PM);
  const checkpoint = createAgentCheckpoint(PM, 'Before the run');
  proj.layers[0]!.value = 99;
  expect(JSON.parse(checkpoint.json).layers[0].value).toBe(42);
  expect(JSON.stringify(checkpoint)).toBe('{"id":"checkpoint","label":"Before the run"}');
  expect(PM.AgentHarness.rollback(checkpoint)).toBe(true);
  expect(PM.hist.restoreSnapshot).toHaveBeenCalledWith(checkpoint.json, 'Undo agent run');
  expect(PM.takes.save).not.toHaveBeenCalled();
  expect(PM.takes.all).not.toHaveBeenCalled();
});
