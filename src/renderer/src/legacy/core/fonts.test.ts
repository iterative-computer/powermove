import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './fonts';

function fontsRegistry(): { PM: PMRegistry; events: any[] } {
  const events: any[] = [];
  vi.stubGlobal('window', {
    document: { fonts: undefined },
    console,
  });
  const PM: PMRegistry = {
    bus: { emit(...args: any[]) { events.push(args); } },
    invalidate() {},
  };
  install(PM);
  return { PM, events };
}

afterEach(() => vi.unstubAllGlobals());

describe('legacy fonts install', () => {
  it('publishes bundled and cleaned system families to the searchable picker', () => {
    const { PM, events } = fontsRegistry();

    PM.Fonts.setSystemFamilies(['Zapfino', 'Arial', ' Zapfino ', '\u0000Menlo']);

    expect(PM.Fonts.bundled).toEqual(['Geist', 'Geist Mono']);
    expect(PM.Fonts.system).toEqual(['Arial', 'Menlo', 'Zapfino']);
    expect(PM.Fonts.families.slice(0, 5)).toEqual(['Geist', 'Geist Mono', 'Arial', 'Menlo', 'Zapfino']);
    expect(PM.Fonts.options(' Zapfino ')[0]).toBe('Zapfino');
    expect(events).toEqual([['fonts', PM.Fonts.families]]);
  });
});
