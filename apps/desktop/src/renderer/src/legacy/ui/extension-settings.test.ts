// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { ExtensionRecord, ExtensionsBridge, ExtensionsChangedEvent } from '../../../../shared/extensions';
import { createExtensionSettingsControl, extensionSettingsLabels } from './extension-settings';
import { Registry } from '../../kernel/registry';

function record(id: string, overrides: Partial<ExtensionRecord> = {}): ExtensionRecord {
  return {
    id,
    scope: 'user',
    manifest: { id, name: id, version: '1.0.0', apiVersion: 1, author: 'user' },
    dir: `/extensions/${id}`,
    enabled: true,
    bundleUrl: `app://powermove/ext/${id}/bundle.js`,
    bundleHash: 'hash',
    health: { state: 'ok' },
    updatedAt: 1,
    ...overrides
  };
}

function bridge(initial: ExtensionRecord[]) {
  let records = initial;
  let listener: ((event: ExtensionsChangedEvent) => void) | undefined;
  const stop = vi.fn();
  const api = {
    list: vi.fn(async () => records),
    setEnabled: vi.fn(async ({ id, enabled }) => {
      records = records.map((item) => item.id === id
        ? { ...item, enabled, health: enabled ? { state: 'ok' as const } : { state: 'disabled' as const } }
        : item);
      return records;
    }),
    remove: vi.fn(async () => records),
    reload: vi.fn(async () => records),
    create: vi.fn(async () => records),
    reveal: vi.fn(async () => undefined),
    readSource: vi.fn(async () => []),
    reportHealth: vi.fn(),
    onChanged: vi.fn((next) => { listener = next; return stop; })
  } satisfies ExtensionsBridge;
  return { api, stop, emit: (event: ExtensionsChangedEvent) => listener?.(event), setRecords: (next: ExtensionRecord[]) => { records = next; } };
}

