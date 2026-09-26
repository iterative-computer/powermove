import { describe, expect, it } from 'vitest';

import type { ExtensionVarDecl } from '../../shared/extensions';
import { missingKeys, resolveVars } from './resolve';
import type { EnvEntry } from './store';

const decls: ExtensionVarDecl[] = [
  { key: 'API_KEY', label: 'API key', secret: true, required: true },
  { key: 'REGION', label: 'Region' },
  { key: 'ACCOUNT', label: 'Account', required: true }
];

const env = (entries: Record<string, EnvEntry>): Map<string, EnvEntry> => new Map(Object.entries(entries));

describe('resolveVars', () => {
  it('delivers declared values and ignores undeclared ones', () => {
    const result = resolveVars(decls, env({
      API_KEY: { value: 'sk', secret: true },
      ACCOUNT: { value: 'acme', secret: false },
      OTHER: { value: 'leak', secret: false }
    }));
    expect(result).toEqual({ values: { API_KEY: 'sk', ACCOUNT: 'acme' }, missingRequired: [], undecryptable: [], status: 'ok' });
  });

  it('accepts missing and empty values, including legacy required declarations', () => {
    const result = resolveVars(decls, env({ API_KEY: { value: '', secret: true }, REGION: { value: 'eu', secret: false } }));
    expect(result.status).toBe('ok');
    expect(result.missingRequired).toEqual([]);
    expect(result.values).toEqual({ REGION: 'eu' });
  });

  it('omits undecryptable values without blocking other values', () => {
    const result = resolveVars(decls, env({
      API_KEY: { value: null, secret: true },
      ACCOUNT: { value: 'acme', secret: false },
      REGION: { value: null, secret: true }
    }));
    expect(result.status).toBe('ok');
    expect(result.missingRequired).toEqual([]);
    expect(result.undecryptable).toEqual(['API_KEY', 'REGION']);
    expect(missingKeys(result)).toEqual(['API_KEY', 'REGION']);
  });

  it('accepts one of two legacy required values, or neither', () => {
    expect(resolveVars(decls, env({ API_KEY: { value: 'one-key', secret: true } }))).toMatchObject({ status: 'ok', values: { API_KEY: 'one-key' }, missingRequired: [] });
    expect(resolveVars(decls, new Map())).toMatchObject({ status: 'ok', values: {}, missingRequired: [] });
  });

  it('is ok with nothing declared', () => {
    expect(resolveVars(undefined, env({ A_KEY: { value: 'x', secret: false } }))).toEqual({ values: {}, missingRequired: [], undecryptable: [], status: 'ok' });
  });
});
