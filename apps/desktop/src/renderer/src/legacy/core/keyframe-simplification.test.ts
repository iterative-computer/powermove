import { describe, expect, it, vi } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { linearKeyframeEase } from '../../core/anim/temporal-ease';
import { parseAgentEditCommand, parseEditCommand, ValidationError } from '../../core/validate/commands';
import { DENSE_KEYFRAME_THRESHOLD, noticeDenseKeyframes, snapshotKeyframes } from './keyframe-simplification';

function setup() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 10, fps: 30 });
  const a = PM.mkLayer('solid'), b = PM.mkLayer('solid'); PM.proj.layers = [a, b];
  PM.toast = vi.fn();
  const set = (layer: any, path: string, count = 300) => layer.p[path].kf = Array.from({ length: count }, (_, i) =>
    ({ t: i / 30, v: 50, i: `${layer.id}-${path}-${i}`, ...linearKeyframeEase() }));
  const apply = (command: any = { type: 'simplify_keyframes' }, meta = {}) => PM.Edit.apply(command, { label: 'Simplify Keyframes', origin: 'interface', ...meta });
  return { PM, a, b, set, apply };
}

describe('simplification commands', () => {
  it('validates project, selected layers and explicit properties for editor and agent', () => {
    for (const command of [{ type: 'simplify_keyframes' }, { type: 'simplify_keyframes', scope: 'layers' },
      { type: 'simplify_keyframes', scope: 'properties', properties: [{ layerId: 'a', path: 'position.x' }] }]) {
      expect(parseEditCommand(command)).not.toBeInstanceOf(ValidationError);
      expect(parseAgentEditCommand(command)).not.toBeInstanceOf(ValidationError);
    }
  });
  it.each([
    { scope: 'unknown' }, { scope: 'properties', properties: [] },
    { scope: 'layers', targets: [8] }, { scope: 'project', targets: ['a'] },
    { scope: 'properties', properties: [{ layerId: 'a', path: '__proto__' }] },
  ])('rejects malformed scope/targets/properties', fields => {
    expect(parseEditCommand({ type: 'simplify_keyframes', ...fields })).toBeInstanceOf(ValidationError);
  });
  it('simplifies the project as exactly one undo step and restores all key metadata on undo/redo', () => {
    const { PM, a, b, set, apply } = setup(); set(a, 'opacity'); set(b, 'rotation');
    const original = JSON.stringify(PM.proj.layers);
    const result = apply();
    expect(result.ok).toBe(true);
    expect(result.data.results[0].data).toMatchObject({ keysBefore: 600, keysAfter: 4, propertiesChanged: 2 });
    const simplified = JSON.stringify(PM.proj.layers);
    expect(PM.hist.undo()).toBe(true); expect(JSON.stringify(PM.proj.layers)).toBe(original);
    expect(PM.hist.undo()).toBe(false);
    expect(PM.hist.redo()).toBe(true); expect(JSON.stringify(PM.proj.layers)).toBe(simplified);
  });
  it('limits selected-layer and selected-property scopes', () => {
    const { PM, a, b, set, apply } = setup(); set(a, 'opacity'); set(a, 'rotation'); set(b, 'opacity');
    PM.selectLayers(a.id);
    expect(apply({ type: 'simplify_keyframes', scope: 'properties', properties: [{ layerId: a.id, path: 'opacity' }] }).ok).toBe(true);
    expect(a.p.opacity.kf).toHaveLength(2); expect(a.p.rotation.kf).toHaveLength(300);
    expect(apply({ type: 'simplify_keyframes', scope: 'layers' }).ok).toBe(true);
    expect(a.p.rotation.kf).toHaveLength(2); expect(b.p.opacity.kf).toHaveLength(300);
  });
  it('skips locked layers and expression consumers of another property', () => {
    const { a, b, set, apply } = setup(); set(a, 'opacity'); a.lock = true;
    expect(apply().ok).toBe(true); expect(a.p.opacity.kf).toHaveLength(300);
    a.lock = false; b.p.rotation.expr = 'layer.p.opacity.kf.length';
    expect(apply().ok).toBe(true); expect(a.p.opacity.kf).toHaveLength(300);
  });
  it.each(['nested', 'remap', 'stretch', 'parent', '3d'])('keeps out-of-range changing keys for %s consumers', kind => {
    const { PM, a, b, apply } = setup();
    a.dur = 1;
    a.p.opacity.kf = [-5, -1, 0, 1, 2, 5].map((t, i) => ({ t, v: i * 10, i: `k${i}`, ...linearKeyframeEase() }));
    if (kind === 'nested') PM.proj.comps.child = { layers: [], comps: {}, dur: 1, fps: 30 };
    if (kind === 'remap') b.d.timeRemap = { v: 0, kf: [], expr: null };
    if (kind === 'stretch') a.timeStretch = 2;
    if (kind === 'parent') b.parent = a.id;
    if (kind === '3d') a.threeD = true;
    expect(apply().ok).toBe(true); expect(a.p.opacity.kf).toHaveLength(6);
  });
  it('simplifies numeric effect, mask, text content and 3D properties without trimming', () => {
    const { PM, a, apply } = setup();
    const prop = () => ({ v: 5, expr: null, kf: [0, 1, 2].map(t => ({ t, v: 5, ...linearKeyframeEase() })) });
    a.fx = [{ id: 'fx', type: 'blur', p: { radius: prop() } }];
    a.masks = [{ id: 'mask', p: { feather: prop() } }];
    a.threeD = true; a.p['position.z'] = prop();
    expect(apply().ok).toBe(true);
    for (const path of ['fx.radius', 'm.mask.feather', 'position.z']) expect(PM.findProp(a, path).kf).toHaveLength(2);
  });
  it('rolls back an invalid command in a mixed batch', () => {
    const { PM, a, set, apply } = setup(); set(a, 'opacity');
    expect(apply([{ type: 'simplify_keyframes' }, { type: 'simplify_keyframes', scope: 'properties', properties: [{ layerId: a.id, path: 'missing' }] }]).ok).toBe(false);
    expect(PM.L(a.id).p.opacity.kf).toHaveLength(300); expect(PM.hist.undo()).toBe(false);
  });
  it('simplifies nested composition properties with undo even when that composition is closed', () => {
    const { PM, a, b, set, apply } = setup(); set(b, 'opacity');
    PM.proj.layers = [a];
    PM.proj.comps.child = { id: 'child', layers: [b], comps: {}, dur: 10, fps: 30 };
    PM.ProjectIndex.invalidate();
    expect(apply().ok).toBe(true);
    expect(b.p.opacity.kf).toHaveLength(2);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.comps.child.layers[0].p.opacity.kf).toHaveLength(300);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.proj.comps.child.layers[0].p.opacity.kf).toHaveLength(2);
  });
});

