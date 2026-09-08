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

  it('uses original logo alpha only as the animated glow mask and keeps the desktop transparent', async () => {
    const animation = await readFile(path.join(root, 'animation.js'), 'utf8');
    const styles = await readFile(path.join(root, 'onboarding.css'), 'utf8');
    expect(animation).toContain("glow.keepOrig !== true");
    expect(animation).toContain("'o = coloredGlow * (1. - original.a);'");
    expect(animation).toContain('transparent: true');
    expect(animation).toContain('audio: true');
    expect(styles).toContain('background: transparent');
    expect(styles).toContain('width: min(100vw, calc(100vh * 16 / 9))');
  });
});
