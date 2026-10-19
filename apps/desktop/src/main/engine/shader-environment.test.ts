import { expect, it } from 'vitest';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { persistShaderEnvironment, readShaderEnvironment, shaderCacheDirectory } from './shader-environment';
it('shares the exact desktop probe namespace and fails closed on a corrupt store', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'pm-probe-env-'));
  try {
    expect(readShaderEnvironment(dir)).toContain('native-without-desktop-probes');
    const descriptor = '{"engine":123,"chrome":"test","os":"test"}';
    persistShaderEnvironment(dir, descriptor); expect(readShaderEnvironment(dir)).toBe(descriptor);
    writeFileSync(path.join(dir, 'desktop-environment.json'), '{broken');
    expect(() => readShaderEnvironment(dir)).toThrow('shader-probe-store-unreadable');
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

it('reads the machine desktop cache unless this profile or the caller supplies one', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'pm-probe-location-'));
  try {
    const profile = path.join(dir, 'hidden'), desktop = path.join(dir, 'desktop');
    const localCache = path.join(profile, 'Engine Cache', 'Shader Cache'), desktopCache = path.join(desktop, 'Engine Cache', 'Shader Cache');
    expect(shaderCacheDirectory(profile, {}, desktop)).toBe(localCache);
    persistShaderEnvironment(desktopCache, 'desktop-probe');
    expect(shaderCacheDirectory(profile, {}, desktop)).toBe(desktopCache);
    persistShaderEnvironment(localCache, 'isolated-probe');
    expect(shaderCacheDirectory(profile, {}, desktop)).toBe(localCache);
    expect(shaderCacheDirectory(profile, { POWERMOVE_ENGINE_SHADER_CACHE: '/explicit/cache' }, desktop)).toBe('/explicit/cache');
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
