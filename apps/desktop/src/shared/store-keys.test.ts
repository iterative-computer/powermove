import { expect, it } from 'vitest';
import { parseStoreKey, storeFileName } from './store-keys';

it('allows project-local thread archives without allowing filesystem traversal', () => {
  const parsed = parseStoreKey('agentThreads.project-123');
  expect(parsed).toEqual({kind:'dynamic',prefix:'agentThreads',id:'project-123'});
  expect(storeFileName(parsed!)).toBe('agentThreads.project-123.json');
  for (const key of ['agentThreads.', 'agentThreads../outside', 'agentThreads.a/b', 'agentThreads.a.b']) {
    expect(parseStoreKey(key)).toBeNull();
  }
});

it('allows compact project recovery journals', () => {
  const parsed = parseStoreKey('projectJournal.project-123');
  expect(parsed).toEqual({ kind: 'dynamic', prefix: 'projectJournal', id: 'project-123' });
  expect(storeFileName(parsed!)).toBe('projectJournal.project-123.json');
});

it('accepts immutable agent attachment payloads with validated ids', () => {
  expect(parseStoreKey('agentAttachment.attachment-123')).toEqual({ kind: 'dynamic', prefix: 'agentAttachment', id: 'attachment-123' });
  expect(parseStoreKey('agentAttachment../escape')).toBeNull();
});
