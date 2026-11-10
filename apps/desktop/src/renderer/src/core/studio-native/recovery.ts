import type { StudioBackend, StudioBackendContext, StudioBackendFactory, StudioCameraDescription, StudioCameraOptions, StudioRenderRequest, StudioSceneDescription, StudioScenePatch, StudioTextureSource, StudioShutterSample } from '../scene3d/studio-backend';
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
  private nativeFailure:string|null=null;
  get id(): string { return this.fallback?.id ?? this.native.id; }
  constructor(private native: StudioBackend, private fallbackFactory: StudioBackendFactory, private context: StudioBackendContext) {}
  async initialize(): Promise<void> { try { await this.native.initialize(); } catch (error) { this.nativeFailure=String(error);console.warn('Native Studio initialization failed; using WebGPU',error);await this.recover(); } }
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
  get shutterAccumulation():boolean {return (this.fallback ?? this.native).shutterAccumulation===true;}
  async renderShutter(samples:StudioShutterSample[],key:string) {
    if(!this.fallback && this.native.renderShutter){
      try{return await this.native.renderShutter(samples,key);}
      catch(error){if(this.closed || samples.some(s=>s.request.signal?.aborted))throw error;this.nativeFailure=String(error);await this.recover();}
    }
    const first=samples[0];if(!first)throw new Error('Studio shutter has no samples');
    const source=new OffscreenCanvas(first.request.size.width,first.request.size.height),ctx=source.getContext('2d')!;
    ctx.globalCompositeOperation='lighter';ctx.globalAlpha=1/samples.length;
    let total=0;
    for(const sample of samples){
      await this.setPreparedScene(sample.scene,sample.maps,()=>{throw new Error('Fallback shutter requires prepared scene support');});
      await this.setView(sample.view);await this.setTime(sample.time);
      const frame=await this.render({...sample.request,shutter:undefined});
      try{ctx.drawImage(frame.source,0,0);total+=frame.samples;}finally{frame.release?.();}
    }
    return {source,width:source.width,height:source.height,samples:total,version:`shutter-fallback:${key}`,release:()=>{source.width=source.height=0;}};
  }
  async render(request: StudioRenderRequest) {
    if (this.closed) throw new Error('Studio backend is closed');
    if (request.signal?.aborted) throw new Error('Studio frame cancelled');
    try { return await (this.fallback ?? this.native).render(request); }
    catch (error) {
      if (this.closed || request.signal?.aborted || this.fallback || (error as Error)?.name==='StudioFrameCancelled') throw error;
      this.nativeFailure=String(error);
      console.warn('Native Studio frame failed; using WebGPU',error);
      return (await this.recover()).render(request);
    }
  }
  async pick(x: number, y: number) { return await (this.fallback ?? this.native).pick?.(x, y) ?? null; }
  async bounds(layer?: string) { return await (this.fallback ?? this.native).bounds?.(layer) ?? null; }
  async liveHistory() { return await (this.fallback ?? this.native as any).liveHistory?.() ?? null; }
  diagnostics() { return {...((this.fallback ?? this.native).diagnostics?.() ?? {}),nativeFailure:this.nativeFailure}; }
  cancelRefine(){(this.fallback??this.native).cancelRefine?.();}
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