describe('extension settings control', () => {
  it('shows added extensions grouped by type and hides built-ins', async () => {
    const harness = bridge([
      record('builtin-ext', { scope: 'builtin', manifest: { id: 'builtin-ext', name: 'Builtin', version: '1.0.0', apiVersion: 1, author: 'powermove' } }),
      record('agent-ext', { manifest: { id: 'agent-ext', name: 'Agent tool', version: '2.0.0', apiVersion: 1, author: 'agent' } }),
      record('disabled-ext', { enabled: false, health: { state: 'disabled' } }),
      record('old-timeline', { health: { state: 'replaced', by: 'new-timeline' } }),
      record('broken-ext', { manifest: null, bundleUrl: null, bundleHash: null, health: { state: 'manifest-error', error: 'Invalid manifest' } })
    ]);
    const control = createExtensionSettingsControl(harness.api);

    await vi.waitFor(() => expect(control.element.querySelectorAll('[data-extension-id]')).toHaveLength(4));
    expect(control.element.textContent).toContain('4 installed');
    expect(control.element.querySelector('[data-extension-id="agent-ext"] .settings-extension-tag')).toBeNull();
    expect(control.element.querySelector('.settings-extension-group')?.textContent).toContain('Other extensions');
    expect(control.element.querySelector('[data-extension-id="builtin-ext"]')).toBeNull();
    expect(control.element.querySelector('[data-extension-id="disabled-ext"] .toggle')?.getAttribute('aria-pressed')).toBe('false');
    expect(control.element.querySelector('[data-extension-id="broken-ext"] .settings-extension-status')?.textContent).toBe('Needs attention');
    expect(control.element.querySelector('[data-extension-id="old-timeline"]')?.textContent).toContain('Replaced by new-timeline');
    expect(control.element.querySelector('[data-extension-id="broken-ext"]')?.textContent).toContain('Invalid manifest');

    control.destroy();
    expect(harness.stop).toHaveBeenCalledOnce();
  });

  it('updates while Settings is open and toggles through the native bridge', async () => {
    const harness = bridge([record('one')]);
    const control = createExtensionSettingsControl(harness.api);
    await vi.waitFor(() => expect(control.element.querySelectorAll('[data-extension-id]')).toHaveLength(1));

    harness.setRecords([record('one'), record('new-agent', {
      manifest: { id: 'new-agent', name: 'New agent extension', version: '1.0.0', apiVersion: 1, author: 'agent' }
    })]);
    harness.emit({ ids: ['new-agent'], reason: 'create' });
    await vi.waitFor(() => expect(control.element.querySelectorAll('[data-extension-id]')).toHaveLength(2));

    const toggle = control.element.querySelector<HTMLButtonElement>('[data-extension-id="new-agent"] .toggle');
    if (!toggle) throw new Error('New agent extension toggle was not rendered.');
    expect(toggle.classList.contains('on')).toBe(true);
    toggle.click();
    await vi.waitFor(() => expect(harness.api.setEnabled).toHaveBeenCalledWith({ id: 'new-agent', enabled: false }));
    await vi.waitFor(() => expect(control.element
      .querySelector('[data-extension-id="new-agent"] .toggle')?.getAttribute('aria-pressed')).toBe('false'));
    control.destroy();
  });

  it('explains replacements and dependencies and requires confirmation before deletion', async () => {
    const base = record('viewer', { scope: 'builtin', manifest: { id: 'viewer', name: 'Composition viewer', version: '1.0.0', apiVersion: 1 } });
    const custom = record('custom-viewer', { manifest: { id: 'custom-viewer', name: 'Custom viewer', version: '1.0.0', apiVersion: 1, replaces: ['viewer'], contributes: ['panels', 'keybindings'] } });
    const harness = bridge([base, custom]);
    harness.api.remove.mockResolvedValue([base]);
    const control = createExtensionSettingsControl(harness.api);
    await vi.waitFor(() => expect(control.element.querySelector('[data-extension-id="custom-viewer"] .settings-extension-open')).not.toBeNull());
    expect(control.element.querySelector('[data-extension-id="viewer"]')).toBeNull();
    control.element.querySelector<HTMLButtonElement>('[data-extension-id="custom-viewer"] .settings-extension-open')!.click();
    expect(control.element.textContent).toContain('Replaces');
    expect(control.element.textContent).toContain('Composition viewer');
    expect(control.element.textContent).toContain('Panels · Keyboard shortcuts');
    const remove = () => control.element.querySelector<HTMLButtonElement>('[aria-label="Delete Custom viewer"]')!;
    remove().click();
    expect(harness.api.remove).not.toHaveBeenCalled();
    expect(control.element.textContent).toContain('permanently removed');
    const buttons = () => Array.from(control.element.querySelectorAll<HTMLButtonElement>('.settings-extension-confirm button'));
    buttons().find(button => button.textContent === 'Cancel')!.click();
    expect(control.element.querySelector('.settings-extension-confirm')).toBeNull();
    remove().click();
    buttons().find(button => button.textContent === 'Delete extension')!.click();
    await vi.waitFor(() => expect(harness.api.remove).toHaveBeenCalledExactlyOnceWith({ id: 'custom-viewer' }));
    await vi.waitFor(() => expect(control.element.querySelector('[data-extension-id="custom-viewer"]')).toBeNull());
    control.destroy();
  });

  it('keeps the extension and reports a failed delete', async () => {
    const harness = bridge([record('failed')]);
    harness.api.remove.mockRejectedValue(new Error('Permission denied'));
    const control = createExtensionSettingsControl(harness.api);
    await vi.waitFor(() => expect(control.element.querySelector('[data-extension-id="failed"] .settings-extension-open')).not.toBeNull());
    control.element.querySelector<HTMLButtonElement>('[data-extension-id="failed"] .settings-extension-open')!.click();
    expect(control.element.querySelector('[aria-label="Delete failed"]')).not.toBeNull();
    control.element.querySelector<HTMLButtonElement>('[aria-label="Delete failed"]')!.click();
    control.element.querySelector<HTMLButtonElement>('.settings-extension-confirm .settings-extension-delete')!.click();
    await vi.waitFor(() => expect(control.element.textContent).toContain('Permission denied'));
    expect(control.element.querySelector('[data-extension-id="failed"]')).not.toBeNull();
    expect(control.element.querySelector<HTMLButtonElement>('.settings-extension-confirm .settings-extension-delete')!.disabled).toBe(false);
    control.destroy();
  });

  it('uses concise health and source labels', () => {
    expect(extensionSettingsLabels.sourceLabel(record('x', { scope: 'project' }))).toBe('Project');
    expect(extensionSettingsLabels.displayState(record('x', { health: { state: 'runtime-error', error: 'crashed' } })))
      .toEqual({ label: 'Needs attention', tone: 'warning', detail: 'crashed' });
  });
});

