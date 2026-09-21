/*
 * The Kernel: one object holding every contribution registry plus the pieces of
 * state that must be singular (the key listener, the event bus, the active
 * theme). Nothing here knows about extensions, PM, or the DOM beyond one
 * `keydown` listener — `host.ts` scopes this surface per extension and
 * `install.ts` wires it to the legacy app.
 */
import type {
  CommandDefinition,
  Disposable,
  EffectDefinition,
  ExtensionLayerDefinition,
  KernelEvents,
  InspectorSectionDefinition,
  KeybindingDefinition,
  MenuContribution,
  MenuLocation,
  PaletteProvider,
  PanelDefinition,
  StatusItem,
  ThemeDefinition,
  TransitionDefinition
} from './api';
import { chordMatches, chordModifierCount, chordOfEvent, hasTextSelection, isFieldTarget, selectableTextRoot, selectTextContents, normalizeChord } from './keychord';
import { Registry } from './registry';
import * as glslHelpers from './glsl';
import { validateEffect, validateTransition } from './glsl';
import { validateExtensionLayerDefinition } from './extension-layers';
import { createServicesRegistry, type ServicesRegistry } from './services';

/* ── keybindings ─────────────────────────────────────────── */

export interface KeybindingEntry extends KeybindingDefinition {
  /** `${chord}::${command}` — the registry id. */
  id: string;
  chord: string;
  priority: number;
  inFields: boolean;
  repeat: boolean;
  ownerId: string;
}

export const bindingId = (chord: string, command: string): string => `${chord}::${command}`;

/* ── palette / menus ─────────────────────────────────────── */

export interface PaletteProviderEntry {
  ownerId: string;
  provider: PaletteProvider;
}

export interface MenuEntry {
  ownerId: string;
  items: (ctx: Record<string, unknown>) => MenuContribution[];
}

/* ── events ──────────────────────────────────────────────── */

type AnyHandler = (payload: never) => void;

class EventBus {
  private map = new Map<string, Array<{ ownerId: string; fn: AnyHandler }>>();

  on<K extends keyof KernelEvents>(event: K, fn: (payload: KernelEvents[K]) => void, ownerId = 'kernel'): Disposable {
    const key = String(event);
    let list = this.map.get(key);
    if (!list) {
      list = [];
      this.map.set(key, list);
    }
    const record = { ownerId, fn: fn as AnyHandler };
    list.push(record);
    return {
      dispose: () => {
        const current = this.map.get(key);
        if (!current) return;
        const index = current.indexOf(record);
        if (index >= 0) current.splice(index, 1);
      }
    };
  }

  emit<K extends keyof KernelEvents>(event: K, payload: KernelEvents[K]): void {
    const list = this.map.get(String(event));
    if (!list) return;
    for (const record of [...list]) {
      try {
        (record.fn as (value: KernelEvents[K]) => void)(payload);
      } catch (error) {
        console.error(`[kernel] event handler failed: ${String(event)}`, error);
      }
    }
  }

  disposeOwner(ownerId: string): void {
    for (const [key, list] of this.map) {
      const kept = list.filter((record) => record.ownerId !== ownerId);
      if (kept.length) this.map.set(key, kept);
      else this.map.delete(key);
    }
  }

  clear(): void {
    this.map.clear();
  }
}

/* ── theme state ─────────────────────────────────────────── */

export type SchemePreference = 'light' | 'dark' | 'system';

export interface ThemeState {
  activeId: string;
  scheme: SchemePreference;
}

/* ── kernel ──────────────────────────────────────────────── */

export type CommandRunner = (command: string, args: unknown[]) => unknown;

