import type { Kernel, SchemePreference, ThemeState } from './registries';

export interface ThemeStateBackend {
  load(): unknown;
  save(state: unknown): void;
}

type Field = keyof ThemeState;
type Entry = { base: string; stack: Array<{ owner: string; value: string }> };
const isScheme = (value: unknown): value is SchemePreference => ['light', 'dark', 'system'].includes(value as string);

class ThemeOverrides {
  private fields = new Map<Field, Entry>();
  private loadedSelection = false;
  private pendingSelection: string | null = null;
  constructor(private kernel: Kernel, private backend?: ThemeStateBackend) {
    const saved = backend?.load() as { version?: unknown; fields?: unknown } | null;
    if (saved?.version === 1 && Array.isArray(saved.fields)) for (const item of saved.fields) {
      if (!Array.isArray(item) || !['activeId', 'scheme'].includes(item[0])) continue;
      const field = item[0] as Field, entry = item[1] as Entry;
      const valid = field === 'scheme' ? isScheme : (value: unknown) => typeof value === 'string' && value.length > 0;
      if (!entry || !valid(entry.base) || !Array.isArray(entry.stack) || !entry.stack.length
        || !entry.stack.every(frame => typeof frame?.owner === 'string' && valid(frame.value))) continue;
      this.fields.set(field, structuredClone(entry));
    }
    this.loadedSelection = this.fields.has('activeId');
    kernel.themes.onChange(() => {
      if (!this.pendingSelection) return;
      if (kernel.theme.activeId !== 'default') { this.pendingSelection = null; return; }
      if (kernel.themes.has(this.pendingSelection)) {
        const id = this.pendingSelection; this.pendingSelection = null;
        kernel.activateTheme(id);
      }
    });
  }

  private save(): void {
    this.backend?.save({ version: 1, fields: [...this.fields] });
  }

  private set(field: Field, value: string): void {
    if (field === 'activeId') this.kernel.activateTheme(value);
    else this.kernel.setScheme(value as SchemePreference);
  }

  change(owner: string, field: Field, value: string): void {
    const previous = this.kernel.theme[field];
    if (previous === value) return;
    let entry = this.fields.get(field);
    // Boot temporarily selects Default before user themes have registered.
    const reapplying = field === 'activeId' && this.loadedSelection && previous === 'default'
      && entry?.stack.at(-1)?.owner === owner && entry.stack.at(-1)?.value === value;
    if (!entry || entry.stack.at(-1)?.value !== previous && !reapplying) this.fields.set(field, entry = { base: previous, stack: [] });
    if (field === 'activeId') { this.loadedSelection = false; this.pendingSelection = null; }
    entry.stack = entry.stack.filter(frame => frame.owner !== owner);
    entry.stack.push({ owner, value });
    this.save();
    this.set(field, value);
  }

  release(owner: string): void {
    for (const [field, entry] of this.fields) {
      if (!entry.stack.some(frame => frame.owner === owner)) continue;
      const top = entry.stack.at(-1)!;
      const bootFallback = field === 'activeId' && this.loadedSelection && this.kernel.theme.activeId === 'default';
      if (this.kernel.theme[field] !== top.value && !bootFallback) { this.fields.delete(field); continue; }
      entry.stack = entry.stack.filter(frame => frame.owner !== owner);
      if (top.owner === owner) {
        let value = entry.stack.at(-1)?.value ?? entry.base;
        if (field === 'activeId' && !this.kernel.themes.has(value)) {
          if (bootFallback) this.pendingSelection = value;
          value = 'default';
        }
        this.set(field, value);
      }
      if (!entry.stack.length) this.fields.delete(field);
    }
    this.save();
  }

  reconcile(owners: ReadonlySet<string>): void {
    const stale = new Set<string>();
    for (const entry of this.fields.values()) for (const { owner } of entry.stack) if (!owners.has(owner)) stale.add(owner);
    for (const owner of stale) this.release(owner);
  }
}

const states = new WeakMap<Kernel, ThemeOverrides>();
function managerFor(kernel: Kernel, backend?: ThemeStateBackend): ThemeOverrides {
  let manager = states.get(kernel);
  if (!manager) states.set(kernel, manager = new ThemeOverrides(kernel, backend));
  return manager;
}

export function reconcileThemeState(kernel: Kernel, owners: ReadonlySet<string>, backend?: ThemeStateBackend): void {
  managerFor(kernel, backend).reconcile(owners);
}

/** Theme selection and scheme changes belong to the calling mod as well as its CSS. */
export function ownThemeState(kernel: Kernel, owner: string, backend?: ThemeStateBackend): {
  activate(id: string): void; setScheme(mode: SchemePreference): void; dispose(): void;
} {
  const manager = managerFor(kernel, backend);
  let disposed = false;
  return {
    activate: id => { if (!disposed) manager.change(owner, 'activeId', id); },
    setScheme: mode => { if (!disposed) manager.change(owner, 'scheme', mode); },
    dispose() {
      if (disposed) return;
      disposed = true;
      manager.release(owner);
    }
  };
}
