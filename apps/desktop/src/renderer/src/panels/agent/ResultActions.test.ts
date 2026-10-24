// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, expect, it } from 'vitest';
import ResultActions from './ResultActions.svelte';
import Turn from './Turn.svelte';
import { agentState, resetAgentState } from './agent-state.svelte';

afterEach(() => resetAgentState());
it.each([false, true])('reports a composition preview failure exactly once (archived=%s)', async archived => {
  resetAgentState();
  const message = 'Composition preview unavailable: Video frame decode timed out';
  agentState.run = { autonomous: true, summary: 'Ready', reviewError: message, reviewErrorInConversation: archived };
  const target = document.createElement('div'); document.body.append(target);
  const instances = [mount(ResultActions, { target, props: { PM: {} } })];
  if (archived) instances.push(mount(Turn, { target, props: { PM: {}, message: { role: 'assistant', text: message } } }));
  flushSync();
  expect(target.textContent?.split(message).length).toBe(2);
  for (const instance of instances) await unmount(instance);
  target.remove();
});
