// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Turn from './Turn.svelte';
import { WORD_REVEAL_SETTLE_MS } from './motion';
import type { AgentMessage } from './agent-state.svelte';
import { agentState, resetAgentState } from './agent-state.svelte';

let target: HTMLDivElement;
let instance: Record<string, any> | undefined;

function render(message: AgentMessage) {
  instance = mount(Turn, { target, props: { PM: {}, message } }) as Record<string, any>;
  flushSync();
}

beforeEach(() => {
  resetAgentState();
  vi.useFakeTimers();
  target = document.createElement('div');
  document.body.appendChild(target);
});

afterEach(() => {
  if (instance) unmount(instance);
  instance = undefined;
  target.remove();
  vi.useRealTimers();
});

describe('assistant word reveal', () => {
  it.each(['chatgpt', 'claude', 'compatible'] as const)('offers Project continuation for %s only when the user chooses it', (provider) => {
    agentState.provider = provider;
    const continueWithProject = vi.fn();
    instance = mount(Turn, { target, props: {
      PM: { AgentUI: { continueWithProject } }, messageIndex: 3,
      message: { role: 'assistant', text: 'This effect needs Project access.', requiresProject: true }
    } });
    flushSync();
    expect(continueWithProject).not.toHaveBeenCalled();
    const button = target.querySelector<HTMLButtonElement>('button')!;
    expect(button.textContent).toBe('Continue with Project access');
    button.click();
    expect(continueWithProject).toHaveBeenCalledWith(3);
  });

  it('keeps replayed tool details readable and partial failures visible while collapsed', () => {
    render({
      role: 'trace',
      steps: [
        { kind: 'tool', id: 'replayed', toolName: 'bash', label: 'Read project', status: 'done' },
        { kind: 'tool', id: 'replayed', toolName: 'bash', label: 'Read project again', status: 'continued' },
        { kind: 'tool', id: 'failed', toolName: 'bash', label: 'Preview failed', status: 'error' }
      ]
    });
    const row = target.querySelector<HTMLDetailsElement>('details.agent-trace-tool')!;
    expect(row.open).toBe(false);
    expect(row.querySelector('summary')?.textContent).toContain('1 failed');
    expect(row.className).toContain('is-partial');
    expect(row.querySelectorAll('.agent-tool-details > div')).toHaveLength(3);
    expect(row.querySelector('.agent-tool-details')?.textContent).toContain('Continued');
  });

  it('keeps archived agent text visible between prompts', () => {
    render({
      role: 'trace',
      steps: [{ kind: 'text', id: 'before-steer', text: 'I have started building the blur.' }]
    });
    expect(target.querySelector('.agent-trace-text')?.textContent).toBe('I have started building the blur.');
  });

  it('keeps archived tool details expandable without live motion', () => {
    render({
      role: 'trace',
      steps: [
        { kind: 'tool', id: 'tool-1', toolName: 'bash', label: 'bash · npm test', status: 'done' },
        { kind: 'tool', id: 'tool-2', toolName: 'edit', label: 'edit · Turn.svelte', status: 'done' }
      ]
    });
    const row = target.querySelector('details.agent-trace-tool')!;
    expect(row.className).not.toContain('is-pulsing');
    expect([...row.querySelectorAll('.agent-tool-details span')].map((item) => item.textContent))
      .toEqual(['bash · npm test', 'edit · Turn.svelte']);
  });

  it('keeps a finished reply in one stable text node', () => {
    render({ role: 'assistant', text: 'I moved the panel', entering: true });
    const words = target.querySelectorAll('.agent-word');
    expect(words.length).toBe(0);
    expect(target.querySelector('p')?.textContent).toBe('I moved the panel');
  });

  it('does not delay a completed reply behind a word sweep', () => {
    render({ role: 'assistant', text: 'one two three four', entering: true });
    const delays = Array.from(target.querySelectorAll('.agent-word')).map((word) =>
      parseFloat((word.getAttribute('style') || '').replace(/[^0-9.]/g, '')));
    expect(delays).toEqual([]);
    for (let i = 1; i < delays.length; i++) {
      expect(delays[i]).toBeGreaterThan(delays[i - 1]!);
    }
  });

  /* The regression that made this invisible: `entering` is cleared on the very
     next snapshot read, which lands mid-reveal. */
  it('keeps the words when entering is cleared mid-reveal', async () => {
    const message: AgentMessage = { role: 'assistant', text: 'still here', entering: true };
    render(message);
    const paragraph = target.querySelector('p');
    const textNode = paragraph?.firstChild;

    message.entering = false;
    flushSync();
    expect(target.querySelector('p')?.firstChild).toBe(textNode);
  });

  it('keeps plain text after the reveal settles', () => {
    render({ role: 'assistant', text: 'settled text', entering: true });
    vi.advanceTimersByTime(WORD_REVEAL_SETTLE_MS + 10);
    flushSync();
    expect(target.querySelectorAll('.agent-word').length).toBe(0);
    expect(target.textContent).toContain('settled text');
  });

  it('leaves a replayed message alone', () => {
    render({ role: 'assistant', text: 'from history', entering: false });
    expect(target.querySelectorAll('.agent-word').length).toBe(0);
    expect(target.textContent).toContain('from history');
  });

  it('does not word-split the user turn', () => {
    render({ role: 'user', text: 'move the panel', entering: true });
    expect(target.querySelectorAll('.agent-word').length).toBe(0);
    expect(target.textContent).toContain('move the panel');
  });
});

it('shows a readable error with the raw diagnostic behind details', () => {
  const raw = 'The agent failed: Error: thread/resume: thread/resume failed: thread abc already has an active writer (code -32600)';
  render({ role: 'assistant', error: true, text: raw, entering: true });
  expect(target.querySelector('[role="alert"]')).toBeTruthy();
  expect(target.querySelector('strong')?.textContent).toBe('Agent session couldn’t reopen');
  expect(target.querySelector('details')?.open).toBe(false);
  expect(target.querySelector('pre')?.textContent).toBe(raw);
  expect(target.querySelector('p')?.textContent).not.toContain('-32600');
});
