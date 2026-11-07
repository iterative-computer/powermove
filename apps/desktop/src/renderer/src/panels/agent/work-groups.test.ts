import { describe, expect, it } from 'vitest';
import type { AgentMessage, TraceStep } from './agent-state.svelte';
import { groupConversation, workDuration } from './work-groups';

const text = (id: string): TraceStep => ({ kind: 'text', id, text: id });
const tool = (id: string): TraceStep => ({ kind: 'tool', id, toolName: 'shell', label: id, status: 'done' } as TraceStep);
const shape = (conversation: AgentMessage[], live: number | null = null) => groupConversation(conversation, live).map(entry =>
  entry.kind === 'work' ? { work: entry.items.map(item => item.key), durationMs: entry.durationMs } : entry.key);

describe('groupConversation', () => {
  it('folds the work of a finished turn and keeps the spoken reply out', () => {
    const conversation: AgentMessage[] = [
      { role: 'user', text: 'Build it' },
      { role: 'trace', steps: [text('looking'), tool('read')], steering: true, durationMs: 30_000 },
      { role: 'user', text: 'Make it blue', steering: true },
      { role: 'trace', steps: [tool('edit'), text('Done.'), text('All blue.')], durationMs: 150_000 }
    ];
    expect(shape(conversation)).toEqual([
      'user-0',
      { work: ['trace-1', 'user-2', 'trace-3:work'], durationMs: 150_000 },
      'trace-3:reply'
    ]);
    const reply = groupConversation(conversation, null).at(-1)!;
    expect(reply.kind === 'message' && reply.message.steps?.map(step => step.id)).toEqual(['Done.', 'All blue.']);
  });

  it('keeps the trace prose above a notes-only assistant turn in the answer', () => {
    expect(shape([
      { role: 'user', text: 'Go' },
      { role: 'trace', steps: [tool('a'), text('Finished.')], durationMs: 5000 },
      { role: 'assistant', text: 'Note: exported.' }
    ])).toEqual(['user-0', { work: ['trace-1:work'], durationMs: 5000 }, 'trace-1:reply', 'assistant-2']);
  });

  it('folds everything before a separate assistant reply', () => {
    expect(shape([
      { role: 'user', text: 'Go' },
      { role: 'trace', steps: [tool('a')] },
      { role: 'assistant', text: 'Here it is.' },
      { role: 'user', text: 'Again' },
      { role: 'assistant', text: 'Sure.' }
    ])).toEqual(['user-0', { work: ['trace-1'], durationMs: undefined }, 'assistant-2', 'user-3', 'assistant-4']);
  });

  it('keeps published visual replies visible with the same key after completion', () => {
    const conversation: AgentMessage[] = [
      { role: 'user', text: 'Show the timing' },
      { role: 'assistant', visualReplies: [{ version: 1, id: 'chart', runId: 'run', title: 'Chart', html: '<h1>Chart</h1>', height: 300 }] },
      { role: 'trace', steps: [tool('publish')], durationMs: 1000 },
      { role: 'assistant', text: 'Here is the timing.' }
    ];
    expect(shape(conversation, 0)).toEqual(['user-0', 'assistant-1', 'trace-2', 'assistant-3']);
    expect(shape(conversation)).toEqual([
      'user-0', { work: ['trace-2'], durationMs: 1000 }, 'assistant-1', 'assistant-3'
    ]);
  });

  it('leaves the running turn and silent runs open', () => {
    const running: AgentMessage[] = [
      { role: 'user', text: 'Old' }, { role: 'trace', steps: [tool('a'), text('ok')] },
      { role: 'user', text: 'New' }, { role: 'trace', steps: [tool('b'), text('so far')], steering: true },
      { role: 'user', text: 'steer', steering: true }
    ];
    expect(shape(running, 4)).toEqual(['user-0', { work: ['trace-1:work'], durationMs: undefined }, 'trace-1:reply', 'user-2', 'trace-3', 'user-4']);
    expect(shape([{ role: 'user', text: 'Go' }, { role: 'trace', steps: [tool('a')] }])).toEqual(['user-0', 'trace-1']);
  });
});

it('formats work durations like the live timer', () => {
  expect([workDuration(45_000), workDuration(150_000), workDuration(3_725_000)]).toEqual(['45s', '2m 30s', '1h 2m 5s']);
});
