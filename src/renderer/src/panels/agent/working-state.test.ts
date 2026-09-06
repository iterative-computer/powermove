import { afterEach, expect, it, vi } from 'vitest';
import { agentState, resetAgentState, setAgentSnapshot } from './agent-state.svelte';

afterEach(() => { resetAgentState(); vi.useRealTimers(); });
it('preserves the start through progress snapshots and resets between runs', () => {
  vi.useFakeTimers(); vi.setSystemTime(1000);
  resetAgentState();
  const snapshot = { ...agentState, threadId: undefined, legacyPhase: 'working', requestToken: 1 };
  setAgentSnapshot(snapshot);
  expect(agentState.workingStartedAt).toBe(1000);
  vi.setSystemTime(5000);
  setAgentSnapshot({ ...snapshot, activity: 'Still working' });
  expect(agentState.workingStartedAt).toBe(1000);
  setAgentSnapshot({ ...snapshot, legacyPhase: 'idle' });
  expect(agentState.workingStartedAt).toBeNull();
  setAgentSnapshot({ ...snapshot, requestToken: 2 });
  expect(agentState.workingStartedAt).toBe(5000);
});