export interface Kernel {
  readonly panels: Registry<PanelDefinition>;
  readonly inspectorSections: Registry<InspectorSectionDefinition>;
  readonly commands: Registry<CommandDefinition>;
  readonly keybindings: Registry<KeybindingEntry>;
  readonly effects: Registry<EffectDefinition>;
  readonly transitions: Registry<TransitionDefinition>;
  readonly layerTypes: Registry<ExtensionLayerDefinition>;
  readonly themes: Registry<ThemeDefinition>;
  readonly status: Registry<StatusItem>;
  readonly services: ServicesRegistry;
  readonly events: EventBus;
  readonly theme: ThemeState;
  /**
   * Shader codegen helpers, reachable from legacy code that only holds the
   * kernel object (`PM.Kernel.glsl.toLegacyTransition`, gl/transitions.ts).
   */
  readonly glsl: typeof glslHelpers;

  /** Active bindings for a chord, lowest priority first, registration order within a priority. */
  bindingsFor(chord: string): KeybindingEntry[];
  bind(ownerId: string, def: KeybindingDefinition): Disposable;
  unbind(ownerId: string, key: string, all?: boolean): void;
  listBindings(): KeybindingEntry[];

  registerEffect(ownerId: string, def: EffectDefinition): Disposable;
  registerTransition(ownerId: string, def: TransitionDefinition): Disposable;
  registerLayerType(ownerId: string, def: ExtensionLayerDefinition): Disposable;

  registerPaletteProvider(ownerId: string, provider: PaletteProvider): Disposable;
  paletteProviders(): PaletteProviderEntry[];

  contributeMenu(ownerId: string, location: MenuLocation, items: (ctx: Record<string, unknown>) => MenuContribution[]): Disposable;
  collectMenu(location: MenuLocation, ctx?: Record<string, unknown>): MenuContribution[];

  activateTheme(id: string): void;
  setScheme(mode: SchemePreference): void;

  /** Install the ONE window keydown listener that dispatches keybindings. */
  installKeyListener(runCommand: CommandRunner, target?: EventTarget): Disposable;

  disposeOwner(ownerId: string): void;
  dispose(): void;
}

