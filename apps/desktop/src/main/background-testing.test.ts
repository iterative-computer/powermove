import { describe, expect, it } from 'vitest';
import { backgroundTesting, backgroundWindowOptions } from './background-testing';

describe('background-only Electron testing', () => {
  it('does not alter ordinary windows', () => {
    expect(backgroundTesting({}, '/user/live')).toBe(false);
    expect(backgroundWindowOptions(false)).toEqual({});
  });
  it('fails closed without a separate absolute profile', () => {
    for (const profile of [undefined, '', 'relative', '/user/live', '/user/other/../live']) {
      expect(() => backgroundTesting({ POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_USER_DATA: profile }, '/user/live')).toThrow(/isolated/);
    }
  });
  it('renders while hidden without focus, taskbar, or DevTools', () => {
    expect(backgroundTesting({ POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_USER_DATA: '/tmp/test-profile' }, '/user/live')).toBe(true);
    expect(backgroundWindowOptions(true)).toEqual({
      show: false, skipTaskbar: true, focusable: false,
      webPreferences: { backgroundThrottling: false, devTools: false }
    });
  });
});
