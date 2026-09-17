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
  it('keeps every open project beyond eight tabs and restores their stable order', () => {
    const { PM, memory } = projectsRegistry();
    const ids = Array.from({ length: 100 }, (_, index) => `P${index}`);
    for (const id of ids) {
      PM.Projects.put({ id, name: id, layers: [] });
      PM.Projects.markOpen(id);
    }
    PM.Projects.markOpen(ids[50]);
    install(PM);
    expect(PM.Projects.tabs()).toEqual(ids);
    expect(memory.get('openTabs')).toEqual(ids);
  });

  it('validates many restored tabs in one metadata read and removes duplicates and stale ids', () => {
    const { PM, memory } = projectsRegistry();
    const ids = Array.from({ length: 1000 }, (_, index) => `P${index}`);
    memory.set('projects', ids.map(id => ({ id, name: id })));
    memory.set('openTabs', [...ids, ids[0], 'missing', null]);
    const get = vi.spyOn(PM.store, 'get');
    expect(PM.Projects.tabs()).toEqual(ids);
    expect(get.mock.calls.filter(([key]) => key === 'projects')).toHaveLength(1);
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

  it('keeps open-tab order stable and drops unknown projects', () => {
    const { PM } = projectsRegistry();
    for (const id of ['A', 'B']) PM.Projects.put({ id, name: id, layers: [] });

    PM.Projects.markOpen('A');
    PM.Projects.markOpen('B');
    PM.Projects.markOpen('A');
    PM.Projects.markOpen('GONE');

    expect(PM.Projects.tabs()).toEqual(['A', 'B']);
  });

  it('chooses content before a named empty fallback', () => {
    const { PM } = projectsRegistry();
    const get = (id: string) => ({ id, name: id === 'named' ? 'Storyboard' : 'Untitled', layers: id === 'hero' ? [{}] : [] });

    expect(PM.Projects.pickBoot({ tabs: ['blank'], metas: [{ id: 'blank' }, { id: 'named' }], get }).id).toBe('named');
    expect(PM.Projects.pickBoot({ tabs: ['blank'], metas: [{ id: 'blank' }, { id: 'hero' }], get }).id).toBe('hero');
  });

  it('restores the last active open project, including a deliberately empty composition', () => {
    const { PM } = projectsRegistry();
    const get = (id: string) => ({ id, name: id, layers: id === 'older' ? [{}] : [] });
    const project = PM.Projects.pickBoot({ tabs: ['older', 'current'], metas: [], get, getState: (id: string) => ({ lastActiveAt: id === 'current' ? 20 : 10 }) });
    expect(project.id).toBe('current');
    expect(project.layers).toEqual([]);
  });
});
