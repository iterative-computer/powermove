// @vitest-environment happy-dom
// @ts-nocheck -- legacy PM is intentionally a dynamic registry.
/*
 * `PM.commands`, `PM.PANELS` and `PM.FX` are now Proxies over the kernel
 * registries. These tests pin the object spellings the rest of the legacy tree
 * uses on them — indexing, `Object.keys/values`, `for..in`, `in`, assignment
 * and delete — plus the two behaviours the views add: an override reaches the
 * mounted panel, and a replaced effect drops its compiled program.
 */
import { describe, expect, it, vi } from 'vitest';

import { EFFECTS } from '../../../../extensions/effects-basic/effects';
import { install as installLayout } from '../ui/layout';
import { install as installShaders } from '../gl/shaders';
import { install as installShortcuts } from '../ui/shortcuts';

function commandsPM() {
  const PM: any = {
    h: () => ({}),
    proj: { w: 1920, h: 1080, fps: 30, dur: 8, layers: [], assets: {} },
    time: 0,
    snapF: (value: number) => value
  };
  installShortcuts(PM);
  return PM;
}

describe('PM.commands view over kernel.commands', () => {
  it('reads a command in the legacy {id,label,kb,run,cat} shape', () => {
    const PM = commandsPM();
    expect(PM.commands.undo).toMatchObject({ id: 'undo', label: 'Undo', kb: '⌘Z', cat: 'Edit' });
    expect(PM.commands.undo.run).toBeTypeOf('function');
    expect(PM.commands.nothingHere).toBeUndefined();
  });

  it('supports keys, values, for..in and the in operator', () => {
    const PM = commandsPM();
    const keys = Object.keys(PM.commands);
    expect(keys).toContain('undo');
    expect(keys).toContain('palette');
    expect(Object.values(PM.commands).map((command: any) => command.id)).toEqual(keys);

    const iterated: string[] = [];
    for (const key in PM.commands) iterated.push(key);
    expect(iterated).toEqual(keys);

    expect('undo' in PM.commands).toBe(true);
    expect('nothingHere' in PM.commands).toBe(false);
  });

  it('supports plain-object entries, spread, JSON, descriptors and direct hasOwnProperty', () => {
    const PM = commandsPM();
    const keys = Object.keys(PM.commands);
    const entries = Object.entries(PM.commands);
    const spread = { ...PM.commands };
    const json = JSON.parse(JSON.stringify(PM.commands));

    expect(entries.map(([id]) => id)).toEqual(keys);
    expect(entries.find(([id]) => id === 'undo')?.[1]).toMatchObject({ id: 'undo', label: 'Undo' });
    expect(spread.undo).toMatchObject({ id: 'undo', label: 'Undo' });
    expect(json.undo).toMatchObject({ id: 'undo', label: 'Undo' });
    expect(Object.getOwnPropertyDescriptor(PM.commands, 'undo')).toMatchObject({
      enumerable: true,
      configurable: true,
      writable: true
    });
    expect(PM.commands.hasOwnProperty('undo')).toBe(true);
    expect(PM.commands.hasOwnProperty('nothingHere')).toBe(false);
    expect(Object.prototype.hasOwnProperty.call(PM.commands, 'undo')).toBe(true);
  });

  it('re-registers the command when a field is assigned (overlays/install.ts)', () => {
    const PM = commandsPM();
    const open = vi.fn();

    PM.commands.palette.run = open;

    expect(PM.commands.palette.run).toBe(open);
    expect(PM.Kernel.commands.get('palette').run).toBe(open);
    PM.cmd('palette');
    expect(open).toHaveBeenCalledOnce();
    /* Label and category survive the partial update. */
    expect(PM.commands.palette.label).toBe('Command palette');
    expect(PM.commands.palette.cat).toBe('View');
  });

  it('accepts a whole command assigned onto the view and removes it on delete', () => {
    const PM = commandsPM();
    const run = vi.fn(() => 'done');

    PM.commands.custom = { id: 'custom', label: 'Custom', kb: null, cat: 'Test', run };

    expect(PM.Kernel.commands.get('custom').label).toBe('Custom');
    expect(PM.cmd('custom')).toBe('done');
    expect(Object.keys(PM.commands)).toContain('custom');

    delete PM.commands.custom;
    expect('custom' in PM.commands).toBe(false);
  });

  it('surfaces and deletes commands registered by other owners', () => {
    const PM = commandsPM();
    PM.Kernel.commands.register('ext:demo', { id: 'demo.go', label: 'Go', run: () => 7 });

    expect(PM.commands['demo.go'].label).toBe('Go');
    expect(PM.cmd('demo.go')).toBe(7);
    expect(delete PM.commands['demo.go']).toBe(true);
    expect(PM.Kernel.commands.has('demo.go')).toBe(false);
  });

  it('deleting an extension override restores the legacy-owned command', () => {
    const PM = commandsPM();
    PM.Kernel.commands.register('ext:undo', { id: 'undo', label: 'Extension undo', run: () => 7 });

    expect(PM.commands.undo.label).toBe('Extension undo');
    expect(delete PM.commands.undo).toBe(true);
    expect(PM.commands.undo.label).toBe('Undo');
    expect(PM.Kernel.commands.topEntry('undo').ownerId).toBe('legacy');
  });

  it('keeps the keymap-only commands out of the offered set via when()', () => {
    const PM = commandsPM();
    for (const id of ['transportPause', 'transportPlay', 'trimIn', 'trimOut', 'blurField']) {
      expect(PM.commands[id].when()).toBe(false);
    }
    expect(PM.commands.undo.when).toBeUndefined();
  });

  it('normalises a false return so a bound key still prevents the default', () => {
    const PM = commandsPM();
    /* `false` is the kernel key listener's "unhandled, try the next binding"
       sentinel. The legacy handler always prevented the default, so `def()`
       must never let one through. */
    PM.hist = { undo: () => false };
    expect(PM.cmd('undo')).toBeUndefined();
  });
});

