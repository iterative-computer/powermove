import { describe, expect, it, vi } from 'vitest';
import { AgentThreads, conversationForAgent, conversationContextForAgent, normalizeGeneratedThreadTitle, threadTitle, threadMessageText } from './threads';

function setup() {
  const saved = new Map<string, unknown>(); let id = 0;
  const store = { get: (k: string, d: unknown) => saved.get(k) ?? d, set: vi.fn((k: string, v: unknown) => { saved.set(k, v); return true; }) };
  const make = () => new AgentThreads(store, () => `thread-${++id}`);
  return { saved, store, make };
}

describe('agent thread archive', () => {
  it('protects older decisions and answered questions from verbose recent replies', () => {
    const { make } = setup(); const threads = make(); threads.load('decisions');
    threads.active.conversation.push({ role: 'user', text: 'Create the title sequence.' },
      { role: 'user', text: 'Use our existing serif font throughout.' },
      { role: 'trace', steps: [{ kind: 'question', id: 'q', status: 'answered', transport: 'reply', blocking: false, questions: [{ id: 'color', question: 'Which palette?', header: 'Color', options: [], allowOther: true, secret: false }], answers: { color: 'Blue and white.' } }] });
    for (let i = 0; i < 30; i++) threads.active.conversation.push({ role: 'assistant', text: 'Detailed inspection. '.repeat(1000) });
    threads.active.conversation.push({ role: 'user', text: 'Keep the font, but use green instead of blue.' });
    threads.save(); const restored = make(); restored.load('decisions');
    const context = JSON.parse(conversationContextForAgent(restored.active.conversation, restored.activeId));
    expect(context.messages.find((turn: any) => turn.position === 1).text).toContain('serif font');
    expect(context.messages.find((turn: any) => turn.position === 2).text).toContain('Blue and white.');
    expect(context.messages.at(-1).text).toContain('green instead');
    expect(context.guidance).toContain('newest user direction wins');
    expect(context.guidance).toContain('offset=position');
    expect(context.messages.reduce((sum: number, turn: any) => sum + turn.text.length, 0)).toBeLessThanOrEqual(40_000);
    expect(threadMessageText(restored.active.conversation[3]!)).toHaveLength('Detailed inspection. '.repeat(1000).length);
  });

  it('retains the original brief and older decisions after long conversations and restart', () => {
    const { make } = setup(); const threads = make(); threads.load('memory');
    threads.active.conversation.push({ role: 'user', text: 'Keep the title readable for two seconds and use warm colors.' });
    for (let i = 0; i < 40; i++) threads.active.conversation.push({ role: 'assistant', text: `Checked layer ${i}.` });
    threads.active.conversation.push({ role: 'user', text: 'Use cool colors instead; keep the readable hold.' });
    threads.save(); const restored = make(); restored.load('memory');
    const context = conversationForAgent(restored.active.conversation);
    expect(context.find(turn => turn.text.includes('readable for two seconds'))).toBeDefined();
    expect(context.at(-1)?.text).toContain('cool colors instead');
  });

  it('keeps model-controlled settings and retry metadata across restart', () => {
    const { make } = setup(); const threads = make(); threads.load('project');
    Object.assign(threads.active, { provider: 'claude', model: 'claude-sonnet-5-5', reasoningEffort: 'high', access: 'project', lastRunId: 'controlled-run', lastRunStatus: 'completed', orchestration: { callerThreadId: 'parent', clientRequestId: 'launch', fingerprint: 'a'.repeat(64) }, controls: [{ callerThreadId: 'parent', clientRequestId: 'followup', fingerprint: 'b'.repeat(64), runId: 'controlled-run' }] });
    threads.save(); const restored = make(); restored.load('project');
    expect(restored.active).toMatchObject({ provider: 'claude', reasoningEffort: 'high', lastRunId: 'controlled-run', orchestration: threads.active.orchestration, controls: threads.active.controls });
  });

  it('makes full message and task output available for paginated explicit reads', () => {
    const text = 'Complete reply '.repeat(3000);
    expect(threadMessageText({ role: 'assistant', text })).toBe(text);
    expect(threadMessageText({ role: 'trace', steps: [{ kind: 'tool', id: 'task', toolName: 'delegate_task', status: 'done', label: 'Timing review', output: 'Timing is consistent.' }, { kind: 'text', id: 'reply', text: 'Ready.' }] })).toBe('Timing review\nTiming is consistent.\n\nReady.');
  });
  it('creates, switches, and restores independent drafts, attachments, and conversations', () => {
    const { make } = setup(); const threads = make(); threads.load('project-a');
    const first = threads.activeId;
    threads.active.conversation.push({ role: 'user', text: 'First conversation' });
    threads.active.composerDraft = 'Keep this draft';
    threads.active.attachments = [{ id: 'image', name: 'reference.png', dataUrl: 'data:image/png;base64,AA==' }];
    const second = threads.create().id;
    expect(threads.active.conversation).toEqual([]);
    expect(threads.active.composerDraft).toBe('');
    threads.active.composerDraft = 'Second draft'; threads.save();
    const restored = make(); restored.load('project-a');
    expect(restored.activeId).toBe(second);
    expect(restored.active.composerDraft).toBe('Second draft');
    restored.select(first);
    expect(restored.active.conversation[0]?.text).toBe('First conversation');
    expect(restored.active.composerDraft).toBe('Keep this draft');
    expect(restored.active.attachments[0]?.name).toBe('reference.png');
    expect(restored.select('missing')).toBe(false);
    expect(restored.activeId).toBe(first);
  });
  it('reuses a blank thread instead of stacking empties on "new thread"', () => {
    const { make } = setup(); const threads = make(); threads.load('project-a');
    const first = threads.activeId;
    expect(AgentThreads.isBlank(threads.active)).toBe(true);
    // "+" on a thread that has nothing in it stays put.
    expect(threads.create().id).toBe(first);
    expect(threads.threads).toHaveLength(1);
    threads.active.conversation.push({ role: 'user', text: 'Now it exists' });
    const second = threads.create().id;
    expect(second).not.toBe(first);
    expect(threads.threads).toHaveLength(2);
    // Switching back to the old thread and pressing "+" returns to the blank one rather than making a third.
    threads.select(first);
    expect(threads.create().id).toBe(second);
    expect(threads.threads).toHaveLength(2);
    // A draft or an attachment makes a thread real enough to keep.
    threads.active.composerDraft = 'half-typed';
    expect(AgentThreads.isBlank(threads.active)).toBe(false);
    expect(threads.create().id).not.toBe(second);
    expect(threads.threads).toHaveLength(3);
  });
  it('isolates projects and saves a snapshot, not mutable references', () => {
    const { make } = setup(); const threads = make(); threads.load('a');
    threads.active.composerDraft = 'Saved'; threads.save();
    threads.active.composerDraft = 'Not saved';
    threads.load('b'); expect(threads.active.composerDraft).toBe('');
    threads.load('a'); expect(threads.active.composerDraft).toBe('Saved');
  });
  it('recovers from malformed archives and duplicate or unsafe ids', () => {
    const { make, saved } = setup();
    saved.set('agentThreads.a', { version: 1, activeId: 'missing', threads: [null, {}, {id:'../bad', conversation:[], composerDraft:''},
      {id:'ok', conversation:[{role:'user',text:'Hello'}, {role:'invalid'}], composerDraft:'draft'},
      {id:'ok', conversation:[],composerDraft:'duplicate'}] });
    const threads = make(); threads.load('a');
    expect(threads.threads).toHaveLength(1); expect(threads.activeId).toBe('ok');
    expect(threads.active.conversation).toHaveLength(1);
    saved.set('agentThreads.a', 'bad'); threads.load('a');
    expect(threads.active.title).toBe('New thread');
  });
  it('reports storage failure without throwing away in-memory history', () => {
    const { make, store } = setup(); const threads = make(); threads.load('a');
    threads.active.composerDraft = 'Unsaved'; store.set.mockReturnValue(false);
    expect(threads.save()).toBe(false); expect(threads.active.composerDraft).toBe('Unsaved');
  });
  it('titles from the first user request, not traces or responses', () => {
    expect(threadTitle([{ role:'assistant',text:'No' },{role:'user',text:' Make\n  this move '}])).toBe('Make this move');
    expect(threadTitle([])).toBe('New thread');
    expect(threadTitle([{role:'user',text:'a'.repeat(100)}])).toHaveLength(64);
  });
  it('normalizes generated titles and rejects empty or non-text output', () => {
    expect(normalizeGeneratedThreadTitle('  **Premiere-style\nTimeline Setup**  ')).toBe('Premiere-style Timeline Setup');
    expect(normalizeGeneratedThreadTitle('a'.repeat(100))).toHaveLength(64);
    expect(normalizeGeneratedThreadTitle('   ')).toBeNull();
    expect(normalizeGeneratedThreadTitle({ title: 'No' })).toBeNull();
  });
});

