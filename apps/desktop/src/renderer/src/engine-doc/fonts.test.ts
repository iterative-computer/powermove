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

  it('applies the window\'s font cache seeds to pm-wasm now, on change, and to a new instance', async () => {
    let listener: ((json: string) => void) | null = null;
    const changed = vi.fn();
    const loader = new FontLoader({
      sync: false,
      answer: async () => '{}',
      subset: async () => new Uint8Array(),
      seeds: async () => '{"seeds":[1]}',
      onSeedsChanged: (l) => { listener = l; },
    }, changed);
    const applied: string[] = [];
    const wasm: any = { setFontSeeds: (json: string) => { applied.push(json); return true; } };
    let ready = false;
    loader.watchSeeds(() => (ready ? wasm : null));
    await new Promise(r => setTimeout(r, 0));
    // Not ready yet: kept for the instance's start.
    expect(applied).toEqual([]);
    ready = true;
    loader.applySeeds(wasm);
    expect(applied).toEqual(['{"seeds":[1]}']);
    listener!('{"seeds":[2]}');
    expect(applied).toEqual(['{"seeds":[1]}', '{"seeds":[2]}']);
    expect(changed).toHaveBeenCalledTimes(2);
    // An unchanged seed set does not invalidate cached editor answers.
    wasm.setFontSeeds = () => false;
    loader.applySeeds(wasm);
    expect(changed).toHaveBeenCalledTimes(2);
    // A revived instance gets the latest seeds.
    const revived: any = { setFontSeeds: vi.fn() };
    loader.applySeeds(revived);
    expect(revived.setFontSeeds).toHaveBeenCalledWith('{"seeds":[2]}');
  });

  it('does not replace an observed seed change with an older startup reply', async () => {
    let startup!: (json: string) => void;
    let changed!: (json: string) => void;
    const loader = new FontLoader({ sync: false, answer: async () => '{}', subset: async () => new Uint8Array(),
      seeds: () => new Promise<string>(resolve => { startup = resolve; }),
      onSeedsChanged: listener => { changed = listener; },
    });
    const wasm: any = { setFontSeeds: vi.fn(() => true) };
    loader.watchSeeds(() => wasm);
    changed('{"seeds":[2]}');
    startup('{"seeds":[1]}');
    await Promise.resolve();
    expect(wasm.setFontSeeds.mock.calls).toEqual([['{"seeds":[2]}']]);
  });

  it('keeps working when the module or the host has no seeds', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const loader = new FontLoader({ sync: true, answer: () => '{}', subset: () => new Uint8Array(), seeds: () => '{"seeds":[]}' });
    const old: any = { setFontSeeds: () => { throw new Error('pm_fonts_seeds is not a function'); } };
    loader.watchSeeds(() => old);
    loader.applySeeds(old);
    new FontLoader({ sync: true, answer: () => '{}', subset: () => new Uint8Array() }).watchSeeds(() => old);
    vi.restoreAllMocks();
  });
});
