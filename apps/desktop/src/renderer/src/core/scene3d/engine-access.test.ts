import { expect, it, vi } from 'vitest';
import { engineGeometry } from './engine-access';

it('fails by name during native startup instead of permitting scene fallback', () => {
  const PM = { EngineDoc: { enabled: true, api: null }, toast: vi.fn() };
  expect(() => engineGeometry(PM, ['camera3d'])).toThrowError(expect.objectContaining({ name: 'EngineFailure' }));
  expect(PM.toast).toHaveBeenCalledOnce();
  PM.EngineDoc.enabled = false;
  expect(engineGeometry(PM, ['camera3d'])).toBeNull();
});
it('requires every geometry member before permitting native use', () => {
  const api = { objectWorlds3d: vi.fn(), objectBoxes3d: vi.fn() };
  const PM = { EngineDoc: { enabled: true, api } };
  expect(engineGeometry(PM, ['objectWorlds3d', 'objectBoxes3d'])).toBe(api);
  expect(() => engineGeometry(PM, ['cursorHit3d'])).toThrowError(expect.objectContaining({ name: 'EngineFailure' }));
});
