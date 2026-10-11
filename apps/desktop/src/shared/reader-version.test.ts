import { describe, expect, it } from 'vitest';

import { CURRENT_READER_VERSION, NEWER_PROJECT_MESSAGE, UNKNOWN_READER_VERSION, needsNewerReader, requiredReaderVersion } from './reader-version';

describe('minReaderVersion guard', () => {
  it('is version 1 with the agreed message', () => {
    expect(CURRENT_READER_VERSION).toBe(1);
    expect(NEWER_PROJECT_MESSAGE).toBe('This project needs a newer version of Powermove');
  });

  it('reads the stamp from the project root and the envelope root', () => {
    expect(requiredReaderVersion({ proj: { layers: [] } })).toBe(0);
    expect(requiredReaderVersion({ layers: [] })).toBe(0);
    expect(requiredReaderVersion({ proj: { minReaderVersion: 2 } })).toBe(2);
    expect(requiredReaderVersion({ minReaderVersion: 3, proj: {} })).toBe(3);
    expect(requiredReaderVersion({ minReaderVersion: 2, proj: { minReaderVersion: 5 } })).toBe(5);
    expect(requiredReaderVersion({ minReaderVersion: 4, layers: [] })).toBe(4);
  });

  it('opens current and older stamps normally', () => {
    for (const document of [{}, { proj: {} }, { proj: { minReaderVersion: 1 } }, { minReaderVersion: 0 }, { proj: { minReaderVersion: null } }, null, undefined, 'text', []]) {
      expect(needsNewerReader(document)).toBe(false);
    }
  });

  it('opens newer and unreadable stamps read-only', () => {
    expect(needsNewerReader({ proj: { minReaderVersion: 2 } })).toBe(true);
    expect(needsNewerReader({ minReaderVersion: 1.5, proj: {} })).toBe(true);
    for (const value of ['2', Number.NaN, Number.POSITIVE_INFINITY, {}, true]) {
      expect(requiredReaderVersion({ proj: { minReaderVersion: value } })).toBe(UNKNOWN_READER_VERSION);
      expect(needsNewerReader({ proj: { minReaderVersion: value } })).toBe(true);
    }
    // The normalized value survives JSON, so local recovery keeps the guard.
    expect(JSON.parse(JSON.stringify({ minReaderVersion: UNKNOWN_READER_VERSION })).minReaderVersion).toBe(UNKNOWN_READER_VERSION);
  });

  it('compares against an explicit reader version', () => {
    expect(needsNewerReader({ proj: { minReaderVersion: 2 } }, 2)).toBe(false);
    expect(needsNewerReader({ proj: { minReaderVersion: 3 } }, 2)).toBe(true);
  });
});
