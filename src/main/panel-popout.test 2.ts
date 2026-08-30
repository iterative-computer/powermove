import { describe, expect, it } from 'vitest';

import { isPanelPopoutRequest } from './panel-popout';

describe('panel pop-out window boundary', () => {
  it('allows only named about:blank children from the trusted editor', () => {
    expect(isPanelPopoutRequest('about:blank', 'pm-panel-notes', true)).toBe(true);
    expect(isPanelPopoutRequest('https://example.com', 'pm-panel-notes', true)).toBe(false);
    expect(isPanelPopoutRequest('about:blank', 'untrusted-window', true)).toBe(false);
    expect(isPanelPopoutRequest('about:blank', 'pm-panel-notes', false)).toBe(false);
    expect(isPanelPopoutRequest('about:blank', 'pm-panel-notes<script>', true)).toBe(false);
  });
});
