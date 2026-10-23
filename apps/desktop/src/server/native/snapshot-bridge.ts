import type { BridgeAddon, BridgeEvent } from '../../main/engine/bridge';
import { CHECK_IN_TIMEOUT_MS, FRAME_BRIDGE_ENV } from '../../main/engine/bridge';
import type { ChildLike } from '../../main/engine/ports';

/** Headless snapshots use the document presenter and its authenticated bridge;
 * they never allocate viewer pools or import Chromium textures. */
export class NativeSnapshotBridge {
  private readonly launches = new Set<() => void>();
  constructor(private readonly addon: Pick<BridgeAddon, 'BridgeHost' | 'serviceName'>,
    private readonly log: (line: string) => void) {}

  spawn(start: (env: Record<string, string>) => ChildLike): ChildLike {
    // The addon permits an unsigned development parent only when it really is
    // unsigned. A signed Node parent still requires the child's matching Team ID.
    const host = new this.addon.BridgeHost(this.addon.serviceName(), true);
    let closed = false;
    const close = () => {
      if (closed) return;
      closed = true; this.launches.delete(close); host.stop(); host.releaseAll();
    };
    this.launches.add(close);
    let child: ChildLike;
    try {
      child = start({ [FRAME_BRIDGE_ENV]: host.name, POWERMOVE_ENGINE_FRAME_SOURCE: 'document', POWERMOVE_ENGINE_HOST_CLOCK: '1' });
    } catch (error) { close(); throw error; }
    const fail = (reason: string) => {
      if (closed) return;
      this.log(`native-snapshot-bridge-failed: ${reason}`); close(); child.kill();
    };
    child.on('spawn', () => {
      if (closed) return;
      try {
        if (!child.pid) throw new Error('The interactive engine has no PID.');
        const expected = host.expectChild(child.pid);
        host.start((event: BridgeEvent) => {
          if (closed) return;
          if (event.kind === 'accepted') this.log(`[native] snapshot bridge authenticated pid ${expected.pid}`);
          else if (event.kind === 'goodbye') close();
          else if (event.kind === 'error' || event.kind === 'gone') fail(event.reason ?? event.kind);
          else if (event.kind === 'rejected') this.log(`[native] snapshot bridge rejected peer: ${event.reason}`);
          else if (event.kind === 'surface' || event.kind === 'remoteLayer') fail('A headless snapshot session attempted viewer presentation.');
        }, CHECK_IN_TIMEOUT_MS);
      } catch (error) { fail(String(error)); }
    });
    child.on('exit', close);
    return child;
  }

  dispose(): void { for (const close of [...this.launches]) close(); }
}
