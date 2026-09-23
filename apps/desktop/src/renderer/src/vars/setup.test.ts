// @vitest-environment happy-dom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ExtensionRecord } from '../../../shared/extensions';
import type { VarsBridge, VarsStatus } from '../../../shared/vars-ipc';
import SetupSheet from './SetupSheet.svelte';
import { usefulHint } from './hint';

const record: ExtensionRecord = {
  id: 'colour-match',
  scope: 'user',
  manifest: {
    id: 'colour-match', name: 'Colour match', version: '1.0.0', apiVersion: 2,
    vars: [
      { key: 'API_KEY', label: 'OpenAI API key', secret: true, required: true, hint: 'OpenAI API key' },
      { key: 'REGION', label: 'Region', hint: 'Where requests are sent' }
    ]
  },
  dir: '/ext/colour-match', enabled: true, bundleUrl: null, bundleHash: null,
  health: { state: 'needs-setup', missing: ['API_KEY'] }, updatedAt: 0
};

function statusOf(set: Record<string, boolean>): VarsStatus {
  return {
    keys: [
      { key: 'API_KEY', label: 'OpenAI API key', hint: 'OpenAI API key', secret: true, required: true, set: !!set.API_KEY, undecryptable: false },
      { key: 'REGION', label: 'Region', hint: 'Where requests are sent', secret: false, required: false, set: !!set.REGION, undecryptable: false }
    ],
    status: set.API_KEY ? 'ok' : 'needs-setup'
  };
}

let component: ReturnType<typeof mount> | null = null;
afterEach(() => {
  if (component) void unmount(component);
  component = null;
  document.body.replaceChildren();
});

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) { await Promise.resolve(); await tick(); }
  flushSync();
}

describe('SetupSheet', () => {
  it('saves typed values without ever asking for stored ones', async () => {
    const set: Record<string, boolean> = { REGION: true };
    const bridge: VarsBridge = {
      status: vi.fn(async () => statusOf(set)),
      set: vi.fn(async ({ key }) => { set[key] = true; return statusOf(set); }),
      delete: vi.fn(async ({ key }) => { set[key] = false; return statusOf(set); }),
      reveal: vi.fn(async () => undefined),
      values: vi.fn(async () => { throw new Error('the sheet must not read values'); })
    };
    const onclose = vi.fn();
    const toast = vi.fn();
    const target = document.createElement('div');
    document.body.append(target);
    component = mount(SetupSheet, { target, props: { PM: { ICONS: { eye: '', eyeoff: '' }, toast }, record, bridge, onclose } });
    await settle();

    expect(target.querySelector('h2')!.textContent).toBe('Colour match');
    expect(target.textContent).toContain('Turns on once required values are set.');
    expect(target.textContent).toContain('Required');
    // A hint that repeats the label is dropped; a useful one stays.
    expect(target.textContent).not.toMatch(/OpenAI API key\s*OpenAI API key/);
    expect(target.textContent).toContain('Where requests are sent');

    const input = target.querySelector<HTMLInputElement>('input[type="password"]')!;
    expect(input).toBeTruthy();
    target.querySelector<HTMLButtonElement>('.vars-eye')!.click();
    flushSync();
    expect(target.querySelector<HTMLInputElement>('.vars-field input')!.type).toBe('text');

    const stored = target.querySelector('.vars-stored')!;
    expect(stored.textContent).toContain('••••');
    (Array.from(stored.querySelectorAll('button')).find((b) => b.textContent === 'Reveal') as HTMLButtonElement).click();
    await settle();
    expect(bridge.reveal).toHaveBeenCalledWith({ id: 'colour-match', key: 'REGION' });

    const field = target.querySelector<HTMLInputElement>('.vars-field input')!;
    field.value = 'sk-typed';
    field.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    target.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(bridge.set).toHaveBeenCalledWith({ id: 'colour-match', key: 'API_KEY', value: 'sk-typed' });
    expect(bridge.values).not.toHaveBeenCalled();
    expect(onclose).toHaveBeenCalled();
    expect(toast).toHaveBeenCalled();
  });

  it('drops hints that only repeat the label', () => {
    expect(usefulHint('API key', 'api key')).toBeNull();
    expect(usefulHint('OpenAI API key', 'API key')).toBeNull();
    expect(usefulHint('Region', 'Where requests are sent')).toBe('Where requests are sent');
    expect(usefulHint('Region', undefined)).toBeNull();
  });
});
