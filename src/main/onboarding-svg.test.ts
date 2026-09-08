// @vitest-environment happy-dom
/// <reference lib="dom" />
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

import { createEngine } from '../renderer/public/onboarding/player.js';
import {
  combinedPathMatrix,
  createOnboardingSvgPlayer,
  gradientState,
  pathData,
  sourceGate,
  tintVisibleGlow,
  unmixWhite
} from '../renderer/public/onboarding/svg-player.js';

const scenePath = path.resolve('src/renderer/public/onboarding/scene.json');
const scene = JSON.parse(await readFile(scenePath, 'utf8'));

function expectedCombinedMatrix(engine: any, layer: any, vectorPath: any, time: number): number[] {
  const prefix = `g.${vectorPath.id}`;
  const read = (key: string, fallback = 0) => Number(engine.evP(layer, vectorPath.p[key], time, `${prefix}.${key}`) ?? fallback);
  const radians = read('rotation') * Math.PI / 180;
  const cosine = Math.cos(radians), sine = Math.sin(radians);
  const local = [
    cosine * read('scaleX', 100) / 100, sine * read('scaleX', 100) / 100,
    -sine * read('scaleY', 100) / 100, cosine * read('scaleY', 100) / 100,
    read('x'), read('y')
  ];
  return engine.mul(engine.worldMatrix(layer, time), local);
}

describe('native SVG onboarding renderer', () => {
  it('removes white by unmixing it into alpha while preserving colored energy', () => {
    expect(unmixWhite('#FFFFFF')).toEqual({ color: '#000000', opacity: 0 });
    const peach = unmixWhite('#FFBE9C');
    expect(peach.opacity).toBeCloseTo(99 / 255, 6);
    expect(peach.color).toBe('#FF5800');
    const straight = [255, 88, 0].map((channel) => channel / 255);
    const composite = straight.map((channel) => channel * peach.opacity + 1 * (1 - peach.opacity));
    expect(composite[0]).toBeCloseTo(1, 6);
    expect(composite[1]).toBeCloseTo(190 / 255, 2);
    expect(composite[2]).toBeCloseTo(156 / 255, 6);
  });

  it('tints visible orange 70% toward white without reviving transparent source white', () => {
    expect(tintVisibleGlow('#FF5800')).toBe('#FFCDB3');
    const engine = createEngine(structuredClone(scene.project));
    const layer = engine.proj.layers.find((candidate: any) => candidate.type === 'shape');
    const effect = layer.fx.find((candidate: any) => candidate.type === 'glow');
    const stops = gradientState(engine, layer, effect, 2).stops;
    expect(stops.find((stop: any) => stop.opacity > 0)?.color).toBe('#FFCDB3');
    expect(stops.filter((stop: any) => stop.opacity === 0).every((stop: any) => stop.color === '#FFCDB3')).toBe(true);
  });

  it('evaluates cubic path geometry, transforms, opacity, and effects at sampled scene times', async () => {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    document.body.append(svg);
    const player = await createOnboardingSvgPlayer({ svg, scene, audio: false, hdr: false });
    const reference = createEngine(structuredClone(scene.project));

    for (const time of [0.5, 2, 4, 6, 8]) {
      await player.seek(time);
      reference.time = time;
      reference.beginEval?.(time);
      const activeLayer = reference.proj.layers.find((layer: any) => layer.type === 'shape' && reference.active(layer, time));
      if (!activeLayer) continue;
      const vectorPath = activeLayer.d.paths[0];
      const output = svg.querySelector(`[data-layer-id="${activeLayer.id}"]`)!;
      expect(output.getAttribute('display')).toBe('inline');
      const renderedPath = svg.querySelector(`#onboarding-bloom-${reference.proj.layers.filter((layer: any) => layer.type === 'shape').indexOf(activeLayer)} path`)!;
      const data = renderedPath.getAttribute('d')!;
      expect(data).toBe(pathData(reference, activeLayer, vectorPath, time));
      expect(data.match(/ C /g)).toHaveLength(vectorPath.vertices.length);
      expect(data.endsWith(' Z')).toBe(true);
      const actualMatrix = renderedPath.getAttribute('transform')!.match(/-?\d+(?:\.\d+)?(?:e[+-]?\d+)?/gi)!.map(Number);
      const expectedMatrix = expectedCombinedMatrix(reference, activeLayer, vectorPath, time);
      actualMatrix.forEach((value: number, index: number) => expect(value).toBeCloseTo(expectedMatrix[index]!, 8));
      combinedPathMatrix(reference, activeLayer, vectorPath, time)
        .forEach((value: number, index: number) => expect(value).toBeCloseTo(expectedMatrix[index]!, 8));
      expect(Number(output.querySelector('rect')!.getAttribute('opacity'))).toBeCloseTo(reference.worldOpacity(activeLayer, time), 8);

      const effect = activeLayer.fx.find((candidate: any) => candidate.type === 'glow');
      const gradient = gradientState(reference, activeLayer, effect, time);
      expect(gradient.stops.some((stop: any) => stop.opacity === 0)).toBe(true);
      expect(gradient.stops.some((stop: any) => stop.opacity > 0)).toBe(true);
      const renderedStops = Array.from(svg.querySelectorAll(`#onboarding-gradient-${reference.proj.layers.filter((layer: any) => layer.type === 'shape').indexOf(activeLayer)} stop`));
      renderedStops.forEach((stop, index) => {
        expect(Number((stop as SVGStopElement).dataset.hdrOpacity)).toBeCloseTo(gradient.stops[index]!.opacity, 8);
        expect(Number(stop.getAttribute('stop-opacity'))).toBeCloseTo(Math.min(1, gradient.stops[index]!.opacity * 3), 8);
      });
      expect(sourceGate(reference, activeLayer, vectorPath, effect, time)).toBeGreaterThan(0);
      const blur = svg.querySelector(`#onboarding-blur-${reference.proj.layers.filter((layer: any) => layer.type === 'shape').indexOf(activeLayer)} feGaussianBlur`)!;
      expect(Number(blur.getAttribute('stdDeviation'))).toBeCloseTo(gradient.radius / 2, 8);
    }
    expect(svg.querySelector('canvas')).toBeNull();
    player.destroy();
    expect(svg.children).toHaveLength(0);
  });
});