function layoutPM() {
  const PM: any = { panelInst: {}, bus: { on: () => () => {}, emit() {} } };
  installLayout(PM);
  return PM;
}

describe('PM.PANELS view over kernel.panels', () => {
  it('registers through PM.registerPanel and defaults id and title', () => {
    const PM = layoutPM();
    PM.registerPanel('notes', { size: 180 });

    expect(PM.PANELS.notes).toMatchObject({ id: 'notes', title: 'notes', size: 180 });
    expect(PM.Kernel.panels.get('notes').size).toBe(180);
  });

  it('supports keys, values, in and delete', () => {
    const PM = layoutPM();
    PM.registerPanel('a', { title: 'A' });
    PM.registerPanel('b', { title: 'B' });

    expect(Object.keys(PM.PANELS)).toEqual(['a', 'b']);
    expect(Object.values(PM.PANELS).map((panel: any) => panel.title)).toEqual(['A', 'B']);
    expect('a' in PM.PANELS).toBe(true);

    delete PM.PANELS.a;
    expect(Object.keys(PM.PANELS)).toEqual(['b']);
  });

  it('accepts Object.assign onto the view', () => {
    const PM = layoutPM();
    Object.assign(PM.PANELS, { alpha: { title: 'Alpha' } });

    expect(PM.PANELS.alpha.title).toBe('Alpha');
    expect(PM.Kernel.panels.has('alpha')).toBe(true);
  });

  it('rebuilds a mounted panel when its definition is overridden', () => {
    const PM = layoutPM();
    const refresh = vi.fn();
    PM.Layout.refresh = refresh;
    PM.registerPanel('notes', { title: 'Notes' });
    const element = document.createElement('div');
    document.body.append(element);
    PM.panelInst.notes = { el: element, def: PM.PANELS.notes };

    PM.Kernel.panels.register('ext:notes', { id: 'notes', title: 'Better notes', build: () => {} });

    expect(refresh).toHaveBeenCalledWith('notes');
    expect(PM.panelInst.notes.def.title).toBe('Better notes');
    expect(PM.PANELS.notes.title).toBe('Better notes');
  });

  it('hides a mounted panel from the workspace when its last definition goes away', () => {
    const PM = layoutPM();
    const hidePanel = vi.fn();
    PM.Layout.refresh = vi.fn();
    PM.Layout.hidePanel = hidePanel;
    PM.WS = { mutate: (fn: any) => fn({ id: 'ws' }) };
    PM.registerPanel('notes', { title: 'Notes' });
    PM.panelInst.notes = { el: document.createElement('div'), def: PM.PANELS.notes };

    delete PM.PANELS.notes;

    expect(hidePanel).toHaveBeenCalledWith({ id: 'ws' }, 'notes');
    expect(PM.PANELS.notes).toBeUndefined();
  });

  it('restores the previous definition when an override is disposed', () => {
    const PM = layoutPM();
    PM.Layout.refresh = vi.fn();
    PM.registerPanel('notes', { title: 'Notes' });
    const override = PM.Kernel.panels.register('ext:notes', { id: 'notes', title: 'Better notes' });

    expect(PM.PANELS.notes.title).toBe('Better notes');
    override.dispose();
    expect(PM.PANELS.notes.title).toBe('Notes');
  });

  it('deletes an extension-owned panel through the facade', () => {
    const PM = layoutPM();
    PM.Kernel.panels.register('ext:notes', { id: 'notes', title: 'Extension notes' });

    expect(delete PM.PANELS.notes).toBe(true);
    expect(PM.Kernel.panels.has('notes')).toBe(false);
    expect(PM.PANELS.notes).toBeUndefined();
  });
});

