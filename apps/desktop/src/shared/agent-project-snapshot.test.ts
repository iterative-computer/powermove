import { describe, expect, it } from 'vitest';
import { agentProjectSnapshot } from './agent-project-snapshot';
import { LIMITS } from './ipc';

describe('agent project snapshots', () => {
  it('preserves complete context for small projects', () => {
    const project = { id: 'project-one', name: 'One', revision: 3, layers: [{ id: 'layer-one', props: { x: { v: 10, kf: [] } } }] };
    expect(agentProjectSnapshot(project)).toBe(JSON.stringify(project));
  });

  it('keeps large projects within the request limit without changing their data', () => {
    const project = { id: 'project-one', name: 'One', revision: 3, w: 1920, h: 1080, fps: 30, dur: 10,
      layers: [{ id: 'layer-one', name: 'Model', type: 'scene3d', content: 'x'.repeat(LIMITS.codexProjectJsonBytes) }] };
    expect(Buffer.byteLength(JSON.stringify(project))).toBeGreaterThan(LIMITS.codexProjectJsonBytes);
    const serialized = agentProjectSnapshot(project);
    expect(Buffer.byteLength(serialized)).toBeLessThan(LIMITS.codexProjectJsonBytes);
    const snapshot = JSON.parse(serialized);
    expect(snapshot).toMatchObject({ id: project.id, revision: 3, w: 1920, h: 1080, layerCount: 1,
      layers: [{ id: 'layer-one', name: 'Model', type: 'scene3d' }], agentSnapshot: { complete: false } });
    expect(snapshot.agentSnapshot.note).toContain('get_project_state');
    expect(project.layers[0]!.content.length).toBe(LIMITS.codexProjectJsonBytes);
    expect(project).not.toHaveProperty('agentSnapshot');
  });

  it('bounds indexes and labels even when UTF-8 data pushes the project over the limit', () => {
    const project = { id: 'unicode-project', revision: 8, name: '🎬'.repeat(1_000),
      layers: Array.from({ length: 1_100 }, (_, index) => ({ id: `layer-${index}`, name: '🎬'.repeat(1_000),
        type: 'text', content: 'é'.repeat(16_000) })) };
    const full = JSON.stringify(project);
    expect(full.length).toBeLessThan(LIMITS.codexProjectJsonBytes);
    expect(Buffer.byteLength(full)).toBeGreaterThan(LIMITS.codexProjectJsonBytes);
    const serialized = agentProjectSnapshot(project);
    expect(Buffer.byteLength(serialized)).toBeLessThan(LIMITS.codexProjectJsonBytes);
    const snapshot = JSON.parse(serialized);
    expect(snapshot.layerCount).toBe(1_100);
    expect(snapshot.layers).toHaveLength(1_000);
    expect(snapshot.name.length).toBeLessThanOrEqual(256);
    expect(snapshot.layers[0]).not.toHaveProperty('content');
  });
});
