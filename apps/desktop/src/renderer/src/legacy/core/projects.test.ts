import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './projects';

function projectsRegistry(): { PM: PMRegistry; memory: Map<string, any> } {
  const memory = new Map<string, any>();
  const PM: PMRegistry = {
    version: '1.0.0',
    proj: null,
    store: {
      get(key: string, fallback: any) { return memory.has(key) ? memory.get(key) : fallback; },
      set(key: string, value: any) { memory.set(key, value); },
      del(key: string) { memory.delete(key); }
    }
  };
  install(PM);
  return { PM, memory };
}

describe('legacy project registry install', () => {
  it('validates many restored windows in one metadata read and removes duplicates and stale ids', () => {
    const { PM, memory } = projectsRegistry();
    const ids = Array.from({ length: 1000 }, (_, index) => `P${index}`);
    memory.set('projects', ids.map(id => ({ id, name: id })));
    memory.set('openWindows', [...ids, ids[0], 'missing', null]);
    const get = vi.spyOn(PM.store, 'get');
    expect(PM.Projects.openProjects()).toEqual(ids);
    expect(get.mock.calls.filter(([key]) => key === 'projects')).toHaveLength(1);
  });

  it('reads a pre-window profile\'s open tabs as its open windows', () => {
    const { PM, memory } = projectsRegistry();
    for (const id of ['A', 'B']) PM.Projects.put({ id, name: id, layers: [] });
    memory.set('openTabs', ['A', 'B', 'GONE']);

    expect(PM.Projects.openProjects()).toEqual(['A', 'B']);

    // The window-era key wins outright once the native side has written one.
    memory.set('openWindows', ['B']);
    expect(PM.Projects.openProjects()).toEqual(['B']);
  });

  it('renames the active live document without reverting unsaved layers to the saved snapshot', () => {
    const { PM } = projectsRegistry();
    const files = new Map<string, any>([['P2', { path: '/tmp/Other.pmv', dirty: false }]]);
    PM.projectFileState = (id: string) => files.get(id);
    PM.Projects.put({ id: 'P1', name: 'Before', layers: [{ id: 'old' }] });
    PM.proj = { id: 'P1', name: 'Before', layers: [{ id: 'old' }, { id: 'new' }] };
    expect(PM.Projects.rename('P1', ' After ')).toBe('After');
    expect(PM.Projects.get('P1')).toEqual(PM.proj);
    expect(PM.Projects.get('P1').layers).toHaveLength(2);
    PM.Projects.put({ id: 'P2', name: 'Other', layers: [] });
    PM.Projects.rename('P2', 'Inactive');
    expect(PM.proj.name).toBe('After');
    expect(PM.Projects.get('P2').name).toBe('Inactive');
    expect(files.get('P2').dirty).toBe(true);
  });

  it('round-trips project data and metadata', () => {
    const { PM } = projectsRegistry();
    const project = { id: 'P1', name: 'Hero', layers: [{ id: 'La' }] };

    PM.proj = project;
    PM.Projects.put(project);

    expect(PM.Projects.list().map((meta: any) => meta.name)).toEqual(['Hero']);
    expect(PM.Projects.get('P1')).toMatchObject(project);
  });

  it('keeps window order stable and drops unknown projects', () => {
    const { PM, memory } = projectsRegistry();
    for (const id of ['A', 'B']) PM.Projects.put({ id, name: id, layers: [] });
    memory.set('openWindows', ['A', 'B', 'A', 'GONE', null]);

    expect(PM.Projects.openProjects()).toEqual(['A', 'B']);
  });

  it('chooses content before a named empty fallback', () => {
    const { PM } = projectsRegistry();
    const get = (id: string) => ({ id, name: id === 'named' ? 'Storyboard' : 'Untitled', layers: id === 'hero' ? [{}] : [] });

    expect(PM.Projects.pickBoot({ open: ['blank'], metas: [{ id: 'blank' }, { id: 'named' }], get }).id).toBe('named');
    expect(PM.Projects.pickBoot({ open: ['blank'], metas: [{ id: 'blank' }, { id: 'hero' }], get }).id).toBe('hero');
  });

  it('restores the last active open project, including a deliberately empty composition', () => {
    const { PM } = projectsRegistry();
    const get = (id: string) => ({ id, name: id, layers: id === 'older' ? [{}] : [] });
    const project = PM.Projects.pickBoot({ open: ['older', 'current'], metas: [], get, getState: (id: string) => ({ lastActiveAt: id === 'current' ? 20 : 10 }) });
    expect(project.id).toBe('current');
    expect(project.layers).toEqual([]);
  });

  it('never boots a second window onto a document another window already has', () => {
    const { PM } = projectsRegistry();
    const get = (id: string) => ({ id, name: id, layers: [{}] });
    const getState = (id: string) => ({ lastActiveAt: id === 'current' ? 20 : 10 });

    // With nothing taken, the most recently active project wins as before.
    expect(PM.Projects.pickBoot({ open: ['older', 'current'], metas: [], get, getState }).id).toBe('current');
    // The window holding it is skipped, all the way down to no choice at all.
    expect(PM.Projects.pickBoot({ open: ['older', 'current'], metas: [], get, getState, taken: ['current'] }).id).toBe('older');
    expect(PM.Projects.pickBoot({ open: ['older', 'current'], metas: [], get, getState, taken: ['current', 'older'] })).toBeNull();
  });
});