it('restores structured mod results with their status and action', () => {
  const { make } = setup(); const threads = make(); threads.load('mods');
  const modResult = { id: 'pexels', name: 'Pexels Browser', action: 'created', status: 'ready' } as const;
  threads.active.conversation.push({ role: 'assistant', text: 'Added mod Pexels Browser', modResult });
  threads.save();
  const restored = make(); restored.load('mods');
  expect(restored.active.conversation[0]?.modResult).toEqual(modResult);
});

it('restores completed work timing and thinking alongside the final reply', () => {
  const { make } = setup(); const threads = make(); threads.load('work-history');
  threads.active.conversation.push({ role: 'trace', durationMs: 543000, steps: [
    { kind: 'thought', id: 'thought', label: 'Check the timing.', live: false },
    { kind: 'text', id: 'answer', text: 'Both stems are ready.' }
  ] });
  threads.save();
  const restored = make(); restored.load('work-history');
  expect(restored.active.conversation[0]).toMatchObject(threads.active.conversation[0]!);
});

it('stores attachment bytes once while draft text and inline positions change', () => {
  const { make, store, saved } = setup(); const threads = make(); threads.load('large');
  const dataBase64 = 'abcd'.repeat(250_000);
  threads.active.attachments = [{ id: 'binary', name: 'scene.aep', dataBase64, promptOffset: 0 }];
  expect(threads.save()).toBe(true);
  store.set.mockClear();
  threads.active.composerDraft = 'Use this file'; threads.active.attachments[0]!.promptOffset = 4;
  expect(threads.save()).toBe(true);
  expect(store.set).toHaveBeenCalledOnce();
  expect(JSON.stringify(saved.get('agentThreads.large')).length).toBeLessThan(1000);
  const restored = make(); restored.load('large');
  expect(restored.active.attachments[0]).toMatchObject({ dataBase64, promptOffset: 4 });
  restored.save();
  expect(store.set.mock.calls.filter(([key]) => key.startsWith('agentAttachment.'))).toHaveLength(0);
});
