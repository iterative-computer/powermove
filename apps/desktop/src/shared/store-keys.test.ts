import { expect, it } from 'vitest';
import { parseStoreKey, storeFileName } from './store-keys';

it('accepts the global panel visibility preference', () => {
  expect(parseStoreKey('panelVisibility')).toEqual({ kind: 'static', key: 'panelVisibility' });
});

it('persists the imported workspace preference', () => {
  const parsed = parseStoreKey('defaultWorkspace');
  expect(parsed).toEqual({ kind: 'static', key: 'defaultWorkspace' });
  expect(storeFileName(parsed!)).toBe('defaultWorkspace.json');
});

it('persists After Effects extension provenance', () => {
  const parsed = parseStoreKey('afterEffectsExtensions');
  expect(parsed).toEqual({ kind: 'static', key: 'afterEffectsExtensions' });
  expect(storeFileName(parsed!)).toBe('afterEffectsExtensions.json');
});

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

it('persists the agent approval mode (an unknown key fails the quit flush and keeps the app open)', () => {
  expect(parseStoreKey('agentApprovalMode')).toEqual({ kind: 'static', key: 'agentApprovalMode' });
});
