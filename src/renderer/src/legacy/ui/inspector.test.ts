import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './inspector';

describe('legacy inspector install', () => {
  it('keeps the audio content source-command contract', () => {
    let panel: any;
    const PM: PMRegistry = {
      h() {},
      registerPanel(_id: string, definition: any) { panel = definition; },
      bus: { on() {} },
    };
    install(PM);
    const source = install.toString();

    expect(panel.title).toBe('Properties');
    expect(source).toMatch(/function audioContent/);
    expect(source).toMatch(/type: ["']set_content["']/);
    for (const field of ['asset', 'trim', 'gain', 'fadeIn', 'fadeOut']) {
      expect(source).toMatch(new RegExp(`command\\(["']${field}["']`));
    }
  });
});