describe('ask-first dense-keyframe guard', () => {
  it.each(['agent', 'import', 'generated-tool', 'generated-ui', 'generated-script'])('offers consent after %s edit and adds a distinct simplification undo step', origin => {
    const { PM, a } = setup();
    const result = PM.Edit.apply({ type: 'replace_keyframes', target: a.id, path: 'opacity', keyframes:
      Array.from({ length: 300 }, (_, i) => ({ time: i / 30, value: 50 })) }, { origin, historyGroup: 'agent-run' });
    expect(result.ok).toBe(true); expect(result.message).toContain('Prefer fewer keyframes');
    expect(a.p.opacity.kf).toHaveLength(300);
    const notice = PM.toast.mock.calls.at(-1)[2];
    expect(notice).toMatchObject({ sticky: true, dismissLabel: 'Keep as is', action: { label: 'Simplify' } });
    notice.action.run(); expect(a.p.opacity.kf).toHaveLength(2);
    PM.hist.undo(); expect(PM.L(a.id).p.opacity.kf).toHaveLength(300);
    PM.hist.undo(); expect(PM.L(a.id).p.opacity.kf).toHaveLength(0);
    PM.hist.redo(); PM.hist.redo(); expect(PM.L(a.id).p.opacity.kf).toHaveLength(2);
  });
  it('never simplifies on dismissal or without consent', () => {
    const { PM, a, set } = setup(); set(a, 'opacity');
    noticeDenseKeyframes(PM);
    const notice = PM.toast.mock.calls.at(-1)[2]; notice.onClose(); notice.action.run();
    expect(a.p.opacity.kf).toHaveLength(300); expect(PM.hist.undo()).toBe(false);
  });
  it.each([DENSE_KEYFRAME_THRESHOLD, DENSE_KEYFRAME_THRESHOLD - 1])('does not trigger for %s keys', count => {
    const { PM, a, set } = setup(); set(a, 'opacity', count);
    expect(noticeDenseKeyframes(PM)).toBeNull(); expect(PM.toast).not.toHaveBeenCalled();
  });
  it('does not trigger for a small change on an existing dense property', () => {
    const { PM, a, set } = setup(); set(a, 'opacity');
    const before = snapshotKeyframes(PM); a.p.opacity.kf[0].v = 10;
    expect(noticeDenseKeyframes(PM, before)).toBeNull();
  });
  it('does not trigger for ordinary interface edits', () => {
    const { PM, a } = setup();
    PM.Edit.apply({ type: 'replace_keyframes', target: a.id, path: 'opacity', keyframes:
      Array.from({ length: 300 }, (_, i) => ({ time: i / 30, value: 50 })) }, { origin: 'interface' });
    expect(PM.toast).not.toHaveBeenCalled();
  });
  it.each(['project', 'property', 'gesture'])('ignores stale notices during changed %s', kind => {
    const { PM, a, set } = setup(); set(a, 'opacity'); noticeDenseKeyframes(PM);
    const action = PM.toast.mock.calls.at(-1)[2].action;
    if (kind === 'project') PM.proj.id = 'other';
    if (kind === 'property') a.p.opacity.kf[0].v = 45;
    if (kind === 'gesture') PM.Edit.begin('Current gesture');
    action.run(); expect(a.p.opacity.kf).toHaveLength(300);
    if (kind === 'gesture') PM.Edit.cancel();
  });
  it('detects dense keys written through a structural import and preserves them until consent', () => {
    const { PM, a, set } = setup();
    const result = PM.Edit.mutate('Import animation', () => set(a, 'opacity'), { origin: 'import' });
    expect(result.ok).toBe(true); expect(PM.toast).toHaveBeenCalledTimes(1);
    expect(a.p.opacity.kf).toHaveLength(300);
  });
  it('detects imported keys in a closed nested composition', () => {
    const { PM, b, set } = setup();
    PM.proj.layers = [];
    const result = PM.Edit.mutate('Import composition', () => {
      set(b, 'opacity');
      PM.proj.comps.child = { id: 'child', layers: [b], comps: {}, dur: 10, fps: 30 };
    }, { origin: 'import' });
    expect(result.ok).toBe(true); expect(b.p.opacity.kf).toHaveLength(300);
    PM.toast.mock.calls.at(-1)[2].action.run();
    expect(b.p.opacity.kf).toHaveLength(2);
  });
});
