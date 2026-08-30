import { describe, expect, it, vi } from 'vitest';
import { AgentThreads, normalizeGeneratedThreadTitle, threadTitle } from './threads';

function setup() {
  const saved = new Map<string, unknown>(); let id = 0;
  const store = { get: (k: string, d: unknown) => saved.get(k) ?? d, set: vi.fn((k: string, v: unknown) => { saved.set(k, v); return true; }) };
  const make = () => new AgentThreads(store, () => `thread-${++id}`);
  return { saved, store, make };
}

describe('agent thread archive', () => {
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
