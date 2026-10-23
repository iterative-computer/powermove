import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ fps: 30 });
  const layer = PM.mkLayer('solid');
  PM.proj.layers = [layer];
  const command = (time: number, value: number, extra: Record<string, unknown> = {}) => ({
    type: 'set_property', target: layer.id, path: 'rotation', mode: 'keyframe', time, value, ...extra,
  });
  return { PM, layer, command };
}

describe('set_property preset easing', () => {
  it.each([false, true])('applies complete presets across uneven segments in one batch (reverse=%s)', reverse => {
    const { PM, layer, command } = editor();
    const commands = [command(0, 10, { ease: 'glide' }), command(2, 110, { ease: 'glide' }), command(3, -40, { ease: 'glide' })];
    expect(PM.Edit.apply(reverse ? commands.reverse() : commands).ok).toBe(true);
    for (const [index, start] of layer.p.rotation.kf.entries()) {
      const end = layer.p.rotation.kf[index + 1];
      if (!end) continue;
      expect(PM.ev(layer, 'rotation', (start.t + end.t) / 2)).toBeCloseTo(start.v + (end.v - start.v) * PM.Ease.fn('glide')(.5), 4);
    }
  });

  it('updates an existing key when ease is explicit and preserves native easing when omitted', () => {
    const { PM, layer, command } = editor();
    PM.animate(layer, 'rotation', [{ t: 0, v: 10 }, { t: 2, v: 110 }]);
    expect(PM.Edit.apply([command(0, 10, { ease: 'glide' }), command(2, 110, { ease: 'glide' })]).ok).toBe(true);
    expect(PM.ev(layer, 'rotation', 1)).toBeCloseTo(10 + 100 * PM.Ease.fn('glide')(.5), 4);
    const ease = structuredClone(layer.p.rotation.kf[0].outEase);
    expect(PM.Edit.apply(command(0, 20)).ok).toBe(true);
    expect(layer.p.rotation.kf[0].outEase).toEqual(ease);
    expect(layer.p.rotation.kf[0].outInterp).toBe('bezier');
  });

  it('uses the last explicit preset and value when edits snap onto the same key', () => {
    const { PM, layer, command } = editor();
    expect(PM.Edit.apply([
      command(0, 0, { ease: 'quadIn' }), command(2, 110, { ease: 'glide' }),
      command(.001, 10, { ease: 'glide' }), command(.002, 20),
    ]).ok).toBe(true);
    expect(layer.p.rotation.kf).toHaveLength(2);
    expect(PM.ev(layer, 'rotation', 1)).toBeCloseTo(20 + 90 * PM.Ease.fn('glide')(.5), 4);
  });

  it('keeps hold edits and makes completed preset batches undoable', () => {
    const { PM, layer, command } = editor();
    expect(PM.Edit.apply([
      command(0, 10, { ease: 'glide', hold: true }), command(2, 110, { ease: 'glide' }),
    ]).ok).toBe(true);
    expect(PM.ev(layer, 'rotation', 1)).toBe(10);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L(layer.id).p.rotation.kf).toHaveLength(0);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.ev(PM.L(layer.id), 'rotation', 1)).toBe(10);
  });

  it('finishes presets before a later set_easing command so that later command wins', () => {
    const { PM, layer, command } = editor();
    PM.animate(layer, 'rotation', [{ t: 0, v: 10 }, { t: 2, v: 110 }]);
    expect(PM.Edit.apply([
      command(0, 10, { ease: 'glide' }), command(2, 110, { ease: 'glide' }),
      { type: 'set_easing', keyframes: layer.p.rotation.kf.map((key: any) => key.i), curve: [.11, 0, .5, 0] },
    ]).ok).toBe(true);
    expect(PM.ev(layer, 'rotation', 1)).toBeCloseTo(10 + 100 * PM.Ease.fn('quadIn')(.5), 4);
  });

  it('finishes presets before a later command captures the evaluated playhead value', () => {
    const { PM, layer, command } = editor();
    PM.time = 1;
    expect(PM.Edit.apply([
      command(0, 10, { ease: 'glide' }), command(2, 110, { ease: 'glide' }),
      { type: 'replace_keyframes', target: layer.id, path: 'rotation', keyframes: [] },
    ]).ok).toBe(true);
    expect(layer.p.rotation.kf).toHaveLength(0);
    expect(layer.p.rotation.v).toBeCloseTo(10 + 100 * PM.Ease.fn('glide')(.5), 4);
  });

  it('rolls back values and easing if a later command fails', () => {
    const { PM, layer, command } = editor();
    PM.animate(layer, 'rotation', [{ t: 0, v: 10 }, { t: 2, v: 110 }]);
    const before = JSON.stringify(layer.p.rotation);
    expect(PM.Edit.apply([
      command(0, 20, { ease: 'glide' }), command(2, 120, { ease: 'glide' }),
      { type: 'set_property', target: layer.id, path: 'missing', value: 1 },
    ]).ok).toBe(false);
    expect(JSON.stringify(PM.L(layer.id).p.rotation)).toBe(before);
  });

  it('completes presets inside a live edit before commit', () => {
    const { PM, layer, command } = editor();
    PM.Edit.begin('Live animation');
    expect(PM.Edit.apply([command(0, 10, { ease: 'glide' }), command(2, 110, { ease: 'glide' })]).ok).toBe(true);
    const expected = 10 + 100 * PM.Ease.fn('glide')(.5);
    expect(PM.ev(layer, 'rotation', 1)).toBeCloseTo(expected, 4);
    expect(PM.Edit.commit().ok).toBe(true);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L(layer.id).p.rotation.kf).toHaveLength(0);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.ev(PM.L(layer.id), 'rotation', 1)).toBeCloseTo(expected, 4);
  });
});
