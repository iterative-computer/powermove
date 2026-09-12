import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './library';

function libraryRegistry(): PMRegistry {
  let nextId = 0;
  const PM: PMRegistry = {
    proj: {
      id: 'project-1', name: 'Test project', dur: 10, fps: 30,
      layers: [{ id: 'layer-1', name: 'Hero', type: 'shape', from: 0 }],
    },
    uid: (prefix: string) => `${prefix}${++nextId}`,
    time: 0,
    Export: { snapshot: () => 'data:image/jpeg;base64,thumb' },
    bus: { emit() {} },
    toast() {},
  };
  install(PM);
  return PM;
}

describe('legacy library install', () => {
  it('snapshots selected layers into the project library', () => {
    const PM = libraryRegistry();
    const entry = PM.Library.saveSection('Hero pair', ['layer-1']);

    expect(entry.name).toBe('Hero pair');
    expect(entry.thumb.startsWith('data:image')).toBe(true);
    expect(entry.layers.map((layer: any) => layer.name)).toEqual(['Hero']);
    expect(entry.versions).toHaveLength(1);
  });
});
