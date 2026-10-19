import { expect, it } from 'vitest';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { persistShaderEnvironment, readShaderEnvironment } from './shader-environment';
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
