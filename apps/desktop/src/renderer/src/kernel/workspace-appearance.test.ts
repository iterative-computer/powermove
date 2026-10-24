import { describe, expect, it } from 'vitest';
import type { WorkspaceAPI } from './api';
import type { WorkspaceManifest } from '../core/types/workspace';
import { ownWorkspaceAppearance, reconcileWorkspaceAppearance, retryPendingWorkspaceAppearance, type WorkspaceAppearanceBackend } from './workspace-appearance';

function harness(saved?: unknown) {
  const workspaces = new Map<string, WorkspaceManifest>();
  let active = 'design';
  let persisted = saved;
  const initial = () => ({ id: 'design', density: 'normal', theme: { accent: '#f60' },
    chrome: { previewCornerRadius: 'square', timeline: { rowHeight: 30, gutterWidth: 224 } },
    layout: { docks: [{ id: 'center', size: 600, panels: [{ id: 'timeline', size: 340 }] }] }
  }) as unknown as WorkspaceManifest;
  workspaces.set(active, initial());
  const api = {
    current: () => workspaces.get(active) ?? null,
    mutate(fn) {
      const ws = workspaces.get(active)!;
      fn(ws);
      const next = structuredClone(ws);
      workspaces.set(next.id, next);
      if (ws.id !== active) workspaces.delete(active);
      active = next.id;
      return next;
    }
  } as WorkspaceAPI;
  const backend: WorkspaceAppearanceBackend = {
    get: id => workspaces.get(id) ?? null,
    mutate: (id, fn) => { fn(workspaces.get(id)!); },
    load: () => persisted,
    save: state => { persisted = JSON.parse(JSON.stringify(state)); }
  };
  return { api, backend, workspaces, initial, switchTo: (id: string) => { active = id; },
    saved: () => persisted, own: (id: string) => ownWorkspaceAppearance(api, id, backend) };
}

