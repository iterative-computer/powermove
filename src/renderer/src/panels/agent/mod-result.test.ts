import { expect, it } from 'vitest';
import { modResultForMessage } from './mod-result';

const created = { id: 'pexels', name: 'Pexels Browser', action: 'created', status: 'ready' } as const;
const panels = [{ ownerId: 'pexels' }];

it('shows successful typed agent panel creations only', () => {
  expect(modResultForMessage({ role: 'assistant', modResult: created }, panels)).toEqual(created);
  expect(modResultForMessage({ role: 'user', modResult: created }, panels)).toBeNull();
  expect(modResultForMessage({ role: 'assistant', modResult: created }, [])).toBeNull();
  expect(modResultForMessage({ role: 'assistant', modResult: created }, [{ ownerId: 'other' }])).toBeNull();
});

it('does not announce removals or failed changes', () => {
  for (const action of ['removed'] as const) {
    expect(modResultForMessage({ role: 'assistant', modResult: { ...created, action } }, panels)).toBeNull();
  }
  for (const status of ['error', 'removed'] as const) {
    expect(modResultForMessage({ role: 'assistant', modResult: { ...created, status } }, panels)).toBeNull();
  }
});

it('does not infer agent panel creation from legacy notices or arbitrary prose', () => {
  for (const text of ['Added mod Pexels Browser', 'Updated mod Pexels Browser', 'I created Pexels Browser']) {
    expect(modResultForMessage({ role: 'assistant', text }, panels)).toBeNull();
  }
});

it('shows edits only when the agent result owns a registered panel', () => {
  const modResult = { ...created, action: 'updated' } as const;
  expect(modResultForMessage({ role: 'assistant', modResult }, panels)).toEqual(modResult);
  expect(modResultForMessage({ role: 'assistant', modResult }, [])).toBeNull();
  expect(modResultForMessage({ role: 'user', modResult }, panels)).toBeNull();
});
