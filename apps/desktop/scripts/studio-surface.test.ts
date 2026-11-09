import { createRequire } from 'node:module';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const desktop = path.resolve(__dirname, '..');
describe.skipIf(process.platform !== 'darwin')('native Studio surface authentication', () => {
  let directory: string, sender: string, addon: any;
  beforeAll(() => {
    directory = mkdtempSync(path.join(os.tmpdir(), 'pm-surface-test-'));
    sender = path.join(directory, 'sender');
    execFileSync('xcrun', ['clang++', '-std=c++17', '-fobjc-arc', '-I', path.resolve(desktop, '../../engine/crates/pm-studio/native'), '-framework', 'Foundation', '-framework', 'IOSurface', path.join(desktop, 'native/studio-surface-sender.mm'), '-o', sender]);
    // Exercise this checkout's source, rather than a possibly stale app build.
    const nodeInclude = [process.env.NODE_INCLUDE_DIR, path.resolve(path.dirname(process.execPath), '../include/node'), '/opt/homebrew/include/node'].find(directory => directory && existsSync(path.join(directory, 'node_api.h')));
    if (!nodeInclude) throw new Error('Node headers are needed for the native Studio surface test.');
    const addonPath = path.join(directory, 'studio-surface.node');
    execFileSync('xcrun', ['clang++', '-std=c++17', '-fobjc-arc', '-bundle', '-undefined', 'dynamic_lookup', '-I', nodeInclude, '-I', path.resolve(desktop, '../../engine/crates/pm-studio/native'), '-framework', 'Foundation', '-framework', 'IOSurface', '-lbsm', path.join(desktop, 'native/studio-surface.mm'), '-o', addonPath]);
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
  it('rejects a different Buffer view over an owned handle',()=>{
    const receiver=addon.create();
    try{
      const peer=spawnSync(sender,[receiver.service]);expect(peer.status).toBe(0);
      const surface=addon.take(receiver.port,42,peer.pid);
      expect(()=>addon.release(surface.handle.subarray(0))).toThrow('Unknown Studio surface handle');
      addon.release(surface.handle);addon.release(surface.handle);
    }finally{addon.close(receiver.port,receiver.service);}
  });
});
