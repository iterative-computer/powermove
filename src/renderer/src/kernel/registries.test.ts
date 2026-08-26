import { describe, expect, it, vi } from 'vitest';
import type { EffectDefinition, MenuContribution, PaletteEntry } from './api';
import { createKernel, runKernelCommand } from './registries';

const effect = (over: Partial<EffectDefinition> = {}): EffectDefinition => ({
  id: 'wave',
  label: 'Wave',
  group: 'Distort',
  params: [{ k: 'amount', label: 'Amount', def: 1, min: 0, max: 10 }],
  frag: 'o = src(v_st) * u_amount;',
  ...over
});

describe('kernel keybindings', () => {
  it('normalises chords, sorts by priority then registration, and lists bindings', () => {
    const kernel = createKernel();
    kernel.bind('built-in', { key: 'Meta+K', command: 'palette', priority: 100 });
    kernel.bind('ext-b', { key: 'cmd+k', command: 'other', priority: 0 });
    kernel.bind('ext-a', { key: 'cmd+k', command: 'first', priority: 0 });

    expect(kernel.bindingsFor('CMD+K').map((b) => b.command)).toEqual(['other', 'first', 'palette']);
    expect(kernel.bindingsFor('cmd+j')).toEqual([]);
    expect(kernel.listBindings()).toHaveLength(3);
    expect(kernel.bindingsFor('cmd+k')[0]?.inFields).toBe(false);
  });

  it('unbinds this owner by default and everyone with all=true', () => {
    const kernel = createKernel();
    kernel.bind('built-in', { key: 'space', command: 'play' });
    kernel.bind('ext', { key: 'space', command: 'scrub' });

    kernel.unbind('ext', 'space');
    expect(kernel.bindingsFor('space').map((b) => b.command)).toEqual(['play']);

    kernel.bind('ext', { key: 'space', command: 'scrub' });
    kernel.unbind('ext', ' ', true);
    expect(kernel.bindingsFor('space')).toEqual([]);
  });

  it('rejects invalid bindings', () => {
    const kernel = createKernel();
    expect(() => kernel.bind('ext', { key: '', command: 'x' })).toThrow(/invalid key/);
    expect(() => kernel.bind('ext', { key: 'cmd+k', command: '' })).toThrow(/command/);
  });
});

describe('kernel effects & transitions', () => {
  it('validates ids, passes, params, and frag size', () => {
    const kernel = createKernel();
    expect(() => kernel.registerEffect('ext', effect({ id: '9bad' }))).toThrow(/invalid id/);
    expect(() => kernel.registerEffect('ext', effect({ passes: 0 }))).toThrow(/passes/);
    expect(() => kernel.registerEffect('ext', effect({ passes: 9 }))).toThrow(/passes/);
    expect(() => kernel.registerEffect('ext', effect({ params: [{ k: 'Bad', label: 'x', def: 0, min: 0, max: 1 }] }))).toThrow(/invalid param key/);
    expect(() => kernel.registerEffect('ext', effect({ frag: 'x'.repeat(64 * 1024 + 1) }))).toThrow(/too large/);
    expect(() =>
      kernel.registerEffect('ext', effect({ params: Array.from({ length: 33 }, (_, i) => ({ k: `p${i}`, label: 'x', def: 0, min: 0, max: 1 })) }))
    ).toThrow(/too many params/);

    const handle = kernel.registerEffect('ext', effect());
    expect(kernel.effects.get('wave')?.label).toBe('Wave');
    handle.dispose();
    expect(kernel.effects.list()).toEqual([]);
  });

  it('registers transitions with the same validation', () => {
    const kernel = createKernel();
    expect(() => kernel.registerTransition('ext', { id: 'wipe', label: '', params: [], frag: 'o=vec4(1.);' })).toThrow(/label/);
    kernel.registerTransition('ext', { id: 'wipe', label: 'Wipe', params: [], frag: 'o=vec4(1.);' });
    expect(kernel.transitions.get('wipe')?.label).toBe('Wipe');
  });
});

