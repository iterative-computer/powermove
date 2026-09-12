import { expect, it } from 'vitest';
import { presentError } from './presentation';

it.each([
  ['Error: ENOSPC: no space left on device', 'Your disk is full'],
  ['EACCES: permission denied', 'File access was denied'],
  ['Error invoking remote method \'codex:run\': Error: fetch failed', 'Connection interrupted'],
  ['The coding agent took too long to respond', 'The request timed out'],
  ['Codex CLI is not signed in', 'Sign-in needs attention'],
  ['429 rate limit exceeded', 'Agent is temporarily unavailable'],
  ['The autonomous agent returned an invalid result', 'The agent couldn’t finish the result'],
])('explains %s while retaining its diagnostic', (raw, title) => {
  expect(presentError(raw)).toMatchObject({ title, details: raw });
  expect(presentError(raw).message).not.toContain('Error:');
});

it('retains useful application context and keeps raw exception text in details', () => {
  expect(presentError('Could not import clip.mov').message).toBe('Could not import clip.mov');
  const error = new Error('TypeError: cannot read property foo of undefined');
  expect(presentError(error).message).not.toContain('TypeError');
  expect(presentError(error).details).toBe(error.message);
});

it('does not mistake a provider writer lock for another active Powermove conversation', () => {
  const raw = 'Error: thread/resume: thread abc already has an active writer (code -32600)';
  expect(presentError(raw).title).toBe('Agent session couldn’t reopen');
  expect(presentError(raw).message).not.toContain('current run');
  expect(presentError('An agent is already running in this conversation.').title).toBe('Conversation is busy');
});
