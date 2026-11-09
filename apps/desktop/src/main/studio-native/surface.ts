/** macOS IOSurface import. All OS handles stay in main; the renderer receives
 * Electron's managed shared texture and its VideoFrame capability. */
import { sharedTexture, type WebContents } from 'electron';
import { createRequire } from 'node:module';
import path from 'node:path';
import { existsSync } from 'node:fs';
import type { StudioNativeReply } from '../../shared/studio-native';

type Addon = {
  create(): { port: number; service: string };
  take(port: number, frame: number, expectedPid: number): { handle: Buffer; width: number; height: number } | null;
  release(handle: Buffer): void;
  close(port: number, service: string): void;
};
export class StudioSurfaceReceiver {
  private closed = false;
  private receiver: { port: number; service: string };
  get service(): string { return this.receiver.service; }
  private constructor(private readonly addon: Addon) { this.receiver = addon.create(); }
  static open(executable: string | null): StudioSurfaceReceiver | null {
    if (process.platform !== 'darwin' || !executable || typeof sharedTexture?.importSharedTexture !== 'function' || process.env.STUDIO_NO_SHARED_TEXTURE) return null;
    const filename = path.join(path.dirname(executable), 'studio-surface.node');
    if (!existsSync(filename)) return null;
    try { return new StudioSurfaceReceiver(createRequire(__filename)(filename) as Addon); }
    catch { return null; } // The binary/canvas transport remains available.
  }
  async present(contents: WebContents, session: string, reply: StudioNativeReply, expectedPid: number | null, released: () => void): Promise<void> {
    if (this.closed || contents.isDestroyed()) { released(); throw new Error('Studio surface receiver is closed'); }
    if (!expectedPid) { released(); throw new Error('Studio surface process is unavailable'); }
    const meta = reply.result;
    let surface: ReturnType<Addon['take']>;
    try { surface = this.addon.take(this.receiver.port, meta.surface, expectedPid); }
    catch (error) { released(); throw error; }
    if (!surface || surface.width !== meta.width || surface.height !== meta.height) {if(surface)this.addon.release(surface.handle);released();throw new Error('Studio surface frame is missing');}
    let finished = false;
    const release = () => {
      if (finished) return;
      finished = true;
      try { this.addon.release(surface.handle); } finally { released(); }
    };
    let imported: Electron.SharedTextureImported | null = null;
    try {
      imported = sharedTexture.importSharedTexture({ textureInfo: { pixelFormat: 'rgba', codedSize: { width: surface.width, height: surface.height }, colorSpace: { primaries: 'bt709', transfer: 'srgb', matrix: 'rgb', range: 'full' }, timestamp: Math.round(meta.time * 1_000_000), handle: { ioSurface: surface.handle } }, allReferencesReleased: release });
      await sharedTexture.sendSharedTexture({ frame: contents.mainFrame, importedSharedTexture: imported }, { session, frame: meta.surface, width: meta.width, height: meta.height });
      meta.transport = 'sharedTexture';
    } finally {
      if (imported) imported.release(); else release();
    }
  }
  close(): void { if (this.closed) return; this.closed = true; this.addon.close(this.receiver.port, this.receiver.service); }
}
