import { EventEmitter } from 'node:events';
import { expect, it, vi } from 'vitest';
import type { BridgeEvent } from '../../main/engine/bridge';
import type { ChildLike } from '../../main/engine/ports';
import { NativeSnapshotBridge } from './snapshot-bridge';

function fixture() {
  const hosts: any[] = [];
  const addon = {
    serviceName: () => `launch-${hosts.length}`,
    BridgeHost: class {
      readonly name: string;
      callback?: (event: BridgeEvent) => void;
      expectChild = vi.fn((pid: number) => ({ pid, pidversion: 1, parentPid: 1 }));
      start = vi.fn((callback: (event: BridgeEvent) => void) => { this.callback = callback; });
      stop = vi.fn(); releaseAll = vi.fn();
      constructor(name: string) { this.name = name; hosts.push(this); }
    }
  };
  const events = new EventEmitter(), kill = vi.fn(() => true);
  const child = Object.assign(events, { pid: 42, kill, postMessage() {} }) as ChildLike & EventEmitter;
  const log = vi.fn();
  const bridge = new NativeSnapshotBridge(addon as any, log);
  return { bridge, hosts, child, log, kill };
}

it('enables document snapshots with a fresh authenticated bridge on each launch', () => {
  const f = fixture(), envs: Record<string, string>[] = [];
  const start = (env: Record<string, string>) => { envs.push(env); return f.child; };
  f.bridge.spawn(start); f.child.emit('spawn');
  expect(envs[0]).toEqual({ POWERMOVE_FRAME_BRIDGE: 'launch-0', POWERMOVE_ENGINE_FRAME_SOURCE: 'document', POWERMOVE_ENGINE_HOST_CLOCK: '1' });
  expect(f.hosts[0].expectChild).toHaveBeenCalledWith(42);
  expect(f.hosts[0].start).toHaveBeenCalledWith(expect.any(Function), 15_000);
  f.child.emit('exit', 1);
  f.bridge.spawn(start); f.child.emit('spawn');
  expect(envs[1]?.POWERMOVE_FRAME_BRIDGE).toBe('launch-1');
  expect(f.hosts[0].releaseAll).toHaveBeenCalledOnce();
  f.bridge.dispose();
  expect(f.hosts[1].releaseAll).toHaveBeenCalledOnce();
});

it('kills a failed bridge child so the engine supervisor restarts it', () => {
  const f = fixture(); f.bridge.spawn(() => f.child); f.child.emit('spawn');
  f.hosts[0].callback({ kind: 'error', reason: 'check-in timed out' });
  expect(f.kill).toHaveBeenCalledOnce();
  expect(f.log).toHaveBeenCalledWith('native-snapshot-bridge-failed: check-in timed out');
  f.child.emit('exit', 1); f.bridge.dispose();
  expect(f.hosts[0].releaseAll).toHaveBeenCalledOnce();
});

it('releases the rendezvous if the engine cannot be spawned', () => {
  const f = fixture();
  expect(() => f.bridge.spawn(() => { throw new Error('fork failed'); })).toThrow('fork failed');
  f.bridge.dispose();
  expect(f.hosts[0].stop).toHaveBeenCalledOnce();
  expect(f.hosts[0].releaseAll).toHaveBeenCalledOnce();
});

it('cleans up a normal bridge goodbye without reporting or killing a failed engine', () => {
  const f = fixture(); f.bridge.spawn(() => f.child); f.child.emit('spawn');
  f.hosts[0].callback({ kind: 'goodbye' });
  expect(f.kill).not.toHaveBeenCalled();
  expect(f.log).not.toHaveBeenCalled();
  f.child.emit('exit', 0); f.bridge.dispose();
  expect(f.hosts[0].releaseAll).toHaveBeenCalledOnce();
});