function shaderPM() {
  let nextId = 0;
  const PM: any = {
    uid: (prefix: string) => `${prefix}${++nextId}`,
    P: (value: any) => ({ v: value, kf: [], expr: '' }),
    GL: { dropProgram: vi.fn() }
  };
  installShaders(PM);
  for (const definition of EFFECTS) PM.Kernel.registerEffect('effects-basic', definition);
  return PM;
}

describe('PM.FX view over kernel.effects', () => {
  it('exposes every built-in in the legacy shape', () => {
    const PM = shaderPM();

    expect(Object.keys(PM.FX)).toContain('blur');
    expect('blur' in PM.FX).toBe(true);
    expect(PM.FX.blur).toMatchObject({ label: 'Gaussian Blur', group: 'Blur & Sharpen', passes: 2 });
    expect(PM.FX.blur.frag).toContain('void main');
    expect(Object.values(PM.FX).every((definition: any) => typeof definition.label === 'string')).toBe(true);
    expect(PM.FX.nothingHere).toBeUndefined();
  });

  it('registers the built-ins into the kernel as raw shaders owned by their extension', () => {
    const PM = shaderPM();
    const entry = PM.Kernel.effects.entries().find((item: any) => item.id === 'blur');

    expect(entry.ownerId).toBe('effects-basic');
    expect(entry.item.rawShader).toBe(true);
  });

  it('shows a kernel-registered effect with generated named uniforms', () => {
    const PM = shaderPM();
    PM.Kernel.registerEffect('ext:vhs', {
      id: 'vhs', label: 'VHS', group: 'Stylize',
      params: [{ k: 'wobble', label: 'Wobble', def: 3, min: 0, max: 20 }],
      frag: 'o = texture(u_tex, v_st + vec2(u_wobble, 0.0));'
    });

    expect(PM.FX.vhs.label).toBe('VHS');
    expect(PM.FX.vhs.frag).toContain('uniform float u_wobble;');
    expect(PM.FX.vhs.frag).toContain('#define u_p0 u_wobble');
    expect(PM.FX.vhs.passes).toBe(1);
    expect(Object.keys(PM.mkEffect('vhs').p)).toEqual(['wobble']);
  });

  it('drops the compiled program when an effect is replaced or removed', () => {
    const PM = shaderPM();
    const override = PM.Kernel.registerEffect('ext:blur', {
      id: 'blur', label: 'Better blur', group: 'Blur & Sharpen',
      params: [{ k: 'amount', label: 'Amount', def: 1, min: 0, max: 2 }],
      frag: 'o = texture(u_tex, v_st);'
    });

    expect(PM.GL.dropProgram).toHaveBeenCalledWith('fx:blur');
    expect(PM.FX.blur.label).toBe('Better blur');

    PM.GL.dropProgram.mockClear();
    override.dispose();
    expect(PM.GL.dropProgram).toHaveBeenCalledWith('fx:blur');
    expect(PM.FX.blur.label).toBe('Gaussian Blur');
  });

  it('keeps mkEffect returning null for an unknown type', () => {
    const PM = shaderPM();
    expect(PM.mkEffect('nothingHere')).toBeNull();
  });

  it('deletes an extension-owned effect through the facade', () => {
    const PM = shaderPM();

    expect(delete PM.FX.blur).toBe(true);
    expect(PM.Kernel.effects.has('blur')).toBe(false);
    expect(PM.FX.blur).toBeUndefined();
  });
});
