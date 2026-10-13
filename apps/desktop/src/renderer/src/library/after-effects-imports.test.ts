import { describe, expect, it, vi } from 'vitest';
import { afterEffectsExtensionIds, recordAfterEffectsExtensions } from './after-effects-imports';

describe('After Effects import provenance', () => {
  it('keeps created and updated extensions across sessions, deduplicates retries and removes deleted items', () => {
    const saved: Record<string, unknown> = {};
    const store = { get: (key: string, fallback?: unknown) => saved[key] ?? fallback, set: (key: string, value: unknown) => { saved[key] = value; } };
    const PM = { store, bus: { emit: vi.fn() } };
    recordAfterEffectsExtensions(PM, [{ id: 'timing-tools', action: 'created' }, { id: 'align-tools', action: 'updated' }]);
    recordAfterEffectsExtensions(PM, [{ id: 'timing-tools', action: 'updated' }, { id: 'align-tools', action: 'removed' }]);
    expect([...afterEffectsExtensionIds({ store })]).toEqual(['timing-tools']);
    expect(PM.bus.emit).toHaveBeenCalledWith('after-effects:imported');
    recordAfterEffectsExtensions(PM, [{ id: 'other-tool', action: 'unchanged' }, { id: '../invalid', action: 'created' }]);
    expect([...afterEffectsExtensionIds(PM)]).toEqual(['timing-tools']);
  });

  it('tolerates missing or malformed saved provenance', () => {
    for (const saved of [undefined, null, {}, 'timing-tools']) {
      expect([...afterEffectsExtensionIds({ store: { get: () => saved, set() {} } })]).toEqual([]);
    }
    expect([...afterEffectsExtensionIds({ store: { get: () => ['timing-tools', 1, '../invalid', 'timing-tools'], set() {} } })]).toEqual(['timing-tools']);
  });
});
