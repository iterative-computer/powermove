import { describe, expect, it } from 'vitest';

import { compileExpression, type ExpressionContext } from './expression';

const context = (overrides: Partial<ExpressionContext> = {}): ExpressionContext => ({
  t: 0.5,
  T: 2,
  fps: 30,
  value: 10,
  layer: { name: 'Shape', p: { amount: 4 } },
  comp: { dur: 8, layers: [{}, {}, {}] },
  param: (name) => name === 'gain' ? 3 : 0,
  ch: 'position.x',
  idx: 2,
  ...overrides,
});

const run = (source: string, overrides: Partial<ExpressionContext> = {}) =>
  compileExpression(source)?.evaluate(context(overrides));

describe('project expression interpreter', () => {
  it('evaluates arithmetic, conditionals, safe members and math helpers', () => {
    expect(run('value + Math.sin(t) * 10')).toBeCloseTo(14.794255);
    expect(run('idx === 2 ? layer.name + " " + comp.dur : "no"')).toBe('Shape 8');
    expect(run('clamp(param("gain") * 5, 0, 12)')).toBe(12);
    expect(run('2 ** 3 ** 2')).toBe(512);
  });

  it('preserves deterministic animation helpers', () => {
    expect(run('linear(t, 0, 1, 0, 100)')).toBe(50);
    expect(run('ease(t, 0, 1, 0, 100)')).toBe(50);
    expect(run('random(4)')).toBe(run('random(4)'));
    expect(run('wiggle(2, 20, 7)')).toBeTypeOf('number');
    expect(run('pingpong(2, T)')).toBe(2);
  });

  it('rejects code execution and prototype escape syntax', () => {
    expect(compileExpression('value = 4')).toBeNull();
    expect(compileExpression('(() => 7)()')).toBeNull();
    expect(run('layer.constructor')).toBeUndefined();
    expect(run('Math.constructor("return globalThis")()')).toBeUndefined();
    expect(compileExpression('value;' + 'x'.repeat(2_100))).toBeNull();
  });
});
