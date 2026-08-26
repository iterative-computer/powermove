import { describe, expect, it } from 'vitest';

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
});
