import { describe, expect, it } from 'vitest';
import type { ExtensionDetailDto, ListingDto } from '@powermove/registry/wire';

import { CLOUD_UNREACHABLE } from '../../../shared/cloud-ipc';
import type { LibraryItemDto } from '../../../shared/store-ipc';
import {
  KINDS, KIND_PLURAL, artFor, detailAction, detailFromDto, groupLibrary, includesText, libraryAction, listingFromDto,
  loadError, parseLineage, relativeDate, requiresText, statusText
} from './data';

const REPO = '11111111-1111-4111-8111-111111111111';
const R1 = '33333333-3333-4333-8333-333333333333';
const R2 = '44444444-4444-4444-8444-444444444444';

function item(overrides: Partial<LibraryItemDto> = {}): LibraryItemDto {
  return {
    localId: 'glass-blur', name: 'Glass blur', version: '1.0.0', category: 'effects', contributes: ['effects'], vars: [],
    health: { state: 'ok' }, enabled: true, description: null, group: 'store', maker: { handle: 'mara' },
    origin: { coordinate: 'mara/glass-blur', version: '1.0.0', repoId: REPO, releaseId: R1 },
    update: null, modified: false,
    ...overrides
  };
}

type Update = NonNullable<LibraryItemDto['update']>;
const available = (modified = false): Update => ({ version: '1.1.0', releaseId: R2, modified, state: 'available' });
const staged: Update = { version: '1.1.0', releaseId: R2, modified: true, state: 'staged-for-merge' };
const MY_REPO = '77777777-7777-4777-8777-777777777777';
const mine = { coordinate: 'jude/glass-blur', version: '1.0.0', releaseId: R2, repoId: MY_REPO };
const fork = { coordinate: 'mara/glass-blur', version: '1.0.0', releaseId: R1, upstreamReleaseId: R1 };
const required = [{ key: 'OPENAI_API_KEY', label: 'OpenAI API key', secret: true, required: true }];
const optional = [{ key: 'PALETTE_SIZE', label: 'Palette size' }];

describe('action labels', () => {
  /* state → the detail's primary action and the Library row's trailing control. */
  const matrix: Array<[state: string, input: Parameters<typeof detailAction>[0], detail: string, row: string | null]> = [
    ['not installed', {}, 'Install', null],
    ['not installed, optional values', { vars: optional }, 'Install', null],
    ['not installed, required values', { vars: required }, 'Install and set up', null],
    ['installed', { item: item() }, 'Installed', 'On'],
    ['installed, turned off', { item: item({ enabled: false }) }, 'Installed', 'Off'],
    ['installed, needs setup', { item: item({ health: { state: 'needs-setup', missing: ['OPENAI_API_KEY'] } }) }, 'Set up', 'Set up'],
    ['update available', { item: item({ update: available() }) }, 'Update', 'Update'],
    ['update available, files changed', { item: item({ modified: true, update: available(true) }) }, 'Update…', 'Update…'],
    ['update staged beside the folder', { item: item({ modified: true, update: staged }) }, 'Update…', 'On'],
    ['no longer on the store', { item: item({ removed: true }) }, 'Installed', 'On'],
    ['yours', { item: item({ group: 'yours', maker: { you: true } }) }, 'Open', 'On'],
    ['yours, never published', { item: item({ group: 'yours', maker: { you: true }, publish: 'first' }) }, 'Publish…', 'Publish…'],
    ['yours, changed since publishing', { item: item({ group: 'yours', maker: { you: true }, publish: 'update', published: mine }) }, 'Publish Update…', 'Publish Update…'],
    ['yours, published, unchanged', { item: item({ group: 'yours', maker: { you: true }, publish: null, published: mine }) }, 'Open', 'On'],
    ['your fork, on the original’s page', { item: item({ group: 'yours', maker: { you: true }, published: mine, fork }), repoId: REPO }, 'Forked', 'On'],
    ['built in', { item: item({ group: 'builtin', maker: { builtin: true } }) }, 'Open', 'On']
  ];

  it.each(matrix)('%s', (_state, input, detail, row) => {
    expect(detailAction(input).label).toBe(detail);
    if (input.item) expect(libraryAction(input.item).label).toBe(row);
  });

  it('matches the matrix', () => {
    const table = matrix.map(([state, input]) =>
      `${state.padEnd(34)}| ${detailAction(input).label.padEnd(19)}| ${input.item ? libraryAction(input.item).label : '—'}`).join('\n');
    expect(`\n${table}\n`).toMatchInlineSnapshot(`
      "
      not installed                     | Install            | —
      not installed, optional values    | Install            | —
      not installed, required values    | Install and set up | —
      installed                         | Installed          | On
      installed, turned off             | Installed          | Off
      installed, needs setup            | Set up             | Set up
      update available                  | Update             | Update
      update available, files changed   | Update…            | Update…
      update staged beside the folder   | Update…            | On
      no longer on the store            | Installed          | On
      yours                             | Open               | On
      yours, never published            | Publish…           | Publish…
      yours, changed since publishing   | Publish Update…    | Publish Update…
      yours, published, unchanged       | Open               | On
      your fork, on the original’s page | Forked             | On
      built in                          | Open               | On
      "
    `);
  });

  it('keeps Open disabled until it does something', () => {
    expect(detailAction({ item: item({ group: 'yours', maker: { you: true } }) }).disabled).toBe(true);
  });
});

