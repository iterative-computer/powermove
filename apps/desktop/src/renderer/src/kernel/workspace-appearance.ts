import type { WorkspaceAPI } from './api';
import type { WorkspaceManifest } from '../core/types/workspace';

/** Host-only access to saved workspaces, including ones no longer active. */
export interface WorkspaceAppearanceBackend {
  get(id: string): WorkspaceManifest | null;
  mutate(id: string, fn: (workspace: WorkspaceManifest) => void): void;
  load?(): unknown;
  save?(state: unknown): void;
}

type PathPart = string | { dock: string } | { panel: string };
type Value = unknown;
type Override = { base: Value; stack: Array<{ owner: string; value: Value }> };
const safeKey = (key: string): boolean => !['__proto__', 'constructor', 'prototype'].includes(key);
const equal = (a: Value, b: Value): boolean => Object.is(a, b) || JSON.stringify(a) === JSON.stringify(b);

function values(workspace: WorkspaceManifest | null): Map<string, Value> {
  const result = new Map<string, Value>();
  const visit = (value: Value, path: PathPart[]): void => {
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      for (const [key, item] of Object.entries(value)) if (safeKey(key)) visit(item, [...path, key]);
    } else result.set(JSON.stringify(path), structuredClone(value));
  };
  if (!workspace) return result;
  visit(workspace.density, ['density']);
  visit(workspace.theme, ['theme']);
  visit(workspace.chrome, ['chrome']);
  // Address geometry by stable ids so later panel additions/reordering survive.
  for (const dock of workspace.layout?.docks ?? []) {
    const path: PathPart[] = ['layout', 'docks', { dock: dock.id }];
    for (const key of ['size', 'flex', 'hidden'] as const) visit(dock[key], [...path, key]);
    for (const panel of dock.panels) {
      for (const key of ['size', 'min', 'flex', 'collapsed', 'title'] as const) {
        visit(panel[key], [...path, 'panels', { panel: panel.id }, key]);
      }
    }
  }
  return result;
}

function write(workspace: WorkspaceManifest, key: string, value: Value): void {
  const path = JSON.parse(key) as PathPart[];
  let target: any = workspace;
  for (const part of path.slice(0, -1)) {
    if (typeof part === 'string') target = target[part] ??= {};
    else target = Array.isArray(target) ? target.find(item => item.id === ('dock' in part ? part.dock : part.panel)) : null;
    if (!target) return; // A deleted panel or dock stays deleted.
  }
  const field = path.at(-1) as string;
  if (value === undefined) delete target[field];
  else target[field] = structuredClone(value);
}

class AppearanceOverrides {
  private workspaces = new Map<string, Map<string, Override>>();
  private pending = new Set<string>();
  constructor(private backend: WorkspaceAppearanceBackend) {
    const saved = backend.load?.() as { version?: unknown; workspaces?: unknown } | undefined;
    if (saved?.version !== 1 || !Array.isArray(saved.workspaces)) return;
    for (const item of saved.workspaces) {
      if (!Array.isArray(item) || typeof item[0] !== 'string' || !Array.isArray(item[1])) continue;
      const fields = new Map<string, Override>();
      for (const field of item[1]) {
        if (!Array.isArray(field) || typeof field[0] !== 'string' || !validPath(field[0])) continue;
        const entry = field[1];
        if (!entry || !Array.isArray(entry.stack) || !entry.stack.length
          || !entry.stack.every((frame: any) => frame && typeof frame.owner === 'string')) continue;
        fields.set(field[0], structuredClone(entry));
      }
      if (fields.size) this.workspaces.set(item[0], fields);
    }
  }

  private save(): void {
    this.backend.save?.({ version: 1, workspaces: [...this.workspaces].map(([id, fields]) => [id, [...fields]]) });
  }

  reconcile(owners: ReadonlySet<string>): void {
    const stale = new Set(this.pending);
    for (const fields of this.workspaces.values()) for (const entry of fields.values()) {
      for (const { owner } of entry.stack) if (!owners.has(owner)) stale.add(owner);
    }
    for (const owner of stale) this.release(owner);
  }

  retryPending(): void {
    for (const owner of [...this.pending]) this.release(owner);
  }

  record(owner: string, id: string, before: Map<string, Value>, after: Map<string, Value>): void {
    this.pending.delete(owner);
    let fields = this.workspaces.get(id);
    for (const key of new Set([...before.keys(), ...after.keys()])) {
      const value = after.get(key);
      const previous = before.get(key);
      if (equal(previous, value)) continue;
      if (!fields) this.workspaces.set(id, fields = new Map());
      let entry = fields.get(key);
      // A change outside the tracked mod API becomes the new baseline.
      if (!entry || !equal(entry.stack.at(-1)?.value, previous)) {
        fields.set(key, entry = { base: previous, stack: [] });
      }
      entry.stack = entry.stack.filter(item => item.owner !== owner);
      entry.stack.push({ owner, value });
    }
    this.save();
  }

