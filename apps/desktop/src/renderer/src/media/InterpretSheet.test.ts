// @vitest-environment happy-dom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { EngineProbeResult } from '../../../shared/engine-ipc';
import type { MediaFacts } from '../../../shared/engine-protocol';
import InterpretSheet from './InterpretSheet.svelte';

/** The job engine's answer for a 720 × 480 anamorphic clip, read with `interpret`. */
function factsFor(interpret: Record<string, unknown> | null): MediaFacts {
  const square = Array.isArray(interpret?.['pixelAspect']);
  return {
    kind: 'video',
    facts: [{ label: 'Codec', value: 'prores (HQ)' }, { label: 'Frames', value: '30' }],
    settings: [
      { member: 'primaries', value: 'bt601_525', automatic: 'bt601_525', origin: { kind: 'default', rule: 'Untagged video 480 lines high reads as BT.601 525-line' } },
      { member: 'transfer', value: 'displayE', automatic: 'displayE', origin: { kind: 'file', source: 'colr nclc' } },
      { member: 'matrix', value: 'bt601', automatic: 'bt601', origin: { kind: 'file', source: 'colr nclc' } },
      { member: 'range', value: 'limited', automatic: 'limited', origin: { kind: 'default', rule: 'Untagged Y′CbCr video is limited range' } },
      { member: 'siting', value: 'left', automatic: 'left', origin: { kind: 'default', rule: 'Untagged chroma is sited left (type 0)' } },
      { member: 'frameRate', value: '30000/1001', automatic: '30000/1001', origin: { kind: 'file', source: 'timestamps' } },
      { member: 'fieldOrder', value: 'progressive', automatic: 'progressive', origin: { kind: 'file', source: 'fiel' } },
      square
        ? { member: 'pixelAspect', value: '1:1', automatic: '8:9', origin: { kind: 'override' } }
        : { member: 'pixelAspect', value: '8:9', automatic: '8:9', origin: { kind: 'file', source: 'pasp' } }
    ],
    issues: [],
    displaySize: square ? [720, 480] : [640, 480],
    duration: 1.001
  };
}

let component: ReturnType<typeof mount> | null = null;
afterEach(() => {
  if (component) void unmount(component);
  component = null;
  document.body.replaceChildren();
});

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) { await Promise.resolve(); await tick(); }
  flushSync();
}

describe('InterpretSheet', () => {
  it('shows each setting with its origin, re-reads with the draft, and applies it', async () => {
    const probe = vi.fn(async (interpret: Record<string, unknown> | null): Promise<EngineProbeResult> => ({ status: 'facts', facts: factsFor(interpret) }));
    const onapply = vi.fn();
    const onclose = vi.fn();
    const target = document.createElement('div');
    document.body.append(target);
    component = mount(InterpretSheet, { target, props: { name: 'A001_C003.mov', initial: null, probe, onapply, onclose } });
    await settle();

    expect(probe).toHaveBeenCalledWith(null);
    expect(target.querySelector('h2')!.textContent).toBe('Interpret Footage');
    expect(target.textContent).toContain('A001_C003.mov');
    const titles = Array.from(target.querySelectorAll('.sg-section-title')).map((t) => t.textContent);
    expect(titles).toEqual(['Color', 'Time', 'Geometry', 'File']);
    expect(target.textContent).toContain('Untagged video 480 lines high reads as BT.601 525-line');
    expect(target.textContent).toContain('From the file (pasp)');
    expect(target.textContent).toContain('Shown at640 × 480');
    // Nothing changed yet: Apply waits.
    const apply = Array.from(target.querySelectorAll('button')).find((b) => b.textContent === 'Apply')!;
    expect(apply.disabled).toBe(true);

    const pasp = target.querySelector<HTMLSelectElement>('select[aria-label="Pixel aspect"]')!;
    expect(pasp.options[0]!.textContent).toBe('Automatic (8:9 (0.89))');
    pasp.value = '1:1';
    pasp.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(probe).toHaveBeenLastCalledWith({ pixelAspect: [1, 1] });
    expect(target.textContent).toContain('Overridden · the file reads 8:9 (0.89)');
    expect(target.textContent).toContain('Shown at720 × 480');
    expect(apply.disabled).toBe(false);
    apply.click();
    expect(onapply).toHaveBeenCalledTimes(1);
    expect(onapply.mock.calls[0]![0]).toEqual({ pixelAspect: '1:1' });
    expect(onapply.mock.calls[0]![1].displaySize).toEqual([720, 480]);
  });

  it('says why when the engine cannot read the file', async () => {
    const probe = vi.fn(async (): Promise<EngineProbeResult> => ({ status: 'unavailable', reason: 'Interpret Footage needs the new engine (Settings › General › Use New Engine).' }));
    const target = document.createElement('div');
    document.body.append(target);
    component = mount(InterpretSheet, { target, props: { name: 'clip.mov', initial: { range: 'full' }, probe, onapply: vi.fn(), onclose: vi.fn() } });
    await settle();
    expect(probe).toHaveBeenCalledWith({ range: 'full' });
    expect(target.textContent).toContain('needs the new engine');
    expect(Array.from(target.querySelectorAll('button')).find((b) => b.textContent === 'Apply')!.disabled).toBe(true);
  });
});