describe('library', () => {
  it('groups store installs, yours and built-ins, in that order', () => {
    const groups = groupLibrary([
      item({ localId: 'timeline', name: 'Timeline', group: 'builtin', maker: { builtin: true } }),
      item({ localId: 'zoom', name: 'Zoom', group: 'store' }),
      item({ localId: 'ease-lab', name: 'Ease lab', group: 'yours', maker: { you: true } }),
      item({ localId: 'blur', name: 'Blur', group: 'store' })
    ]);
    expect(groups.store.map((i) => i.localId)).toEqual(['blur', 'zoom']);
    expect(groups.yours.map((i) => i.localId)).toEqual(['ease-lab']);
    expect(groups.builtin.map((i) => i.localId)).toEqual(['timeline']);
  });

  it('says what is up on the third line', () => {
    expect(statusText(item())).toBe('Installed 1.0.0');
    expect(statusText(item({ update: available() }))).toBe('Installed 1.0.0 · Update to 1.1.0');
    expect(statusText(item({ modified: true, update: available(true) }))).toBe('You changed the files · Update to 1.1.0');
    expect(statusText(item({ modified: true, update: staged }))).toBe('You changed the files · 1.1.0 is beside your folder');
    expect(statusText(item({ modified: true }))).toBe('You changed the files');
    expect(statusText(item({ health: { state: 'needs-setup', missing: ['X'] } }))).toBe('Needs setup');
    expect(statusText(item({ removed: true, update: available() }))).toBe('No longer on the store');
    expect(statusText(item({ group: 'yours', maker: { you: true }, published: { coordinate: null, version: '0.3.0', releaseId: R2, repoId: REPO } }))).toBe('Published 0.3.0');
    expect(statusText(item({ group: 'yours', maker: { you: true }, modified: true, publish: 'update', published: { coordinate: 'jude/glass-blur', version: '0.3.0', releaseId: R2, repoId: REPO } })))
      .toBe('Published 0.3.0 · Changed since');
    expect(statusText(item({ group: 'builtin', maker: { builtin: true } }))).toBeNull();
  });
});