  release(owner: string): void {
    this.pending.delete(owner);
    for (const [id, fields] of this.workspaces) {
      const workspace = this.backend.get(id);
      if (!workspace) {
        if ([...fields.values()].some(entry => entry.stack.some(frame => frame.owner === owner))) this.pending.add(owner);
        continue;
      }
      const current = values(workspace);
      const restore = new Map<string, Value>();
      for (const [key, entry] of fields) {
        if (!entry.stack.some(item => item.owner === owner)) continue;
        const top = entry.stack.at(-1)!;
        if (!equal(current.get(key), top.value)) {
          // Retain user edits and explicit cleanup performed by the mod.
          fields.delete(key);
          continue;
        }
        entry.stack = entry.stack.filter(item => item.owner !== owner);
        if (top.owner === owner) restore.set(key, entry.stack.length ? entry.stack.at(-1)!.value : entry.base);
        if (!entry.stack.length) fields.delete(key);
      }
      if (!fields.size) this.workspaces.delete(id);
      if (restore.size) this.backend.mutate(id, ws => {
        // Remove replacement containers before rebuilding prior nested values.
        const ordered = [...restore].sort((a, b) => Number(a[1] !== undefined) - Number(b[1] !== undefined)
          || (JSON.parse(a[0]) as PathPart[]).length - (JSON.parse(b[0]) as PathPart[]).length);
        for (const [key, value] of ordered) write(ws, key, value);
      });
    }
    this.save();
  }
}

const managers = new WeakMap<WorkspaceAPI, AppearanceOverrides>();

function validPath(key: string): boolean {
  try {
    const path: PathPart[] = JSON.parse(key);
    if (!Array.isArray(path) || !path.length || typeof path.at(-1) !== 'string') return false;
    if (path[0] === 'density') return path.length === 1;
    if (path[0] === 'theme' || path[0] === 'chrome') return path.length > 1 && path.every(part => typeof part === 'string' && safeKey(part));
    const dock = path[2], panel = path[4];
    return path[0] === 'layout' && path[1] === 'docks' && !!dock && typeof dock === 'object' && 'dock' in dock && typeof dock.dock === 'string'
      && (path.length === 4 && ['size', 'flex', 'hidden'].includes(path[3] as string)
        || path.length === 6 && path[3] === 'panels' && !!panel && typeof panel === 'object' && 'panel' in panel && typeof panel.panel === 'string'
          && ['size', 'min', 'flex', 'collapsed', 'title'].includes(path[5] as string));
  } catch { return false; }
}

function managerFor(api: WorkspaceAPI, backend?: WorkspaceAppearanceBackend): AppearanceOverrides {
  let manager = managers.get(api);
  if (!manager) {
    manager = new AppearanceOverrides(backend ?? {
      get: id => api.current()?.id === id ? api.current() : null,
      mutate: (id, fn) => { if (api.current()?.id === id) api.mutate(fn, { inPlace: true }); }
    });
    managers.set(api, manager);
  }
  return manager;
}

/** Recover overrides whose mods were turned off or removed while this window was closed. */
export function reconcileWorkspaceAppearance(api: WorkspaceAPI, owners: ReadonlySet<string>, backend?: WorkspaceAppearanceBackend): void {
  managerFor(api, backend).reconcile(owners);
}

export function retryPendingWorkspaceAppearance(api: WorkspaceAPI): void {
  managers.get(api)?.retryPending();
}

/** Appearance writes through the mod API unwind with its registrations. */
export function ownWorkspaceAppearance(api: WorkspaceAPI, owner: string, backend?: WorkspaceAppearanceBackend): {
  api: WorkspaceAPI; dispose(): void;
} {
  const manager = managerFor(api, backend);
  let disposed = false;
  return {
    api: {
      ...api,
      current: () => {
        const workspace = api.current();
        return workspace ? JSON.parse(JSON.stringify(workspace)) as WorkspaceManifest : null;
      },
      mutate(fn, options) {
        if (disposed) return api.current();
        const before = values(api.current());
        // The host may normalize the workspace or clone a built-in on this write.
        let result: WorkspaceManifest | null = null;
        try { result = api.mutate(fn, options); return result; }
        finally {
          const after = result ?? api.current();
          if (!disposed && after) manager.record(owner, after.id, before, values(after));
        }
      }
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      manager.release(owner);
    }
  };
}
