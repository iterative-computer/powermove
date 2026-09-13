import { afterEach, describe, expect, it } from 'vitest';

import {
  agentState,
  resetAgentState,
  setAgentSnapshot,
  type AgentSnapshot,
  type TraceStep
} from './agent-state.svelte';

function snapshot(trace: TraceStep[]): AgentSnapshot {
  return {
    legacyPhase: 'working',
    requestToken: 1,
    conversation: [],
    activity: '',
    trace,
    plan: null,
    run: null,
    panelRun: null,
    attachments: [],
    steps: [],
    stepsExpanded: false,
    scope: 'workspace',
    autoApplyPanels: true,
    provider: 'chatgpt',
    model: 'gpt-5.6-sol',
    reasoningEffort: 'high',
    accessMode: 'editor',
    composerDraft: '',
    pendingEntering: false,
    models: [],
    providers: [],
    reasoningEfforts: [],
    accessModes: []
  };
}

afterEach(() => resetAgentState());

describe('setAgentSnapshot trace reconciliation', () => {
  it('preserves existing row identity when only the last text row grows', () => {
    setAgentSnapshot(snapshot([
      { kind: 'thought', id: 'thought-1', label: 'Planning', live: false },
      { kind: 'text', id: 'text-1', text: 'Reading ' }
    ]));
    const firstRow = agentState.trace[0];

    setAgentSnapshot(snapshot([
      { kind: 'thought', id: 'thought-1', label: 'Planning', live: false },
      { kind: 'text', id: 'text-1', text: 'Reading the source' }
    ]));

    expect(agentState.trace[0]).toBe(firstRow);
    expect(agentState.trace.at(-1)).toMatchObject({ text: 'Reading the source' });
  });

  it('truncates rows missing from a shorter snapshot', () => {
    setAgentSnapshot(snapshot([
      { kind: 'thought', id: 'thought-1', label: 'Planning', live: false },
      { kind: 'text', id: 'text-1', text: 'Done' }
    ]));

    setAgentSnapshot(snapshot([
      { kind: 'thought', id: 'thought-1', label: 'Planning', live: false }
    ]));

    expect(agentState.trace).toHaveLength(1);
    expect(agentState.trace[0]?.id).toBe('thought-1');
  });

  it('replaces a row when its id changes at the same index', () => {
    setAgentSnapshot(snapshot([
      { kind: 'text', id: 'text-1', text: 'First' }
    ]));
    const originalRow = agentState.trace[0];

    setAgentSnapshot(snapshot([
      { kind: 'text', id: 'text-2', text: 'Second' }
    ]));

    expect(agentState.trace[0]).not.toBe(originalRow);
    expect(agentState.trace[0]).toEqual({ kind: 'text', id: 'text-2', text: 'Second' });
  });
});
