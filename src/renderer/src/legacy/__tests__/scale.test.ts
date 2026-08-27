import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { makePM } from './make-pm';

const baselinePath = path.join(process.cwd(), 'tests-vitest/fixtures/perf-baseline.json');
const CALIBRATION_ITERATIONS = 2_000_000;
const calibrationValues = Float64Array.from(
  { length: 1024 },
  (_value, index) => (index % 97) + 0.25,
);

interface PerformanceBaseline {
  calibrationMs: number;
  evalMedianMs: number;
  exprMedianMs: number;
  evalRatio: number;
  exprRatio: number;
  recordedAt: string;
  node: string;
}

function animModel(): PMRegistry {
  return makePM('core/easing', 'core/model', 'core/anim');
}

function scaleScene(PM: PMRegistry, expression: string | null = null): void {
  const makeLayer = (index: number) => {
    const properties: Record<string, unknown> = {};
    for (const key of ['position.x', 'position.y', 'scale.x', 'scale.y', 'rotation', 'opacity']) {
      properties[key] = {
        v: index,
        kf: Array.from({ length: 60 }, (_value, keyIndex) => ({
          t: keyIndex * 0.1,
          v: Math.sin(index + keyIndex) * 100,
          eo: [0.5, 0],
          ei: [0.5, 1],
          i: `k${keyIndex}`,
        })),
        expr: key === 'position.x' ? expression : null,
      };
    }
    return {
      id: `L${index}`,
      parent: index ? `L${index - 1}` : null,
      from: 0,
      dur: 1e9,
      p: properties,
    };
  };

  PM.proj = {
    layers: Array.from({ length: 1000 }, (_value, index) => makeLayer(index)),
    params: {},
    fps: 30,
  };
}

function evaluateSecond(PM: PMRegistry): { acc: number; ms: number } {
  const startedAt = performance.now();
  let acc = 0;
  for (let frame = 0; frame < 30; frame++) {
    const time = frame / 30;
    PM.beginEval(time);
    // The previous baseline accidentally reused the first frame's hierarchy.
    // Keep the benchmark honest: every timestamp must evaluate new transforms.
    expect(PM.worldMatrix(PM.proj.layers[0], time)[4]).toBeCloseTo(PM.ev(PM.proj.layers[0], 'position.x', time));
    for (let index = 0; index < 1000; index++) {
      acc += PM.ev(PM.proj.layers[index], 'position.x', time);
      acc += PM.worldOpacity(PM.proj.layers[index], time);
    }
    for (const layer of PM.proj.layers.filter((_layer: unknown, index: number) => index % 10 === 0)) {
      acc += PM.worldMatrix(layer, time)[4];
    }
  }
  return { acc, ms: performance.now() - startedAt };
}

function medianBenchmark(PM: PMRegistry): number {
  for (let index = 0; index < 3; index++) {
    expect(Number.isFinite(evaluateSecond(PM).acc)).toBe(true);
  }
  const samples = Array.from({ length: 7 }, () => evaluateSecond(PM).ms).sort((a, b) => a - b);
  return samples[3]!;
}

function calibrateOnce(): { acc: number; ms: number } {
  const startedAt = performance.now();
  let acc = 0;
  for (let index = 0; index < CALIBRATION_ITERATIONS; index++) {
    acc += calibrationValues[index & 1023]! * 1.000001 + (index & 7);
  }
  return { acc, ms: performance.now() - startedAt };
}

function calibrationMedian(): number {
  for (let index = 0; index < 2; index++) {
    expect(Number.isFinite(calibrateOnce().acc)).toBe(true);
  }
  const samples = Array.from({ length: 5 }, () => calibrateOnce().ms).sort((a, b) => a - b);
  return samples[2]!;
}

describe('animation evaluation scale', () => {
  it('stays interactive-scale with 1000 layers x 60 keys', () => {
    const PM = animModel();
    scaleScene(PM);
    const { acc, ms } = evaluateSecond(PM);

    expect(Number.isFinite(acc)).toBe(true);
    expect(ms, `full-project evaluation took ${ms.toFixed(0)}ms (budget 4s)`).toBeLessThan(4000);
  });

  it('keeps evaluation medians within the recorded TypeScript performance envelope', () => {
    const plain = animModel();
    scaleScene(plain);
    const evalMedianMs = medianBenchmark(plain);

    const expressionHeavy = animModel();
    scaleScene(expressionHeavy, 'value + Math.sin(t) * 10');
    const exprMedianMs = medianBenchmark(expressionHeavy);
    const calibrationMs = calibrationMedian();
    const evalRatio = evalMedianMs / calibrationMs;
    const exprRatio = exprMedianMs / calibrationMs;
    const current = {
      calibrationMs,
      evalMedianMs,
      exprMedianMs,
      evalRatio,
      exprRatio,
      recordedAt: new Date().toISOString(),
      node: process.version,
    };
    console.log(
      `scale metrics: evalMedian=${evalMedianMs.toFixed(2)}ms `
      + `exprMedian=${exprMedianMs.toFixed(2)}ms calibration=${calibrationMs.toFixed(2)}ms`,
    );

    if (process.env.PM_RECORD_BASELINE === '1') {
      mkdirSync(path.dirname(baselinePath), { recursive: true });
      writeFileSync(baselinePath, `${JSON.stringify(current, null, 2)}\n`);
      return;
    }

    expect(
      existsSync(baselinePath),
      `Performance baseline is missing at ${baselinePath}; `
      + 'run PM_RECORD_BASELINE=1 npx vitest run src/renderer/src/legacy/__tests__/scale.test.ts to record it intentionally',
    ).toBe(true);
    const baseline = JSON.parse(readFileSync(baselinePath, 'utf8')) as PerformanceBaseline;
    for (const field of ['calibrationMs', 'evalMedianMs', 'exprMedianMs', 'evalRatio', 'exprRatio'] as const) {
      const value = baseline[field];
      expect(
        Number.isFinite(value) && value > 0,
        `Performance baseline field ${field} must be a positive finite number; `
        + 'run PM_RECORD_BASELINE=1 npx vitest run src/renderer/src/legacy/__tests__/scale.test.ts to re-record it',
      ).toBe(true);
    }
    expect(
      evalRatio,
      `plain evaluation ratio ${evalRatio.toFixed(2)} exceeded baseline ${baseline.evalRatio.toFixed(2)} x 1.5 `
      + `(raw ${evalMedianMs.toFixed(2)}ms / calibration ${calibrationMs.toFixed(2)}ms)`,
    ).toBeLessThanOrEqual(baseline.evalRatio * 1.5);
    expect(
      exprRatio,
      `expression evaluation ratio ${exprRatio.toFixed(2)} exceeded baseline ${baseline.exprRatio.toFixed(2)} x 1.5 `
      + `(raw ${exprMedianMs.toFixed(2)}ms / calibration ${calibrationMs.toFixed(2)}ms)`,
    ).toBeLessThanOrEqual(baseline.exprRatio * 1.5);
  });
});
