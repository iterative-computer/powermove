import { describe, expect, it, vi } from 'vitest';
import { FontLoader, MAX_FAILURES } from './fonts';

describe('FontLoader', () => {
  it('gives up a provider that keeps failing, so text stays with the legacy code', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const answer = vi.fn(async () => { throw new Error('No handler registered for engine-fonts:answer'); });
    const loader = new FontLoader({ sync: false, answer, subset: async () => new Uint8Array() });
    const wasm: any = { fontReport: () => ({ misses: [{ family: 'Helvetica' }], pending: [] }) };
    for (let i = 0; i < MAX_FAILURES + 2; i++) {
      expect(loader.resolve(wasm, () => ({ fonts: true }), r => !!r.fonts)).toBeNull();
      await new Promise(r => setTimeout(r, 0));
    }
    expect(loader.disabled).toBe(true);
    expect(answer).toHaveBeenCalledTimes(MAX_FAILURES);
    vi.restoreAllMocks();
  });

  it('keeps a provider whose failures are not consecutive', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    let calls = 0;
    const loader = new FontLoader({
      sync: true,
      answer: () => { calls += 1; if (calls % 2) throw new Error('flaky'); return JSON.stringify({ faces: [], answers: [] }); },
      subset: () => new Uint8Array(),
    });
    const wasm: any = { fontReport: () => ({ misses: [{ family: 'Helvetica' }], pending: [] }), addFontAnswers: () => {} };
    for (let i = 0; i < 8; i++) loader.resolve(wasm, () => ({ fonts: false }), r => r.fonts);
    expect(loader.disabled).toBe(false);
    vi.restoreAllMocks();
  });
});
