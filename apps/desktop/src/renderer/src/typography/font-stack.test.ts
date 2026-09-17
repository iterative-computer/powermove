import { describe, expect, it } from 'vitest';

import { cssFontStack } from './font-stack';

describe('cssFontStack', () => {
  it('routes Apple system families through the aliases Chromium can resolve', () => {
    expect(cssFontStack('SF Pro Display')).toBe('"SF Pro Display", -apple-system, BlinkMacSystemFont, system-ui');
    expect(cssFontStack('sf pro text')).toContain('-apple-system');
    expect(cssFontStack('SF Mono')).toBe('"SF Mono", ui-monospace, Menlo, monospace');
    expect(cssFontStack('New York')).toBe('"New York", ui-serif, Georgia, serif');
  });

  it('quotes other families and never falls back to the default serif', () => {
    expect(cssFontStack('Geist')).toBe('"Geist", system-ui, sans-serif');
    expect(cssFontStack('JetBrains Mono')).toBe('"JetBrains Mono", ui-monospace, monospace');
    expect(cssFontStack('Playfair Display')).toBe('"Playfair Display", ui-serif, serif');
    expect(cssFontStack('Source Sans 3')).toBe('"Source Sans 3", system-ui, sans-serif');
    expect(cssFontStack('Say "hi"')).toBe('"Say \\"hi\\"", system-ui, sans-serif');
    expect(cssFontStack('')).toBe('inherit');
  });
});