describe('extension values in settings', () => {
  const vars = [{ key: 'API_KEY', label: 'API key', secret: true, required: true }];

  it('offers Set up on a row that needs setup and Variables… on the page of a working one', async () => {
    const openSetup = vi.fn();
    (window as any).PM = { Vars: { openSetup } };
    const waiting = record('colour-match', {
      manifest: { id: 'colour-match', name: 'Colour match', version: '1.0.0', apiVersion: 2, author: 'user', vars },
      bundleUrl: null,
      bundleHash: null,
      health: { state: 'needs-setup', missing: ['API_KEY'] }
    });
    const working = record('weather', {
      manifest: { id: 'weather', name: 'Weather', version: '1.0.0', apiVersion: 2, author: 'user', vars }
    });
    const harness = bridge([waiting, working]);
    const control = createExtensionSettingsControl(harness.api);

    await vi.waitFor(() => expect(control.element.querySelectorAll('[data-extension-id]')).toHaveLength(2));
    const row = control.element.querySelector('[data-extension-id="colour-match"]')!;
    expect(row.textContent).toContain('Needs setup');
    const setUp = row.querySelector<HTMLButtonElement>('.settings-extension-setup')!;
    expect(setUp.textContent).toBe('Set up');
    expect(setUp.classList.contains('pri')).toBe(true);
    setUp.click();
    expect(openSetup).toHaveBeenCalledWith(expect.objectContaining({ id: 'colour-match' }));
    expect(control.element.querySelector('.settings-extension-detail')!.hasAttribute('hidden')).toBe(true);
    expect(control.element.querySelector('[data-extension-id="weather"] .settings-extension-setup')).toBeNull();

    control.element.querySelector<HTMLButtonElement>('[data-extension-id="weather"] .settings-extension-open')!.click();
    const variables = control.element.querySelector<HTMLButtonElement>('.settings-extension-detail .settings-extension-setup')!;
    expect(variables.textContent).toBe('Variables…');
    variables.click();
    expect(openSetup).toHaveBeenLastCalledWith(expect.objectContaining({ id: 'weather' }));
    expect(extensionSettingsLabels.displayState(waiting)).toEqual({ label: 'Needs setup', tone: 'setup' });
    control.destroy();
    delete (window as any).PM;
  });
});

