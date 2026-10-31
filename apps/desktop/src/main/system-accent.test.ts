import { describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({ systemPreferences: {} }));
import { systemAccentCss } from './system-accent';

describe('systemAccentCss', () => {
  it('turns the AppKit accent into a CSS variable and ignores anything else', () => {
    expect(systemAccentCss('0a82ffff')).toBe(':root{--system-accent:#0A82FF}');
    expect(systemAccentCss('ff5257')).toBe(':root{--system-accent:#FF5257}');
    expect(systemAccentCss('red')).toBeNull();
    expect(systemAccentCss(undefined)).toBeNull();
  });
});
