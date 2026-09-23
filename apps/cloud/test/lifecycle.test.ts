import { expect, test } from 'bun:test';
import { canBrowse, canReadFiles, canViewDetail, type RepoState, updateOffered } from '../src/lifecycle';
const owner = { publisherId: 'owner', admin: false }, other = { publisherId: null, admin: false };
const base: RepoState = { visibility: 'public', moderation: 'none', tombstonedAt: null, ownerId: 'owner' };
test.each(
  [
    ['public', {}, true, 'ok', 'ok', true],
    ['unlisted', { visibility: 'unlisted' }, false, 'ok', 'ok', true],
    ['hidden', { moderation: 'hidden' }, false, 'not_found', 'not_found', false],
    ['removed', { moderation: 'removed' }, false, 'gone_removed', 'gone_removed', false],
    ['yanked', {}, true, 'ok', 'gone_yanked', false],
    ['tombstoned', { tombstonedAt: new Date() }, false, 'gone_tombstoned', 'gone_tombstoned', false],
  ] as const,
)('%s lifecycle', (_name, patch, browse, detail, read, offered) => {
  const repo = { ...base, ...patch } as RepoState, release = { yankedAt: _name === 'yanked' ? new Date() : null };
  expect(canBrowse(repo, other)).toBe(browse);
  expect(canViewDetail(repo, other)).toBe(detail);
  expect(canReadFiles(repo, release, other)).toBe(read);
  expect(updateOffered(repo, release)).toBe(offered);
  if (_name === 'hidden') {
    expect(canViewDetail(repo, owner)).toBe('ok');
    expect(canReadFiles(repo, release, owner)).toBe('ok');
  }
});
