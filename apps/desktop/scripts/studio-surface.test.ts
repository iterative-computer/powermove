import { createRequire } from 'node:module';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const desktop = path.resolve(__dirname, '..');
const addonPath = path.join(desktop, 'resources/studio/studio-surface.node');
describe.skipIf(process.platform !== 'darwin' || !existsSync(addonPath))('native Studio surface authentication', () => {
  let directory: string, sender: string, addon: any;
  beforeAll(() => {
    directory = mkdtempSync(path.join(os.tmpdir(), 'pm-surface-test-'));
    sender = path.join(directory, 'sender');
    execFileSync('xcrun', ['clang++', '-std=c++17', '-fobjc-arc', '-I', path.resolve(desktop, '../../engine/crates/pm-studio/native'), '-framework', 'Foundation', '-framework', 'IOSurface', path.join(desktop, 'native/studio-surface-sender.mm'), '-o', sender]);
    addon = createRequire(__filename)(addonPath);
  });
  afterAll(() => { if (directory) rmSync(directory, { recursive: true, force: true }); });
  it('rejects a correctly framed surface from an unexpected process', () => {
    const receiver = addon.create();
    try {
      expect(spawnSync(sender, [receiver.service]).status).toBe(0);
      expect(addon.take(receiver.port, 42, process.pid)).toBeNull();
    } finally { addon.close(receiver.port, receiver.service); }
  });
  it('accepts the expected process and releases its surface exactly once', () => {
    const receiver = addon.create();
    try {
      const peer = spawnSync(sender, [receiver.service]); expect(peer.status).toBe(0);
      const surface = addon.take(receiver.port, 42, peer.pid);
      expect(surface).toMatchObject({ width: 4, height: 4 });
      addon.release(surface.handle); addon.release(surface.handle);
      expect(addon.take(receiver.port, 42, peer.pid)).toBeNull();
    } finally { addon.close(receiver.port, receiver.service); }
  });
});