describe('kernel palette, menus, events, theme', () => {
  it('collects palette providers and menu contributions in registration order', () => {
    const kernel = createKernel();
    const entry = (id: string): PaletteEntry => ({ id, label: id, category: 'Test', run: () => id });
    kernel.registerPaletteProvider('a', () => [entry('a1')]);
    const second = kernel.registerPaletteProvider('b', () => [entry('b1')]);
    expect(kernel.paletteProviders().flatMap((p) => p.provider('').map((e) => e.id))).toEqual(['a1', 'b1']);
    second.dispose();
    expect(kernel.paletteProviders()).toHaveLength(1);

    kernel.contributeMenu('a', 'titlebar:right', () => [{ label: 'A' }]);
    kernel.contributeMenu('b', 'titlebar:right', (ctx) => [{ label: String(ctx.label ?? 'B') }]);
    expect(kernel.collectMenu('titlebar:right', { label: 'ctx' })).toEqual<MenuContribution[]>([{ label: 'A' }, { label: 'ctx' }]);
    expect(kernel.collectMenu('layer:context')).toEqual([]);
  });

  it('skips a throwing menu contribution instead of losing the whole menu', () => {
    const kernel = createKernel();
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    kernel.contributeMenu('bad', 'panel:context', () => {
      throw new Error('boom');
    });
    kernel.contributeMenu('good', 'panel:context', () => [{ label: 'still here' }]);
    expect(kernel.collectMenu('panel:context')).toEqual([{ label: 'still here' }]);
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });

  it('emits typed events, isolates handler errors, and scopes by owner', () => {
    const kernel = createKernel();
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    const seen: number[] = [];
    kernel.events.on('time', () => {
      throw new Error('boom');
    }, 'bad');
    kernel.events.on('time', (t) => void seen.push(t), 'good');
    kernel.events.emit('time', 3);
    expect(seen).toEqual([3]);

    kernel.events.disposeOwner('good');
    kernel.events.emit('time', 4);
    expect(seen).toEqual([3]);
    error.mockRestore();
  });

  it('tracks the active theme and scheme preference and announces changes', () => {
    const kernel = createKernel();
    const changes: Array<{ id: string; scheme: string }> = [];
    kernel.events.on('theme:changed', (payload) => void changes.push(payload));
    kernel.themes.register('ext', { id: 'win98', name: 'Win98', scheme: 'light' });

    kernel.activateTheme('win98');
    expect(kernel.theme.activeId).toBe('win98');
    expect(changes.at(-1)).toEqual({ id: 'win98', scheme: 'light' });

    kernel.setScheme('dark');
    expect(kernel.theme.scheme).toBe('dark');
    expect(changes.at(-1)).toEqual({ id: 'win98', scheme: 'dark' });
  });
});

describe('runKernelCommand', () => {
  it('runs the top command and reports false for unknown ids', () => {
    const kernel = createKernel();
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    kernel.commands.register('base', { id: 'undo', label: 'Undo', run: () => 'base' });
    kernel.commands.register('ext', { id: 'undo', label: 'Undo', run: (...args) => `ext:${String(args[0])}` });

    expect(runKernelCommand(kernel, 'undo', [7])).toBe('ext:7');
    expect(runKernelCommand(kernel, 'missing')).toBe(false);
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it('disposeOwner clears every registry, palette provider, menu, and event handler', () => {
    const kernel = createKernel();
    kernel.commands.register('ext', { id: 'c', label: 'C', run: () => 1 });
    kernel.panels.register('ext', { id: 'p', title: 'P', build: () => {} });
    kernel.bind('ext', { key: 'cmd+1', command: 'c' });
    kernel.registerEffect('ext', effect());
    kernel.status.register('ext', { id: 's', text: () => 'hi' });
    kernel.registerPaletteProvider('ext', () => []);
    kernel.contributeMenu('ext', 'viewer:context', () => [{ label: 'x' }]);
    kernel.events.on('layout', () => {}, 'ext');

    kernel.disposeOwner('ext');

    expect(kernel.commands.list()).toEqual([]);
    expect(kernel.panels.list()).toEqual([]);
    expect(kernel.listBindings()).toEqual([]);
    expect(kernel.effects.list()).toEqual([]);
    expect(kernel.status.list()).toEqual([]);
    expect(kernel.paletteProviders()).toEqual([]);
    expect(kernel.collectMenu('viewer:context')).toEqual([]);
  });
});
