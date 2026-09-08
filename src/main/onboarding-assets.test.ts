import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const root = path.resolve('src/renderer/public/onboarding');

describe('onboarding export assets', () => {
  it('keeps the supplied scene timing and sound-effect binding intact', async () => {
    const scene = JSON.parse(await readFile(path.join(root, 'scene.json'), 'utf8'));
    expect(scene.project).toMatchObject({ w: 1920, h: 1080, fps: 30, dur: 9.766666666666667 });
    const audio = scene.project.layers.find((layer: { type: string }) => layer.type === 'audio');
    expect(audio).toMatchObject({ from: 0, dur: 7.3, d: { asset: 'awojon75' } });
    expect(scene.assets.awojon75).toBe('assets/media-0.wav');
    const wav = await readFile(path.join(root, 'assets/media-0.wav'));
    expect(wav.subarray(0, 4).toString('ascii')).toBe('RIFF');
    expect(wav.byteLength).toBe(1_401_644);
  });

  it('uses live SVG vectors, transparent-white gradient stops, and keeps the desktop transparent', async () => {
    const animation = await readFile(path.join(root, 'animation.js'), 'utf8');
    const renderer = await readFile(path.join(root, 'svg-player.js'), 'utf8');
    const html = await readFile(path.join(root, 'animation.html'), 'utf8');
    const styles = await readFile(path.join(root, 'onboarding.css'), 'utf8');
    expect(animation).toContain('createOnboardingSvgPlayer');
    expect(animation).toContain('audio: true');
    expect(renderer).toContain("import { createEngine } from './player.js'");
    expect(renderer).toContain("svgNode('feGaussianBlur')");
    expect(renderer).toContain("candidate.type === effectDefinition.id");
    expect(renderer).toContain('WARM_WHITE_TINT = .7');
    expect(html).toContain('<svg id="onboarding-svg"');
    expect(html).not.toContain('<canvas');
    expect(renderer).not.toContain("createElement('canvas')");
    expect(styles).toContain('background: transparent');
    expect(styles).toContain('background: rgba(0, 0, 0, 0.45)');
    expect(styles).toContain('animation: onboarding-dim-in 400ms ease-out both');
    expect(styles).toContain('width: min(100vw, calc(100vh * 16 / 9))');
  });

  it('uses a validated extended-range float surface at native display resolution when HDR is available', async () => {
    const output = await readFile(path.join(root, 'hdr-output.js'), 'utf8');
    expect(output).toContain("format: 'rgba16float'");
    expect(output).toContain("colorType: 'float16'");
    expect(output).toContain("sourcePrecision: float16Canvas ? 'float16' : 'unorm8'");
    expect(output).toContain("format: float16Canvas ? 'rgba16float' : 'rgba8unorm'");
    expect(output).toContain("toneMapping: { mode: 'extended' }");
    expect(output).toContain("alphaMode: 'premultiplied'");
    expect(output).toContain("clearValue: { r: 2, g: .25, b: 0, a: 1 }");
    expect(output).toContain("await device.createRenderPipelineAsync(pipelineDescriptor)");
    expect(output).toContain("window.innerWidth * devicePixelRatio");
    expect(output).toContain("const pathCache = new Map()");
    expect(output).toContain("new Path2D(data)");
    expect(output).toContain("source: vectorCanvas");
    expect(output).not.toContain('XMLSerializer');
    expect(output).not.toContain('createImageBitmap');
    expect(output).toContain('statistics.frames += 1');
    expect(output).toContain('probeExtendedScene');
    expect(output).toContain('extended: max > 1');
  });
});
