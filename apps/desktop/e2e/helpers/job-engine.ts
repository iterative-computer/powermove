import type { ElectronApplication } from 'playwright';

/** Export/Blender fixtures deliberately request the otherwise absent job utility. */
export async function requestJobEngine(app: ElectronApplication): Promise<void> {
  await app.evaluate(async () => {
    const host = (globalThis as { __powermoveNativeEngine?: { engine: { ensureReady(process: 'job'): Promise<void> } | null } }).__powermoveNativeEngine?.engine;
    if (host) await host.ensureReady('job');
  });
}
