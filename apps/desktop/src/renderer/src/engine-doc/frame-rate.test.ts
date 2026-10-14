/* The frame-rate validator (shared/frame-rate.ts) against pm-time's
   `rate_from_document`, through pm-wasm (it lives here because it loads the
   renderer's module). */
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { dividesFlick, resolveFrameRate, SUPPORTED_FRAME_RATES } from '../../../shared/frame-rate';
import { instantiate } from './wasm';

const MODULE = fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url));

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
