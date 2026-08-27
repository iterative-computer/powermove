import { beforeEach, describe, expect, it } from 'vitest';
import type { ExtensionRecord } from '../../../shared/extensions';
import { activeIds, extensionsStore, patchRecord, recordFor, records, resetExtensionsStore, setActiveIds, setHealth, setRecords } from './extensions.svelte';

const rec = (id: string): ExtensionRecord => ({
  id,
  scope: 'user',
  manifest: { id, name: id, version: '1.0.0', apiVersion: 1 },
  dir: `/ext/${id}`,
  enabled: true,
  bundleUrl: null,
  bundleHash: null,
  health: { state: 'ok' },
  updatedAt: 0
});

beforeEach(() => resetExtensionsStore());

describe('extensions store', () => {
  it('copies the incoming list so callers cannot mutate the store by reference', () => {
    const incoming = [rec('a'), rec('b')];
    setRecords(incoming);
    incoming.push(rec('c'));
    expect(records().map((record) => record.id)).toEqual(['a', 'b']);
    expect(recordFor('b')?.manifest?.name).toBe('b');
    expect(recordFor('nope')).toBeUndefined();
  });

  it('patches one record in place, keeping list order and replacing the reference', () => {
    setRecords([rec('a'), rec('b')]);
    const before = recordFor('b');
    const patched = patchRecord('b', { enabled: false });

    expect(records().map((record) => record.id)).toEqual(['a', 'b']);
    expect(patched?.enabled).toBe(false);
    expect(recordFor('b')).not.toBe(before);
    expect(recordFor('a')?.enabled).toBe(true);
    expect(patchRecord('missing', { enabled: false })).toBeUndefined();
  });

  it('records health and exposes a lookup map', () => {
    setRecords([rec('a')]);
    setHealth('a', { state: 'runtime-error', error: 'kaboom' });
    expect(recordFor('a')?.health).toEqual({ state: 'runtime-error', error: 'kaboom' });
    expect(extensionsStore.byId.get('a')?.id).toBe('a');
  });

  it('tracks active ids independently of records', () => {
    setActiveIds(['a', 'b']);
    expect(activeIds()).toEqual(['a', 'b']);
    resetExtensionsStore();
    expect(activeIds()).toEqual([]);
    expect(records()).toEqual([]);
  });
});
