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
  it('renders a Claude summary with headings, lists, and bold text', () => {
    agentState.provider = 'claude';
    render({ role: 'assistant', text: '## Done\n\n- **One**\n- Two' });
    expect(target.querySelector('.agent-md-h')?.textContent).toBe('Done');
    expect([...target.querySelectorAll('.agent-md-li')].map(item => item.textContent?.trim())).toEqual(['One', 'Two']);
    expect(target.querySelector('.agent-md-li .is-bold')?.textContent).toBe('One');
  });

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
    expect(target.querySelector('.agent-work-log')).toBeNull();
  });

  it('keeps steering checkpoints visible in stream order, even after the run finishes', () => {
    render({
      role: 'trace', steering: true,
      steps: [
        { kind: 'thought', id: 'before', label: 'Inspecting the timing.', live: false },
        { kind: 'text', id: 'progress', text: 'I found the transition.' },
        { kind: 'tool', id: 'read', toolName: 'bash', label: 'Read timeline', status: 'continued' },
        { kind: 'thought', id: 'after', label: 'Keeping the clips aligned.', live: false },
      ]
    });
    expect(target.querySelector('.agent-work-log')).toBeNull();
    expect([...target.querySelectorAll('.agent-trace-text')].map(row => row.textContent)).toEqual([
      'Inspecting the timing.', 'I found the transition.', 'Keeping the clips aligned.'
    ]);
    expect(target.querySelectorAll('.agent-tool-activity')).toHaveLength(1);
    expect(target.querySelector('.shimmer-text')).toBeNull();
  });

  it.each([false, true])('collapses completed work above the final reply (tools: %s)', (withTools) => {
    render({
      role: 'trace', durationMs: 543000,
      steps: [
        { kind: 'text', id: 'progress', text: 'Inspecting the original audio.' },
        { kind: 'thought', id: 'thought-1', label: 'Check the original timing.', live: false },
        ...(withTools ? [{ kind: 'tool' as const, id: 'read', toolName: 'bash', label: 'Inspect timeline', status: 'done' as const }] : []),
        { kind: 'thought', id: 'thought-2', label: 'Keep both stems aligned.', live: false },
        { kind: 'text', id: 'reply', text: 'Both stems are ready.' }
      ]
    });
    const work = target.querySelector<HTMLDetailsElement>('details.agent-work-log')!;
    expect(work).not.toBeNull();
    expect(work.open).toBe(false);
    expect(work.querySelector('summary')?.textContent).toBe('Worked for 9m 3s');
    expect(work.textContent).toContain('Inspecting the original audio.');
    expect(work.textContent).toContain('Check the original timing.');
    expect(work.textContent).toContain('Keep both stems aligned.');
    expect(work.textContent).not.toContain('Both stems are ready.');
    const tools = work.querySelector<HTMLDetailsElement>('details.agent-tool-activity');
    expect(Boolean(tools)).toBe(withTools);
    if (tools) expect(tools.querySelector('summary')?.textContent).toContain('1 tool call');
    expect(target.querySelector('.is-archived > .agent-trace-prose')?.textContent).toBe('Both stems are ready.');
  });

  it('uses recorded tool timestamps for older history and omits empty work disclosures', () => {
    render({ role: 'trace', steps: [
      { kind: 'tool', id: 'tool', toolName: 'bash', label: 'Inspect', status: 'done', startedAt: 1000, endedAt: 5100 }
    ] });
    expect(target.querySelector('.agent-work-log > summary')?.textContent).toBe('Worked for 4s');
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

it('states what the agent could not do without dressing it as an editor error', () => {
  const text = 'The generated workspace was not safe or complete enough to preview';
  render({ role: 'assistant', error: true, notice: 'alert', text, entering: true });

  const notice = target.querySelector('.error-notice')!;
  expect(notice.classList.contains('is-alert')).toBe(true);
  expect(notice.textContent).toContain(text);
  // The agent's own sentence is the whole story: no title to read past, and
  // nothing technical to unfold.
  expect(target.querySelector('details')).toBeNull();
  expect(target.querySelector('strong')).toBeNull();
  expect(target.querySelector('[role="alert"]')).toBeNull();
  expect([...target.querySelectorAll('button')].some(button => button.textContent === 'Try again')).toBe(true);
});

it('lets a turn where nothing broke read as an ordinary reply', () => {
  const text = 'Those panels were already arranged that way. Nothing was changed.';
  render({ role: 'assistant', error: true, notice: 'plain', text, entering: false });

  expect(target.querySelector('.error-notice')).toBeNull();
  expect(target.querySelector('.agent-msg.is-error')).toBeNull();
  expect(target.textContent).toContain('Those panels were already arranged that way');
  // It is still a turn the user may want to run again.
  expect([...target.querySelectorAll('button')].some(button => button.textContent === 'Try again')).toBe(true);
});

it('shows a readable error with the raw diagnostic behind details', () => {
  const raw = 'The agent failed: Error: thread/resume: thread/resume failed: thread abc already has an active writer (code -32600)';
  render({ role: 'assistant', error: true, text: raw, entering: true });
  expect(target.querySelector('[role="alert"]')).toBeTruthy();
  expect(target.querySelector('strong')?.textContent).toBe('Agent session couldn’t reopen');
  const toggle = target.querySelector<HTMLButtonElement>('.log-toggle')!;
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(target.querySelector('pre')).toBeNull();
  toggle.click();
  flushSync();
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(target.querySelector('pre')?.textContent).toBe(raw);
  expect(target.querySelector('p')?.textContent).not.toContain('-32600');
});