describe('owned workspace appearance', () => {
  it('exposes a read snapshot and ignores late writes from an unloaded mod', () => {
    const h = harness(), mod = h.own('theme');
    mod.api.current()!.chrome.timeline.rowHeight = 22;
    expect(h.api.current()!.chrome.timeline.rowHeight).toBe(30);
    mod.dispose();
    mod.api.mutate(ws => { ws.chrome.timeline.rowHeight = 22; });
    expect(h.api.current()!.chrome.timeline.rowHeight).toBe(30);
  });

  it('restores nested appearance when a mod replaces its container', () => {
    const h = harness(), mod = h.own('theme');
    h.api.current()!.chrome.custom = { nested: 'before' };
    mod.api.mutate(ws => { ws.chrome.custom = 'after'; });
    mod.dispose();
    expect(h.api.current()!.chrome.custom).toEqual({ nested: 'before' });
  });

  it('restores all appearance and geometry while retaining later edits and new panels', () => {
    const h = harness(), mod = h.own('theme');
    mod.api.mutate(ws => {
      ws.density = 'compact'; ws.theme.accent = '#000'; ws.chrome.previewCornerRadius = 'rounded';
      ws.chrome.timeline.rowHeight = 22;
      ws.layout.docks[0]!.size = 800; ws.layout.docks[0]!.panels[0]!.size = 280;
    });
    h.api.mutate(ws => {
      ws.chrome.timeline.gutterWidth = 280;
      ws.layout.docks[0]!.panels.push({ id: 'new-panel', size: 200 });
      ws.layout.docks[0]!.panels.reverse();
    });
    mod.dispose();
    const ws = h.api.current()!;
    expect(ws.density).toBe('normal'); expect(ws.theme.accent).toBe('#f60');
    expect(ws.chrome.previewCornerRadius).toBe('square');
    expect(ws.chrome.timeline).toEqual({ rowHeight: 30, gutterWidth: 280 });
    expect(ws.layout.docks[0]!.size).toBe(600);
    expect(ws.layout.docks[0]!.panels).toEqual([{ id: 'new-panel', size: 200 }, { id: 'timeline', size: 340 }]);
  });

  it.each([true, false])('unwinds overlapping mods in either order (top first: %s)', topFirst => {
    const h = harness(), first = h.own('first'), second = h.own('second');
    first.api.mutate(ws => { ws.chrome.timeline.rowHeight = 22; });
    second.api.mutate(ws => { ws.chrome.timeline.rowHeight = 40; });
    if (topFirst) { second.dispose(); expect(h.api.current()!.chrome.timeline.rowHeight).toBe(22); first.dispose(); }
    else { first.dispose(); expect(h.api.current()!.chrome.timeline.rowHeight).toBe(40); second.dispose(); }
    expect(h.api.current()!.chrome.timeline.rowHeight).toBe(30);
  });

  it('keeps the original baseline across repeated writes and re-enable', () => {
    const h = harness(), mod = h.own('theme');
    for (const height of [22, 22, 40]) mod.api.mutate(ws => { ws.chrome.timeline.rowHeight = height; });
    mod.dispose(); mod.dispose();
    expect(h.api.current()!.chrome.timeline.rowHeight).toBe(30);
    const enabled = h.own('theme');
    enabled.api.mutate(ws => { ws.chrome.timeline.rowHeight = 22; });
    enabled.dispose();
    expect(h.api.current()!.chrome.timeline.rowHeight).toBe(30);
  });

  it('does not restore a stale mod value over a user adjustment', () => {
    const h = harness(), first = h.own('first'), second = h.own('second');
    first.api.mutate(ws => { ws.chrome.timeline.rowHeight = 22; });
    h.api.mutate(ws => { ws.chrome.timeline.rowHeight = 32; });
    second.api.mutate(ws => { ws.chrome.timeline.rowHeight = 40; });
    first.dispose(); second.dispose();
    expect(h.api.current()!.chrome.timeline.rowHeight).toBe(32);
  });

  it('restores every affected workspace and follows built-in cloning', () => {
    const h = harness(), mod = h.own('theme');
    mod.api.mutate(ws => { ws.id = 'design-edited'; ws.chrome.timeline.rowHeight = 22; });
    h.workspaces.set('other', { ...h.initial(), id: 'other' }); h.switchTo('other');
    mod.api.mutate(ws => { ws.chrome.timeline.rowHeight = 40; });
    mod.dispose();
    expect(h.workspaces.get('design-edited')!.chrome.timeline.rowHeight).toBe(30);
    expect(h.workspaces.get('other')!.chrome.timeline.rowHeight).toBe(30);
    expect(h.api.current()!.id).toBe('other');
  });

  it('restores deleted fields, removes added fields and preserves false values', () => {
    const h = harness(), first = h.own('first'), second = h.own('second');
    h.api.current()!.theme.radius = 0;
    first.api.mutate(ws => { ws.chrome.timeline.showTypeBadges = false; delete ws.theme.radius; });
    second.api.mutate(ws => { ws.chrome.timeline.showTypeBadges = true; });
    second.dispose();
    expect(h.api.current()!.chrome.timeline.showTypeBadges).toBe(false);
    first.dispose();
    expect(h.api.current()!.chrome.timeline).not.toHaveProperty('showTypeBadges');
    expect(h.api.current()!.theme.radius).toBe(0);
  });

  it('recovers the original baseline after a restart with the mod still enabled', () => {
    const old = harness(); old.own('theme').api.mutate(ws => { ws.chrome.timeline.rowHeight = 22; });
    const restarted = harness(old.saved());
    restarted.workspaces.set('design', structuredClone(old.api.current()!));
    reconcileWorkspaceAppearance(restarted.api, new Set(['theme']), restarted.backend);
    const mod = restarted.own('theme');
    mod.api.mutate(ws => { ws.chrome.timeline.rowHeight = 22; });
    mod.dispose();
    expect(restarted.api.current()!.chrome.timeline.rowHeight).toBe(30);
  });

  it('cleans a disabled mod after restart, including a workspace loaded later', () => {
    const old = harness(); old.own('theme').api.mutate(ws => { ws.chrome.timeline.rowHeight = 22; });
    const restarted = harness(old.saved()); restarted.workspaces.clear();
    reconcileWorkspaceAppearance(restarted.api, new Set(), restarted.backend);
    restarted.workspaces.set('design', structuredClone(old.api.current()!));
    retryPendingWorkspaceAppearance(restarted.api);
    expect(restarted.api.current()!.chrome.timeline.rowHeight).toBe(30);
    expect(restarted.saved()).toEqual({ version: 1, workspaces: [] });
  });

  it('does not resurrect a deleted dock or panel', () => {
    const h = harness(), mod = h.own('theme');
    mod.api.mutate(ws => { ws.layout.docks[0]!.panels[0]!.size = 280; });
    h.api.current()!.layout.docks = [];
    mod.dispose();
    expect(h.api.current()!.layout.docks).toEqual([]);
  });

  it('records partial mutations from a throwing callback for activation cleanup', () => {
    const h = harness(), mod = h.own('theme');
    expect(() => mod.api.mutate(ws => { ws.chrome.timeline.rowHeight = 22; throw Error('activation failed'); })).toThrow();
    mod.dispose();
    expect(h.api.current()!.chrome.timeline.rowHeight).toBe(30);
  });
});
