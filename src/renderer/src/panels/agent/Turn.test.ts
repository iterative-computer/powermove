// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Turn from './Turn.svelte';
import { WORD_REVEAL_SETTLE_MS } from './motion';
import type { AgentMessage } from './agent-state.svelte';

let target: HTMLDivElement;
let instance: Record<string, any> | undefined;

function render(message: AgentMessage) {
  instance = mount(Turn, { target, props: { PM: {}, message } }) as Record<string, any>;
  flushSync();
}

beforeEach(() => {
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
  it('splits a finished reply into animating word spans', () => {
    render({ role: 'assistant', text: 'I moved the panel', entering: true });
    const words = target.querySelectorAll('.agent-word');
    expect(words.length).toBeGreaterThan(1);
    expect(Array.from(words).map((w) => w.textContent).join('')).toBe('I moved the panel');
  });

  /* Every word carries its own offset, so the reveal reads as a wave rather
     than one block fading. */
  it('gives each word a later delay than the one before it', () => {
    render({ role: 'assistant', text: 'one two three four', entering: true });
    const delays = Array.from(target.querySelectorAll('.agent-word')).map((word) =>
      parseFloat((word.getAttribute('style') || '').replace(/[^0-9.]/g, '')));
    expect(delays[0]).toBe(0);
    for (let i = 1; i < delays.length; i++) {
      expect(delays[i]).toBeGreaterThan(delays[i - 1]!);
    }
  });

  /* The regression that made this invisible: `entering` is cleared on the very
     next snapshot read, which lands mid-reveal. */
  it('keeps the words when entering is cleared mid-reveal', async () => {
    const message: AgentMessage = { role: 'assistant', text: 'still here', entering: true };
    render(message);
    expect(target.querySelectorAll('.agent-word').length).toBeGreaterThan(0);

    message.entering = false;
    flushSync();
    expect(target.querySelectorAll('.agent-word').length).toBeGreaterThan(0);
  });

  /* Once it has settled the paragraph goes back to plain text, so a message
     that has stopped moving is not left as a row of inline-block boxes. */
  it('hands back to plain text after the wave settles', () => {
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
