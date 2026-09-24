import { expect, it } from 'vitest';
import { noticeKind, presentError, stated } from './presentation';

it.each([
  ['Error: ENOSPC: no space left on device', 'Your disk is full'],
  ['EACCES: permission denied', 'File access was denied'],
  ['Error invoking remote method \'codex:run\': Error: fetch failed', 'Connection interrupted'],
  ['The coding agent took too long to respond', 'The request timed out'],
  ['Codex CLI is not signed in', 'Sign-in needs attention'],
  ['429 rate limit exceeded', 'Agent is temporarily unavailable'],
  ['The autonomous agent returned an invalid result', 'The agent couldn’t finish the result'],
  ['Error invoking remote method \'codex:run\': IpcValidationError: codex:run: invalid request', 'Something needs attention'],
])('explains %s while retaining its diagnostic', (raw, title) => {
  expect(presentError(raw)).toMatchObject({ title, details: raw });
  expect(presentError(raw).message).not.toContain('Error:');
  expect(presentError(raw).message).not.toContain('codex:run');
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

it('keeps an unrecognised failure an error and honours a stated kind', () => {
  // Nothing states a kind for a transport fault, and softening one would hide
  // a real problem — so anything unlabelled stays an error.
  expect(noticeKind(new Error('ENOSPC: no space left on device'))).toBe('error');
  expect(noticeKind('some string')).toBe('error');
  expect(noticeKind(null)).toBe('error');
  expect(noticeKind(Object.assign(new Error('x'), { notice: 'nonsense' }))).toBe('error');

  const alert = stated('The generated workspace was not safe or complete enough to preview', 'alert');
  expect(noticeKind(alert)).toBe('alert');
  expect(alert.message).toBe('The generated workspace was not safe or complete enough to preview');
  expect(noticeKind(stated('Those panels were already arranged that way', 'plain'))).toBe('plain');
});

it('pulls versions and the fix out of an outdated-client error', () => {
  const raw = 'Claude failed: API Error: 400 Claude Code 2.1.270 does not support this model; version 2.1.280 or newer is required. Run \'claude update\', or update the Claude desktop app, then try again.';
  expect(presentError(raw)).toMatchObject({
    title: 'Update Claude Code to use this model',
    facts: [{ label: 'Installed', value: '2.1.270' }, { label: 'Required', value: '2.1.280 or newer' }],
    update: 'claude',
    details: raw,
  });
});

it('offers the Codex update when Codex says it is too old', () => {
  expect(presentError('This model requires a newer version of Codex. Please upgrade to the latest app or CLI and try again.').update).toBe('codex');
  expect(presentError('Codex 0.150.0 does not support this model; version 0.156.0 or newer is required.')).toMatchObject({ update: 'codex', title: 'Update Codex to use this model' });
});

it('explains a missing Node.js runtime', () => {
  expect(presentError('Codex App Server stopped: env: node: No such file or directory'))
    .toMatchObject({ title: 'Codex needs Node.js to start', update: 'codex', updateLabel: 'Fix it for me' });
});
