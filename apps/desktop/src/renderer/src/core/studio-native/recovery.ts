import type { StudioBackend, StudioBackendContext, StudioBackendFactory, StudioCameraDescription, StudioCameraOptions, StudioRenderRequest, StudioSceneDescription, StudioScenePatch, StudioTextureSource } from '../scene3d/studio-backend';
import { patchStudioScene } from '../scene3d/studio-backend';
import type { StudioScene } from '../studio/extract';
import type { StudioView } from '../studio/gpu';

/** A process failure switches the current job to the portable backend. Scene
 * replay happens before its first frame, so a crash never leaves an empty view. */
export class RecoveringStudioBackend implements StudioBackend {
  private fallback: StudioBackend | null = null;
  private switching: Promise<StudioBackend> | null = null;
  private input: { scene: StudioScene; maps: StudioTextureSource[]; description: () => StudioSceneDescription } | { document: StudioSceneDescription } | null = null;
  private view: StudioView | null = null;
  private time = 0;
  private camera: [StudioCameraDescription, StudioCameraOptions] | null = null;
  private closed = false;
  get id(): string { return this.fallback?.id ?? this.native.id; }
  constructor(private native: StudioBackend, private fallbackFactory: StudioBackendFactory, private context: StudioBackendContext) {}
  async initialize(): Promise<void> { try { await this.native.initialize(); } catch { await this.recover(); } }
  setPreparedScene(scene: StudioScene, maps: StudioTextureSource[], description: () => StudioSceneDescription) {
    this.input = { scene, maps, description }; this.camera = null;
    const active = this.fallback ?? this.native;
    return active.setPreparedScene ? active.setPreparedScene(scene, maps, description) : active.setScene(description());
  }
  setScene(document: StudioSceneDescription) { this.input = { document }; this.camera = null; return (this.fallback ?? this.native).setScene(document); }
  patchScene(patch: StudioScenePatch) {
    const current = this.input && ('document' in this.input ? this.input.document : this.input.description());
    if (!current) throw new Error('Set a Studio scene before patching it');
    return this.setScene(patchStudioScene(current, patch));
  }
  setView(view: StudioView) { this.view = view; return (this.fallback ?? this.native).setView?.(view); }
  setTime(seconds: number) { this.time = seconds; return (this.fallback ?? this.native).setTime(seconds); }
  setCamera(camera: StudioCameraDescription, options: StudioCameraOptions) { this.camera = [camera, options]; return (this.fallback ?? this.native).setCamera(camera, options); }
  async render(request: StudioRenderRequest) {
    if (this.closed) throw new Error('Studio backend is closed');
    if (request.signal?.aborted) throw new Error('Studio frame cancelled');
    try { return await (this.fallback ?? this.native).render(request); }
    catch (error) {
      if (request.signal?.aborted || this.fallback) throw error;
      return (await this.recover()).render(request);
    }
  }
  async pick(x: number, y: number) { return await (this.fallback ?? this.native).pick?.(x, y) ?? null; }
  async bounds(layer?: string) { return await (this.fallback ?? this.native).bounds?.(layer) ?? null; }
  private recover(): Promise<StudioBackend> {
    this.switching ??= (async () => {
      if (this.closed) throw new Error('Studio backend is closed');
      this.native.dispose();
      const fallback = this.fallbackFactory(this.context);
      try {
        await fallback.initialize();
        if (this.closed) throw new Error('Studio backend is closed');
        const input = this.input;
        if (input && 'document' in input) await fallback.setScene(input.document);
        else if (input) {
          if (fallback.setPreparedScene) await fallback.setPreparedScene(input.scene, input.maps, input.description);
          else await fallback.setScene(input.description());
        }
        if (this.view) await fallback.setView?.(this.view);
        if (this.camera) await fallback.setCamera(...this.camera);
        await fallback.setTime(this.time);
        this.fallback = fallback; this.context.invalidate(); return fallback;
      } catch (error) { fallback.dispose(); throw error; }
    })();
    return this.switching;
  }
  dispose(): void { this.closed = true; this.native.dispose(); this.fallback?.dispose(); this.input = null; }
}