describe('extension collection interactions', () => {
  it('filters 39 records, searches descriptions, expands search matches and preserves collapsed groups', async () => {
    const records = Array.from({ length: 39 }, (_,i) => record(`extension-${i}`, {
      enabled: i < 31,
      health: i < 31 ? { state: 'ok' } : { state: 'disabled' },
      manifest: { id: `extension-${i}`, name: `Extension ${i}`, version: '1', apiVersion: 1,
        description: i === 20 ? 'Optical bloom' : 'A useful extension', contributes: [i < 9 ? 'panels' : 'effects'] }
    }));
    const control = createExtensionSettingsControl(bridge(records).api);
    await vi.waitFor(() => expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(39));
    const group = control.element.querySelector<HTMLButtonElement>('[data-category="effects"]')!;
    expect(group.getAttribute('aria-expanded')).toBe('false');
    group.click(); expect(group.getAttribute('aria-expanded')).toBe('true');
    group.click(); expect(group.getAttribute('aria-expanded')).toBe('false');
    const search = control.element.querySelector<HTMLInputElement>('input')!;
    search.value = 'bloom'; search.dispatchEvent(new Event('input'));
    expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(1);
    expect(control.element.querySelector('.settings-extension-group-body')?.hasAttribute('hidden')).toBe(false);
    search.value = ''; search.dispatchEvent(new Event('input'));
    expect(control.element.querySelector('[data-category="effects"]')?.getAttribute('aria-expanded')).toBe('false');
    control.element.querySelector<HTMLButtonElement>('[data-extension-filter="off"]')!.click();
    expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(8);
    control.element.querySelector<HTMLButtonElement>('[data-extension-filter="all"]')!.click();
    const type = control.element.querySelector<HTMLSelectElement>('select')!;
    type.value = 'panels'; type.dispatchEvent(new Event('change'));
    expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(9);
    search.value = 'nothing'; search.dispatchEvent(new Event('input'));
    expect(control.element.textContent).toContain('No extensions match');
    control.element.querySelector<HTMLButtonElement>('.settings-extension-empty button')!.click();
    expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(39);
    control.destroy();
  });

  it('keeps toggle clicks out of the modal, restores row focus, and contains Escape', async () => {
    const harness = bridge([record('one')]);
    const control = createExtensionSettingsControl(harness.api); document.body.append(control.element);
    await vi.waitFor(() => expect(control.element.querySelector('.toggle')).not.toBeNull());
    control.element.querySelector<HTMLButtonElement>('.toggle')!.click();
    await vi.waitFor(() => expect(harness.api.setEnabled).toHaveBeenCalledOnce());
    const dialog = control.element.querySelector<HTMLDialogElement>('dialog')!;
    expect(dialog.open).toBe(false);
    const open = control.element.querySelector<HTMLButtonElement>('.settings-extension-open')!;
    open.focus(); open.click(); expect(dialog.open).toBe(true);
    const parentEscape = vi.fn(); document.body.addEventListener('keydown', parentEscape);
    dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    expect(dialog.open).toBe(false); expect(document.activeElement).toBe(open); expect(parentEscape).not.toHaveBeenCalled();
    document.body.removeEventListener('keydown', parentEscape); control.destroy(); control.element.remove();
  });

  it('reports toggle and file errors, and does not render after destruction', async () => {
    const harness = bridge([record('one')]);
    harness.api.setEnabled.mockRejectedValue(new Error('Cannot disable'));
    harness.api.reveal.mockRejectedValue(new Error('Cannot reveal'));
    const control = createExtensionSettingsControl(harness.api);
    await vi.waitFor(() => expect(control.element.querySelector('.toggle')).not.toBeNull());
    control.element.querySelector<HTMLButtonElement>('.toggle')!.click();
    await vi.waitFor(() => expect(control.element.textContent).toContain('Cannot disable'));
    expect(control.element.querySelector('.toggle')?.getAttribute('aria-pressed')).toBe('true');
    control.element.querySelector<HTMLButtonElement>('.settings-extension-open')!.click();
    Array.from(control.element.querySelectorAll<HTMLButtonElement>('dialog button')).find(b => b.textContent === 'Show in Finder')!.click();
    await vi.waitFor(() => expect(control.element.textContent).toContain('Cannot reveal'));
    control.destroy();
    harness.emit({ids:['one'],reason:'create'});
    expect(control.element.querySelector('dialog')).toBeNull();
  });

  it('uses the record index for global search and handles the empty installed state', async () => {
    const harness = bridge([record('one')]);
    const control = createExtensionSettingsControl(harness.api);
    await vi.waitFor(() => expect(control.element.querySelector('.settings-extension-row')).not.toBeNull());
    control.element.dispatchEvent(new CustomEvent('settings-search', {detail:'one'}));
    expect(control.element.dataset.settingsMatchCount).toBe('1');
    control.element.dispatchEvent(new CustomEvent('settings-search', {detail:'missing'}));
    expect(control.element.dataset.settingsMatchCount).toBe('0');
    harness.setRecords([]); harness.emit({ids:['one'],reason:'remove'});
    await vi.waitFor(() => expect(control.element.textContent).toContain('No extensions installed.'));
    control.destroy();
  });
});


describe('extension modal updates', () => {
  it('filters actionable health states and recovers a failed modal toggle with focus intact', async () => {
    const harness = bridge([
      record('broken', { health: { state: 'runtime-error', error: 'Extension crashed' } }),
      record('working'),
      record('setup', { health: { state: 'needs-setup', missing: ['TOKEN'] } })
    ]);
    const control = createExtensionSettingsControl(harness.api); document.body.append(control.element);
    await vi.waitFor(() => expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(3));
    control.element.querySelector<HTMLButtonElement>('[data-extension-filter="attention"]')!.click();
    expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(2);
    control.element.querySelector<HTMLButtonElement>('[data-extension-id="broken"] .settings-extension-open')!.click();
    harness.api.setEnabled.mockRejectedValue(new Error('Could not update extension'));
    const toggle = control.element.querySelector<HTMLButtonElement>('dialog .toggle')!;
    toggle.focus(); toggle.click();
    await vi.waitFor(() => expect(control.element.querySelector('dialog [role="status"]')?.textContent).toContain('Could not update extension'));
    const restored = control.element.querySelector<HTMLButtonElement>('dialog .toggle')!;
    expect(restored.disabled).toBe(false); expect(restored.getAttribute('aria-pressed')).toBe('true');
    expect(document.activeElement).toBe(restored);
    control.destroy(); control.element.remove();
  });
});