describe('view models', () => {
  const listing: ListingDto = {
    repoId: REPO, owner: { id: '22222222-2222-4222-8222-222222222222', handle: 'mara', tombstoned: false }, slug: 'glass-blur',
    name: 'Glass blur', tagline: 'Frosted glass.', category: 'tools', iconUrl: null, visibility: 'public',
    latest: { id: R2, version: '1.1.0', publishedAt: new Date(2026, 8, 22, 10).toISOString(), apiVersion: 2, yankedAt: null },
    installCount: 3, forkCount: 0, licence: 'MIT',
    forkedFrom: { repoId: '55555555-5555-4555-8555-555555555555', handle: 'noor', slug: 'glass', releaseId: R1, version: '1.0.0' },
    createdAt: '2026-09-01T00:00:00.000Z', updatedAt: '2026-09-22T10:00:00.000Z'
  };
  // Local times: relative dates count calendar days where you are.
  const now = new Date(2026, 8, 23, 12);

  it('builds a listing from the DTO and the Library', () => {
    const vm = listingFromDto(listing, [item()], now);
    expect(vm).toMatchObject({
      publisher: 'mara', id: 'glass-blur', kind: 'tools', version: '1.1.0', updated: 'Yesterday', installed: true,
      latestReleaseId: R2, forkedFrom: { handle: 'noor', slug: 'glass', version: '1.0.0', releaseId: R1 }
    });
    expect(listingFromDto({ ...listing, forkedFrom: null }, [], now)).not.toHaveProperty('forkedFrom');
    expect(listingFromDto(listing, [], now).installed).toBe(false);
  });

  it('marks withdrawn versions and reads vars from the latest release', () => {
    const release = (id: string, version: string, yankedAt: string | null) => ({
      id, repoId: REPO, version, commitSha: 'c'.repeat(40), treeSha: 't'.repeat(40), tarSha256: 'a'.repeat(64), apiVersion: 2,
      fileCount: 2, sizeBytes: 10, notes: `Notes for ${version}`, publishedAt: '2026-09-20T00:00:00.000Z', yankedAt, basedOnReleaseId: null,
      manifest: { id: 'glass-blur', name: 'Glass blur', version, apiVersion: 2, contributes: ['effects', 'inspector'], vars: required, forkedFrom: null, description: null }
    });
    const dto: ExtensionDetailDto = { ...listing, about: null, moderation: 'none', releases: [release(R2, '1.1.0', null), release(R1, '1.0.0', '2026-09-21T00:00:00.000Z')] };
    const vm = detailFromDto(dto, [], now);
    expect(vm.versions.map((v) => [v.version, v.withdrawn])).toEqual([['1.1.0', false], ['1.0.0', true]]);
    expect(vm.vars).toEqual(required);
    expect(includesText(vm.contributes)).toBe('Effects, Inspector');
    expect(detailAction({ vars: vm.vars }).label).toBe('Install and set up');
  });

  it('writes requirements and dates the way people say them', () => {
    expect(requiresText(2)).toBe('Powermove 1.1 or later');
    expect(requiresText(1)).toBe('Powermove 1.0 or later');
    expect(relativeDate(new Date(2026, 8, 23, 1).toISOString(), now)).toBe('Today');
    expect(relativeDate(new Date(2026, 8, 19, 12).toISOString(), now)).toBe('4 days ago');
    expect(relativeDate(new Date(2026, 7, 2, 12).toISOString(), now)).toBe('Aug 2');
    expect(relativeDate(new Date(2025, 7, 2, 12).toISOString(), now)).toBe('Aug 2, 2025');
  });

  it('draws the same artwork for the same repo', () => {
    expect(artFor(REPO)).toEqual(artFor(REPO));
    expect(artFor(REPO)).not.toEqual(artFor(R1));
  });

  it('tells a store fork from a built-in fork', () => {
    expect(parseLineage('mara/glass-blur@1.0.0')).toEqual({ store: { handle: 'mara', slug: 'glass-blur', version: '1.0.0', releaseId: null } });
    expect(parseLineage('timeline@1.0.0')).toEqual({ builtin: { id: 'timeline', version: '1.0.0' } });
    expect(parseLineage('nonsense')).toBeNull();
  });

  it('has a Tools shelf', () => {
    expect(KINDS).toContain('tools');
    expect(KIND_PLURAL.tools).toBe('Tools');
  });

  it('tells offline from a registry error', () => {
    expect(loadError({ error: 'internal', detail: CLOUD_UNREACHABLE })).toEqual({ offline: true, message: 'Can’t reach the store' });
    expect(loadError({ error: 'gone', reason: 'removed' })).toEqual({ offline: false, message: 'This extension is no longer on the store.' });
  });
});