export function createKernel(): Kernel {
  const panels = new Registry<PanelDefinition>();
  const inspectorSections = new Registry<InspectorSectionDefinition>();
  const commands = new Registry<CommandDefinition>();
  const keybindings = new Registry<KeybindingEntry>();
  const effects = new Registry<EffectDefinition>();
  const transitions = new Registry<TransitionDefinition>();
  const layerTypes = new Registry<ExtensionLayerDefinition>();
  const themes = new Registry<ThemeDefinition>();
  const status = new Registry<StatusItem>();
  const services = createServicesRegistry();
  const events = new EventBus();
  inspectorSections.onChange(() => events.emit('inspector:changed', undefined));
  const theme: ThemeState = { activeId: 'default', scheme: 'system' };

  const paletteProviders: PaletteProviderEntry[] = [];
  const menus = new Map<MenuLocation, MenuEntry[]>();
  const keyListeners = new Set<Disposable>();

  const kernel: Kernel = {
    panels,
    inspectorSections,
    commands,
    keybindings,
    effects,
    transitions,
    layerTypes,
    themes,
    status,
    services,
    events,
    theme,
    glsl: glslHelpers,

    bindingsFor(chord) {
      const wanted = normalizeChord(chord);
      if (!wanted) return [];
      return keybindings
        .entries()
        .filter((entry) => chordMatches(entry.item.chord, wanted, entry.item.looseModifiers === true))
        .sort((a, b) => {
          const priority = a.item.priority - b.item.priority;
          if (priority) return priority;
          const exact = Number(b.item.chord === wanted) - Number(a.item.chord === wanted);
          if (exact) return exact;
          if (a.item.looseModifiers || b.item.looseModifiers) {
            const specificity = chordModifierCount(b.item.chord) - chordModifierCount(a.item.chord);
            if (specificity) return specificity;
          }
          return a.seq - b.seq;
        })
        .map((entry) => entry.item);
    },

    bind(ownerId, def) {
      const chord = normalizeChord(def?.key ?? '');
      if (!chord) throw new Error(`keybindings: invalid key "${String(def?.key)}"`);
      if (typeof def.command !== 'string' || !def.command) throw new Error('keybindings: "command" is required');
      const entry: KeybindingEntry = {
        ...def,
        id: bindingId(chord, def.command),
        chord,
        priority: def.priority ?? 0,
        inFields: def.inFields === true,
        repeat: def.repeat === true,
        ownerId
      };
      return keybindings.register(ownerId, entry);
    },

    unbind(ownerId, key, all = false) {
      const chord = normalizeChord(key);
      if (!chord) return;
      for (const entry of keybindings.entries()) {
        if (entry.item.chord !== chord) continue;
        if (!all && entry.ownerId !== ownerId) continue;
        keybindings.disposeEntry(entry);
      }
    },

    listBindings() {
      return keybindings.list();
    },

    registerEffect(ownerId, def) {
      validateEffect(def);
      return effects.register(ownerId, def);
    },

    registerTransition(ownerId, def) {
      validateTransition(def);
      return transitions.register(ownerId, def);
    },

    registerLayerType(ownerId, def) {
      validateExtensionLayerDefinition(def);
      return layerTypes.register(ownerId, def);
    },

    registerPaletteProvider(ownerId, provider) {
      if (typeof provider !== 'function') throw new Error('palette: provider must be a function');
      const entry: PaletteProviderEntry = { ownerId, provider };
      paletteProviders.push(entry);
      return {
        dispose: () => {
          const index = paletteProviders.indexOf(entry);
          if (index >= 0) paletteProviders.splice(index, 1);
        }
      };
    },

    paletteProviders() {
      return [...paletteProviders];
    },

    contributeMenu(ownerId, location, items) {
      if (typeof items !== 'function') throw new Error('menus: items must be a function');
      let list = menus.get(location);
      if (!list) {
        list = [];
        menus.set(location, list);
      }
      const entry: MenuEntry = { ownerId, items };
      list.push(entry);
      return {
        dispose: () => {
          const current = menus.get(location);
          if (!current) return;
          const index = current.indexOf(entry);
          if (index >= 0) current.splice(index, 1);
        }
      };
    },

    collectMenu(location, ctx = {}) {
      const out: MenuContribution[] = [];
      for (const entry of menus.get(location) ?? []) {
        let items: MenuContribution[] | undefined;
        try {
          items = entry.items(ctx);
        } catch (error) {
          console.error(`[kernel] menu contribution failed (${entry.ownerId} → ${location})`, error);
          continue;
        }
        if (Array.isArray(items)) out.push(...items);
      }
      return out;
    },

    activateTheme(id) {
      const def = themes.get(id);
      theme.activeId = id;
      const scheme = def?.scheme === 'light' || def?.scheme === 'dark' ? def.scheme : resolvedScheme(theme.scheme);
      events.emit('theme:changed', { id, scheme });
    },

    setScheme(mode) {
      theme.scheme = mode;
      events.emit('theme:changed', { id: theme.activeId, scheme: resolvedScheme(mode) });
    },

    installKeyListener(runCommand, target) {
      const host: EventTarget | undefined = target ?? (typeof window === 'undefined' ? undefined : window);
      if (!host) return { dispose: () => {} };
      const handler = (raw: Event): void => {
        const event = raw as KeyboardEvent;
        const chord = chordOfEvent(event);
        if (!chord) return;
        // Enter belongs to a focused control before canvas editing shortcuts.
        if (chord === 'enter' && (event.target as Element | null)?.closest?.('button,a[href],select,[role="button"],[role="radio"]')) return;
        /* A highlighted transcript or other selectable document surface owns
           Copy just like a focused input. Let Chromium place that text on the
           system clipboard instead of dispatching the editor's Copy Layers. */
        if ((chord === 'cmd+c' || chord === 'ctrl+c') && hasTextSelection()) return;
        /* isFieldTarget also walks to an editable ancestor, so nested markup
           inside a panel editor stays in the native text-editing context. */
        const field = isFieldTarget(event.target) || (event.composedPath?.() || []).some(isFieldTarget);
        const textRoot = !field && selectableTextRoot(event.target);
        if (textRoot) {
          if (chord === 'cmd+a' || chord === 'ctrl+a') {
            event.preventDefault(); selectTextContents(textRoot);
          }
          // Read-only conversation text must not cut, paste, select or nudge layers.
          return;
        }
        for (const binding of kernel.bindingsFor(chord)) {
          if (field && !binding.inFields) continue;
          if (event.repeat && !binding.repeat) continue;
          /* Field-aware bindings may deliberately decline to leave the native
             field behavior alone (the built-in Escape-to-blur binding does).
             Ordinary editor-level bindings preserve HEAD's prevent-before-run
             order; repeated events defer cancellation until handled. */
          /* A repeated keydown is only cancellable once a repeat-enabled
             command actually handles it. This lets a declined repeat fall
             through and leaves the browser default intact when every
             candidate is suppressed or declines. Ordinary keydowns retain
             the historical prevent-before-run behavior. */
          if (!event.repeat && !binding.inFields) event.preventDefault();
          let result: unknown;
          try {
            result = runCommand(binding.command, binding.args ?? []);
          } catch (error) {
            console.error(`[kernel] keybinding ${binding.chord} → ${binding.command} failed`, error);
            return;
          }
          if (result === false) continue;
          if (binding.inFields || event.repeat) event.preventDefault();
          return;
        }
      };
      host.addEventListener('keydown', handler);
      let disposed = false;
      const disposable: Disposable = {
        dispose: () => {
          if (disposed) return;
          disposed = true;
          host.removeEventListener('keydown', handler);
          keyListeners.delete(disposable);
        }
      };
      keyListeners.add(disposable);
      return disposable;
    },

    disposeOwner(ownerId) {
      panels.disposeOwner(ownerId);
      inspectorSections.disposeOwner(ownerId);
      commands.disposeOwner(ownerId);
      keybindings.disposeOwner(ownerId);
      effects.disposeOwner(ownerId);
      transitions.disposeOwner(ownerId);
      layerTypes.disposeOwner(ownerId);
      themes.disposeOwner(ownerId);
      status.disposeOwner(ownerId);
      events.disposeOwner(ownerId);
      for (let i = paletteProviders.length - 1; i >= 0; i--) {
        if (paletteProviders[i]?.ownerId === ownerId) paletteProviders.splice(i, 1);
      }
      for (const [location, list] of menus) {
        const kept = list.filter((entry) => entry.ownerId !== ownerId);
        if (kept.length) menus.set(location, kept);
        else menus.delete(location);
      }
    },

    dispose() {
      for (const listener of [...keyListeners]) listener.dispose();
      panels.clear();
      inspectorSections.clear();
      commands.clear();
      keybindings.clear();
      effects.clear();
      transitions.clear();
      layerTypes.clear();
      themes.clear();
      status.clear();
      services.clear();
      paletteProviders.length = 0;
      menus.clear();
      events.clear();
    }
  };

  return kernel;
}

/**
 * Run a command by id. A missing command returns `false` so the key listener
 * treats the binding as unhandled and falls through to the next one.
 */
export function runKernelCommand(kernel: Kernel, id: string, args: unknown[] = []): unknown {
  const command = kernel.commands.get(id);
  if (!command) {
    console.warn(`[kernel] no command "${id}"`);
    return false;
  }
  return command.run(...args);
}

function resolvedScheme(mode: SchemePreference): 'light' | 'dark' {
  if (mode === 'light' || mode === 'dark') return mode;
  const query = typeof globalThis.matchMedia === 'function' ? globalThis.matchMedia('(prefers-color-scheme: dark)') : null;
  return query?.matches ? 'dark' : 'light';
}
