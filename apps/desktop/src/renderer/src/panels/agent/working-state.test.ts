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

it('anchors the divider through repeated steering and resets for the next run', () => {
  resetAgentState();
  const snapshot = { ...agentState, legacyPhase: 'working', requestToken: 1,
    conversation: [{ role: 'user' as const, text: 'Build it' }] };
  setAgentSnapshot(snapshot);
  expect(agentState.workingConversationIndex).toBe(1);
  const conversation = [...snapshot.conversation,
    { role: 'trace' as const, steps: [] },
    { role: 'user' as const, text: 'Continue', steering: true }];
  setAgentSnapshot({ ...snapshot, conversation });
  expect(agentState.workingConversationIndex).toBe(1);
  conversation.push({ role: 'user', text: 'Keep going', steering: true });
  setAgentSnapshot({ ...snapshot, conversation });
  expect(agentState.workingConversationIndex).toBe(1);
  setAgentSnapshot({ ...snapshot, conversation, legacyPhase: 'idle' });
  expect(agentState.workingConversationIndex).toBeNull();
  setAgentSnapshot({ ...snapshot, conversation, requestToken: 2 });
  expect(agentState.workingConversationIndex).toBe(conversation.length);
});

it('keeps one timer when steering replaces the request mid-run', () => {
  vi.useFakeTimers(); vi.setSystemTime(1000);
  resetAgentState();
  const snapshot = { ...agentState, legacyPhase: 'working', requestToken: 1,
    conversation: [{ role: 'user' as const, text: 'Build it' }] };
  setAgentSnapshot(snapshot);
  vi.setSystemTime(90_000);
  const conversation = [...snapshot.conversation,
    { role: 'trace' as const, steps: [] },
    { role: 'user' as const, text: 'Make it blue', steering: true }];
  setAgentSnapshot({ ...snapshot, conversation, requestToken: 2, runStartedAt: 90_000 });
  expect(agentState.workingStartedAt).toBe(1000);
  expect(agentState.workingConversationIndex).toBe(1);
  setAgentSnapshot({ ...snapshot, conversation, requestToken: 2, legacyPhase: 'idle' });
  setAgentSnapshot({ ...snapshot, conversation: [...conversation, { role: 'user', text: 'Next' }], requestToken: 3 });
  expect(agentState.workingStartedAt).toBe(90_000);
  expect(agentState.workingConversationIndex).toBe(4);
});
