import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { dividesFlick, nominalRate, resolveFrameRate, SUPPORTED_FRAME_RATES, validateFrameRate } from './frame-rate';
import { instantiate } from '../renderer/src/engine-doc/wasm';

describe('the frame-rate validator', () => {
  it('snaps floats to the supported rates', () => {
    expect(resolveFrameRate(29.97)).toEqual({ num: 30000, den: 1001, source: 'ntsc' });
    expect(resolveFrameRate(23.976)).toEqual({ num: 24000, den: 1001, source: 'ntsc' });
    expect(resolveFrameRate(59.94)).toEqual({ num: 60000, den: 1001, source: 'ntsc' });
    expect(resolveFrameRate(25.004)).toEqual({ num: 25, den: 1, source: 'integer' });
    expect(resolveFrameRate(12.5)).toEqual({ num: 25, den: 2, source: 'approximated' });
    expect(resolveFrameRate(30, [24000, 1001])).toEqual({ num: 24000, den: 1001, source: 'exact' });
    expect(resolveFrameRate(30, [0, 1])).toEqual({ num: 30, den: 1, source: 'integer' });
    expect(resolveFrameRate(-1)).toBeNull();
    for (const [num, den] of SUPPORTED_FRAME_RATES) expect(dividesFlick({ num, den })).toBe(true);
  });

  it('stores new rates exactly and refuses rates that do not divide the flick', () => {
    expect(validateFrameRate(29.97)).toEqual({ ok: true, fps: 30000 / 1001, fpsRational: [30000, 1001], source: 'ntsc' });
    expect(validateFrameRate('48')).toEqual({ ok: true, fps: 48, fpsRational: [48, 1], source: 'integer' });
    expect(validateFrameRate(12)).toMatchObject({ ok: true, fps: 12, fpsRational: [12, 1] });
    expect(validateFrameRate(11)).toMatchObject({ ok: false });
    expect(validateFrameRate(0.5)).toMatchObject({ ok: false });
    expect(validateFrameRate(241)).toMatchObject({ ok: false });
    expect(validateFrameRate('abc')).toMatchObject({ ok: false });
  });

  it('keeps document rates working and matches media to nominal rates', () => {
    expect(validateFrameRate(29.97, 'document')).toEqual({ ok: true, fps: 29.97, fpsRational: [30000, 1001], source: 'ntsc' });
    expect(validateFrameRate(11, 'document')).toEqual({ ok: true, fps: 11, fpsRational: [11, 1], source: 'approximated' });
    expect(validateFrameRate(29.83, 'media')).toEqual({ ok: true, fps: 30000 / 1001, fpsRational: [30000, 1001], source: 'ntsc' });
    expect(validateFrameRate(24.000001, 'media')).toMatchObject({ fpsRational: [24, 1] });
    expect(nominalRate(14.99)).toEqual([24000, 1001]);
  });
});

const MODULE = fileURLToPath(new URL('../renderer/public/engine/pm_wasm.wasm', import.meta.url));

describe.skipIf(!existsSync(MODULE))('agreement with pm-time (through pm-wasm)', () => {
  it('resolves every rate exactly as rate_from_document does', async () => {
    const wasm = await instantiate(await WebAssembly.compile(readFileSync(MODULE)));
    let seed = 7;
    const random = () => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed / 2147483648; };
    const inputs: Array<[number, unknown]> = [];
    for (const [n, d] of SUPPORTED_FRAME_RATES) inputs.push([n / d, undefined], [Number((n / d).toFixed(3)), undefined], [Number((n / d).toFixed(2)), undefined], [n / d, [n, d]]);
    for (let i = 0; i < 3000; i++) {
      const roll = random();
      const fps = roll < 0.3 ? Math.round(random() * 2400) / 10 : roll < 0.6 ? random() * 300 : roll < 0.8 ? Math.floor(random() * 240) + 1 : random() * 1e-6 + 1e-9;
      inputs.push([fps, random() < 0.1 ? [Math.floor(random() * 300000) - 10, Math.floor(random() * 2002) - 1] : undefined]);
    }
    for (const [fps, rational] of inputs) {
      const ours = resolveFrameRate(fps, rational);
      const reply = wasm.query({ op: 'frameRate', fps, ...(rational === undefined ? {} : { fpsRational: rational }) });
      const theirs = reply.ok;
      if (theirs === null) expect(ours, `${fps} ${JSON.stringify(rational)}`).toBeNull();
      else {
        expect(ours, `${fps} ${JSON.stringify(rational)}`).toEqual({ num: theirs.num, den: theirs.den, source: theirs.source });
        expect(dividesFlick(ours!)).toBe(theirs.dividesFlick);
      }
    }
  });
});
