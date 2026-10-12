import { describe, expect, it, vi } from 'vitest';

import { backgroundWindowOptions } from '../main/background-testing';
import { publishTestBridge, TEST_BRIDGE_GLOBAL, TEST_BRIDGE_SWITCH } from './test-bridge';

describe('e2e bridge seam', () => {
  it('publishes nothing without the switch', () => {
    const expose = vi.fn();
    expect(publishTestBridge(['electron', '--type=renderer'], expose, { store: {} })).toBe(false);
    expect(expose).not.toHaveBeenCalled();
  });

  it('publishes the bridge under the test global with the switch', () => {
    const expose = vi.fn();
    const bridge = { store: {} };
    expect(publishTestBridge(['electron', TEST_BRIDGE_SWITCH], expose, bridge)).toBe(true);
    expect(expose).toHaveBeenCalledExactlyOnceWith(TEST_BRIDGE_GLOBAL, bridge);
    expect(TEST_BRIDGE_GLOBAL).not.toBe('powermove');
  });

  it('only hidden test windows pass the switch', () => {
    expect(backgroundWindowOptions(true).webPreferences?.additionalArguments).toEqual([TEST_BRIDGE_SWITCH]);
    expect(backgroundWindowOptions(false).webPreferences?.additionalArguments).toBeUndefined();
  });
});