describe('manifest capability grouping', () => {
  const withKinds = (id: string, kinds: string[], name = id) => record(id, {manifest:{id,name,version:'1.0.0',apiVersion:1,contributes:kinds as any}});
  const groupOf = (control: ReturnType<typeof createExtensionSettingsControl>, id: string) => control.element.querySelector(`[data-extension-id="${id}"]`)?.closest('.settings-extension-category')?.querySelector<HTMLElement>('.settings-extension-group')?.dataset.category;

  it('groups by declared capabilities, not names, and renders mixed extensions only once', async () => {
    const records = [
      withKinds('effect-panel',['panels','effects','commands']),
      withKinds('theme-panel',['panels','themes','commands']),
      withKinds('layer-tool',['commands','inspector','layers','menus','palette']),
      withKinds('panel-command',['commands','panels'],'Appearance Effect Theme'),
      withKinds('media-tool',['media','hooks']),
      withKinds('hook',['hooks']),
      withKinds('transition',['transitions']),
      withKinds('unknown',['future-capability']),
      record('broken',{manifest:null,health:{state:'manifest-error',error:'Invalid manifest'}})
    ];
    const control = createExtensionSettingsControl(bridge(records).api);
    await vi.waitFor(() => expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(records.length));
    expect(groupOf(control,'effect-panel')).toBe('effects');
    expect(groupOf(control,'theme-panel')).toBe('appearance');
    expect(groupOf(control,'layer-tool')).toBe('layers');
    expect(groupOf(control,'panel-command')).toBe('panels');
    expect(groupOf(control,'media-tool')).toBe('media');
    expect(groupOf(control,'hook')).toBe('tools');
    expect(groupOf(control,'transition')).toBe('effects');
    expect(groupOf(control,'unknown')).toBe('other');
    expect(groupOf(control,'broken')).toBe('other');
    expect(control.element.querySelectorAll('.settings-extension-row .toggle')).toHaveLength(records.length);
    const effectsHeading = control.element.querySelector('[data-category="effects"]')!;
    expect(effectsHeading.querySelector('.settings-extension-count')?.textContent).toBe('2');
    expect(effectsHeading.getAttribute('aria-label')).toBe('Effects, 2 extensions');
    expect(effectsHeading.querySelector('.settings-extension-count')?.getAttribute('aria-hidden')).toBe('true');
    expect(effectsHeading.querySelector('.settings-extension-group-icon')?.getAttribute('aria-hidden')).toBe('true');
    expect(control.element.querySelector('[data-extension-id="effect-panel"] .settings-extension-kinds')?.textContent).toBe('Panels');
    expect(control.element.querySelector('[data-extension-id="theme-panel"] .settings-extension-open')?.getAttribute('aria-description')).toContain('Panels, Appearance, Editor tools');
    expect(Array.from(control.element.querySelectorAll('.settings-extension-group')).every(group => group.getAttribute('aria-expanded')==='false')).toBe(true);
    control.destroy();
  });

  it('finds secondary capabilities with type filters without duplicate rows or toggles', async () => {
    const harness = bridge([withKinds('effect-panel',['effects','panels']),withKinds('theme-panel',['themes','panels']),withKinds('panel',['panels']),withKinds('hook',['hooks'])]);
    const control = createExtensionSettingsControl(harness.api);
    await vi.waitFor(() => expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(4));
    const type = control.element.querySelector<HTMLSelectElement>('select')!;
    type.value='panels';type.dispatchEvent(new Event('change'));
    expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(3);
    expect(control.element.querySelectorAll('.settings-extension-row .toggle')).toHaveLength(3);
    expect(control.element.querySelectorAll('.settings-extension-group[data-category]')).toHaveLength(1);
    expect(groupOf(control,'effect-panel')).toBe('panels');
    type.value='all';type.dispatchEvent(new Event('change'));
    expect(groupOf(control,'effect-panel')).toBe('effects');expect(groupOf(control,'theme-panel')).toBe('appearance');
    const search=control.element.querySelector<HTMLInputElement>('input')!;
    search.value='Panels';search.dispatchEvent(new Event('input'));
    expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(3);
    control.destroy();
  });

  it('omits empty groups and unavailable types, updating them when records change', async () => {
    const harness=bridge([withKinds('effect',['effects'])]);const control=createExtensionSettingsControl(harness.api);
    await vi.waitFor(() => expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(1));
    const type=control.element.querySelector<HTMLSelectElement>('select')!;
    expect(Array.from(type.options,option=>option.value)).toEqual(['all','effects']);
    expect(control.element.querySelectorAll('.settings-extension-group[data-category]')).toHaveLength(1);
    type.value='effects';type.dispatchEvent(new Event('change'));
    harness.setRecords([withKinds('panel',['panels'])]);harness.emit({ids:['effect','panel'],reason:'create'});
    await vi.waitFor(() => expect(groupOf(control,'panel')).toBe('panels'));
    expect(type.value).toBe('all');expect(Array.from(type.options,option=>option.value)).toEqual(['all','panels']);
    harness.setRecords([]);harness.emit({ids:['panel'],reason:'remove'});
    await vi.waitFor(() => expect(control.element.querySelectorAll('.settings-extension-group[data-category]')).toHaveLength(0));
    expect(Array.from(type.options,option=>option.value)).toEqual(['all']);
    control.destroy();
  });
});

