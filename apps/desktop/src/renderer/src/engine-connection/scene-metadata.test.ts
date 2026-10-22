import { describe, expect, it } from 'vitest';
import { EngineSceneMetadata } from './scene-metadata';

describe('native presented scene metadata', () => {
  function fixture() {
    const object = { id: 'object', d: { definition: 'model' } };
    const nested = { id: 'nested', d: { definition: 'scene' } };
    const metadata = new Map<string, any>([['object', { udefs: ['extension-owned'] }]]);
    const definitions = new Map([['model', { id: 'model' }], ['scene', { id: 'scene' }]]);
    const PM = { EngineDoc: { enabled: true }, proj: { id: 'project', compId: 'root', layers: [object],
      comps: { child: { compId: 'child', layers: [nested] } } },
      layerDefinition: (id: string) => definitions.get(id),
      UIState: { setShaderMeta(layer: any, patch: any) { metadata.set(layer.id, { ...metadata.get(layer.id), ...patch }); } } };
    return { PM, metadata, apply: new EngineSceneMetadata(PM), definition: definitions.get('model') };
  }

  it('preserves warning order/duplicates and extension-owned fields like the compositor', () => {
    const { apply, metadata, definition } = fixture();
    const warnings = ['Missing texture: image', 'Missing texture: image', 'Missing model: model'];
    apply.apply({ seq: 1, revision: 7, scene3dMeta: [{ composition: 'root', layers: ['object'], warnings, error: null }] }, 'session');
    expect(metadata.get('object')).toEqual({ udefs: ['extension-owned'], definition, missing: false, sceneWarnings: warnings });
    apply.apply({ seq: 2, revision: 7, scene3dMeta: [{ composition: 'root', layers: ['object'], warnings: [], error: null }] }, 'session');
    expect(metadata.get('object').sceneWarnings).toEqual([]);
  });

  it('writes scene errors and nested-composition diagnostics, then restores missing:false', () => {
    const { apply, metadata } = fixture();
    apply.apply({ seq: 2, revision: 7, scene3dMeta: [{ composition: 'child', layers: ['nested'], warnings: [], error: 'ZodError: invalid scene' }] }, 'session');
    expect(metadata.get('nested')).toMatchObject({ missing: true, sceneError: 'ZodError: invalid scene' });
    // EngineDefinitions clears its generic failure before the good frame
    // is presented; the scene channel preserves the compositor's history.
    metadata.get('nested').sceneError = null;
    apply.apply({ seq: 3, revision: 8, scene3dMeta: [{ composition: 'child', layers: ['nested'], warnings: [], error: null }] }, 'session');
    // setShaderMeta merges patches today, so a successful draw keeps the
    // previous error string while its missing badge and warnings update.
    expect(metadata.get('nested')).toMatchObject({ missing: false, sceneWarnings: [], sceneError: 'ZodError: invalid scene' });
  });

  it('ignores stale shown frames and accepts reset sequence numbers in a new session', () => {
    const { apply, metadata, PM } = fixture();
    const report = { composition: 'root', layers: ['object'], warnings: ['Missing model: current'], error: null };
    apply.apply({ seq: 4, revision: 9, scene3dMeta: [report] }, 'session');
    apply.apply({ seq: 3, revision: 9, scene3dMeta: [{ ...report, warnings: ['stale'] }] }, 'session');
    apply.apply({ seq: 5, revision: 8, scene3dMeta: [{ ...report, warnings: ['stale'] }] }, 'session');
    expect(metadata.get('object').sceneWarnings).toEqual(report.warnings);
    apply.apply({ seq: 1, revision: 1, scene3dMeta: [{ ...report, warnings: [] }] }, 'new-session');
    expect(metadata.get('object').sceneWarnings).toEqual([]);
    PM.EngineDoc.enabled = false;
    apply.apply({ seq: 2, revision: 2, scene3dMeta: [report] }, 'new-session');
    expect(metadata.get('object').sceneWarnings).toEqual([]);
  });
});