describe('nested contribution categories', () => {
  const rec = (id: string, kinds: string[] = ['effects']) => record(id,{manifest:{id,name:id,version:'1.0.0',apiVersion:1,contributes:kinds as any}});
  const kernel = () => ({effects:new Registry<any>(),commands:new Registry<any>(),themes:new Registry<any>()});
  const subOf = (control: ReturnType<typeof createExtensionSettingsControl>, id: string) => control.element.querySelector(`[data-extension-id="${id}"]`)?.closest('.settings-extension-subcategory')?.querySelector<HTMLElement>('[data-subcategory]')?.dataset.subcategory;

  it('uses the Effects panel group metadata for nested disclosures and searches across multi-category entries', async () => {
    const previous=(window as any).PM;const k=kernel();(window as any).PM={Kernel:k};
    k.effects.register('alpha',{id:'alpha-fx',group:'Color',get frag(){throw new Error('Do not compile browsing metadata');}});
    k.effects.register('beta',{id:'beta-fx',group:'Blur & Sharpen'});
    k.effects.register('gamma',{id:'gamma-one',group:'Color'});k.effects.register('gamma',{id:'gamma-two',group:'Stylize'});
    const harness=bridge([rec('alpha'),rec('beta'),rec('gamma'),rec('unknown'),rec('transition',['transitions'])]);
    const control=createExtensionSettingsControl(harness.api);
    try{
      await vi.waitFor(()=>expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(5));
      expect(subOf(control,'alpha')).toBe('Color');expect(subOf(control,'beta')).toBe('Blur & Sharpen');
      expect(subOf(control,'gamma')).toBe('Multiple categories');expect(subOf(control,'unknown')).toBe('Other');expect(subOf(control,'transition')).toBe('Transitions');
      expect(control.element.querySelectorAll('[data-extension-id="gamma"] .toggle')).toHaveLength(1);
      const top=control.element.querySelector<HTMLButtonElement>('[data-category="effects"]')!;top.click();
      expect(Array.from(control.element.querySelectorAll('[data-subcategory]')).every(button=>button.getAttribute('aria-expanded')==='false')).toBe(true);
      const color=control.element.querySelector<HTMLButtonElement>('[data-subcategory="Color"]')!;color.click();
      expect(control.element.querySelector('[data-extension-id="alpha"]')?.closest('[hidden]')).toBeNull();
      const search=control.element.querySelector<HTMLInputElement>('input')!;search.value='Color';search.dispatchEvent(new Event('input'));
      expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(2);
      expect(subOf(control,'gamma')).toBe('Multiple categories');
      search.value='';search.dispatchEvent(new Event('input'));
      expect(control.element.querySelector('[data-subcategory="Color"]')?.getAttribute('aria-expanded')).toBe('true');
      expect(control.element.querySelector('[data-subcategory="Blur & Sharpen"]')?.getAttribute('aria-expanded')).toBe('false');
      harness.setRecords([rec('beta')]);harness.emit({ids:['alpha','gamma'],reason:'remove'});
      await vi.waitFor(()=>expect(control.element.querySelectorAll('[data-subcategory]')).toHaveLength(1));
      expect(control.element.querySelector('[data-subcategory="Color"]')).toBeNull();
    }finally{control.destroy();(window as any).PM=previous;}
  });

  it('subdivides Panels by declared command categories and Appearance by theme schemes only when available', async () => {
    const previous=(window as any).PM;const k=kernel();(window as any).PM={Kernel:k};
    k.commands.register('audio',{id:'audio-command',category:'Media'});k.commands.register('animate',{id:'animate-command',category:'Animation'});
    k.themes.register('light',{id:'light-theme',scheme:'light'});k.themes.register('dark',{id:'dark-theme',scheme:'dark'});
    k.themes.register('auto',{id:'auto-theme',scheme:'auto'});k.themes.register('both',{id:'both-light',scheme:'light'});k.themes.register('both',{id:'both-dark',scheme:'dark'});
    const control=createExtensionSettingsControl(bridge([rec('audio',['panels','commands']),rec('animate',['panels','commands']),rec('plain',['panels']),rec('light',['themes']),rec('dark',['themes']),rec('auto',['themes']),rec('both',['themes'])]).api);
    try{
      await vi.waitFor(()=>expect(control.element.querySelectorAll('.settings-extension-row')).toHaveLength(7));
      expect(subOf(control,'audio')).toBe('Media');expect(subOf(control,'animate')).toBe('Animation');expect(subOf(control,'plain')).toBe('Other panels');
      expect(subOf(control,'light')).toBe('Light themes');expect(subOf(control,'dark')).toBe('Dark themes');expect(subOf(control,'auto')).toBe('Adaptive themes');expect(subOf(control,'both')).toBe('Multiple appearances');
    }finally{control.destroy();(window as any).PM=previous;}
    const plain=createExtensionSettingsControl(bridge([rec('plain',['panels']),rec('theme',['themes'])]).api);
    await vi.waitFor(()=>expect(plain.element.querySelectorAll('.settings-extension-row')).toHaveLength(2));
    expect(plain.element.querySelectorAll('[data-subcategory]')).toHaveLength(0);plain.destroy();
  });

  it('retains observed categories when disabled, invalidates new code, and keeps mixed entries single under filters', async () => {
    const previous=(window as any).PM;const k=kernel();(window as any).PM={Kernel:k};
    const effect=k.effects.register('mixed',{id:'mixed-fx',group:'Generate'});const command=k.commands.register('mixed',{id:'mixed-command',category:'Media'});
    const harness=bridge([rec('mixed',['effects','panels','commands'])]);const control=createExtensionSettingsControl(harness.api);
    try{
      await vi.waitFor(()=>expect(subOf(control,'mixed')).toBe('Generate'));
      const type=control.element.querySelector<HTMLSelectElement>('select')!;type.value='panels';type.dispatchEvent(new Event('change'));
      expect(subOf(control,'mixed')).toBe('Media');expect(control.element.querySelectorAll('.settings-extension-row .toggle')).toHaveLength(1);
      type.value='all';type.dispatchEvent(new Event('change'));
      harness.setRecords([{...rec('mixed',['effects','panels','commands']),enabled:false,health:{state:'disabled'}}]);effect.dispose();command.dispose();harness.emit({ids:['mixed'],reason:'disable'});
      await vi.waitFor(()=>expect(control.element.querySelector('.toggle')?.getAttribute('aria-pressed')).toBe('false'));
      expect(subOf(control,'mixed')).toBe('Generate');
      harness.setRecords([{...rec('mixed',['effects','panels','commands']),bundleHash:'new-code'}]);harness.emit({ids:['mixed'],reason:'reload'});
      await vi.waitFor(()=>expect(subOf(control,'mixed')).toBe('Other'));
      k.effects.register('mixed',{id:'new-fx',group:'Text'});
      await vi.waitFor(()=>expect(subOf(control,'mixed')).toBe('Text'));
    }finally{control.destroy();(window as any).PM=previous;}
  });
});
