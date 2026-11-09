import type { StudioNativeBridge, StudioNativeCommand, StudioNativeReply } from '../../../../shared/studio-native';
import { patchStudioScene, type StudioBackend, type StudioBounds, type StudioCameraDescription, type StudioCameraOptions, type StudioFrame, type StudioPick, type StudioRenderRequest, type StudioSceneDescription, type StudioScenePatch, type StudioTextureSource } from '../scene3d/studio-backend';
import { MAP_SIZE, STUDIO_MATERIAL_FLOATS, type StudioScene } from '../studio/extract';
import type { StudioView } from '../studio/gpu';
import { compileSceneDescription, viewFromDescription } from '../studio/scene-description';
import { StudioBinaryWriter } from './buffers';

function geometryKey(scene:StudioScene):string {
  const text=[scene.geometryKey,scene.instanceCount,scene.lightCount,scene.positions.byteLength,scene.tris.byteLength,scene.blasNodes.byteLength,scene.tlasNodes.byteLength,scene.sdata.byteLength,scene.materials.byteLength,...scene.objects].join('|');
  let a=2166136261,b=0x9e3779b9;for(let i=0;i<text.length;i++){const c=text.charCodeAt(i);a=Math.imul(a^c,16777619);b=Math.imul(b^c,2246822519);}
  return `${a>>>0}:${b>>>0}`;
}
let nextSession = 1;
const vec = (v: number[]) => ({ x: v[0]!, y: v[1]!, z: v[2]! });

/** The native process owns tracing, acceleration structures and denoising. The
 * page owns only canvases and ordinary renderer data. Calls are serialized so a
 * view can never overtake its geometry upload. */
export class NativeStudioBackend implements StudioBackend {
  readonly id = 'native-studio';
  readonly session = `studio-${nextSession++}`;
  private prepared: StudioScene | null = null;
  private maps: StudioTextureSource[] = [];
  private description: (() => StudioSceneDescription) | null = null;
  private document: StudioSceneDescription | null = null;
  private view: StudioView | null = null;
  private time = 0;
  private revision = 0;
  private uploaded = -1;
  private sceneRevision=0;
  private uploadedSceneRevision=-1;
  private supportsViewPatch=false;
  private geometryKey: string | null = null;
  private uploadedMaps: StudioTextureSource[] = [];
  private mapPixels: Uint8Array[] = [];
  private mapsRevision=0;
  private uploadedMapsRevision=-1;
  private generation = 0;
  private closed = false;
  private graph: import('three').Scene | null = null;
  private textureFrames=new Map<number,HTMLCanvasElement>();
  private textureWaiters=new Map<number,{resolve:(source:HTMLCanvasElement)=>void;reject:(error:Error)=>void;timer:ReturnType<typeof setTimeout>}>();
  private textureUnsubscribe:(()=>void)|null=null;
  private lastTextureMs=0;
  private queue: Promise<unknown> = Promise.resolve();
  private width = 1;
  private height = 1;
  private rendered = '';
  /** Last measurements are available to the diagnostic harness, outside product UI. */
  lastFrame: { host: any; roundTripMs: number; uploadMs: number; pixelsMs: number } | null = null;
  constructor(private readonly bridge: StudioNativeBridge) {}

  async initialize(): Promise<void> {
    if(this.closed)throw new Error('Studio backend is closed');
    this.textureUnsubscribe=this.bridge.onTexture?.((info,texture)=>{
      if(info.session!==this.session)return;
      const start=performance.now(), frame=texture.getVideoFrame() as VideoFrame;
      const source=document.createElement('canvas');source.width=info.width;source.height=info.height;
      try {source.getContext('2d',{alpha:true})!.drawImage(frame,0,0);}finally{frame.close();}
      if(this.closed){source.width=0;source.height=0;return;}
      this.lastTextureMs=performance.now()-start;
      const waiter=this.textureWaiters.get(info.frame);
      if(waiter){this.textureWaiters.delete(info.frame);clearTimeout(waiter.timer);waiter.resolve(source);}else{this.textureFrames.set(info.frame,source);}
      for(const [key,old] of this.textureFrames){if(this.textureFrames.size<=3)break;old.width=0;old.height=0;this.textureFrames.delete(key);}
    })??null;
    const hello = await this.send({ op: 'hello' }); this.generation = hello.generation;this.supportsViewPatch=hello.result?.capabilities?.includes('preparedView')??false;
    if(hello.result?.layout?.materialBytes!==STUDIO_MATERIAL_FLOATS*4 || hello.result?.layout?.triangleBytes!==96)throw new Error('Native Studio needs a matching renderer build');
  }
  setPreparedScene(scene: StudioScene, maps: StudioTextureSource[], description: () => StudioSceneDescription): void {
    this.disposeGraph(); this.prepared = scene; this.maps = maps; this.description = description; this.document = null; this.revision++; this.sceneRevision++; this.mapsRevision++;
  }
  setScene(document: StudioSceneDescription): void {
    if (document.version !== 1) throw new Error('Unsupported Studio scene version');
    this.disposeGraph(); const compiled = compileSceneDescription(document);
    this.graph = compiled.graph; this.document = document; this.description = () => document;
    this.prepared = compiled.scene; this.maps = compiled.maps; this.geometryKey = null; this.revision++; this.sceneRevision++; this.mapsRevision++;
    this.setView(viewFromDescription(document, this.width / this.height));
  }
  patchScene(patch: StudioScenePatch): void {
    const current = this.document ?? this.description?.();
    if (!current) throw new Error('Set a Studio scene before patching it');
    this.setScene(patchStudioScene(current, patch));
  }
  setView(view: StudioView): void {
    if (JSON.stringify(this.view) !== JSON.stringify(view)) { this.view = view; this.revision++; }
  }
  setTime(seconds: number): void {
    if (!Number.isFinite(seconds)) throw new Error('Studio time must be finite');
    if (this.time !== seconds) { this.time = seconds; this.revision++; }
  }
  setCamera(camera: StudioCameraDescription, options: StudioCameraOptions): void {
    const document = this.document ?? this.description?.();
    if (!document) throw new Error('Set a Studio scene before its camera');
    this.setView(viewFromDescription({ ...document, scene: { ...document.scene, camera }, camera_options: options }, this.width / this.height));
  }
  private serial<T>(action: () => Promise<T>): Promise<T> {
    const result = this.queue.then(() => { if (this.closed) throw new Error('Studio backend is closed'); return action(); });
    this.queue = result.catch(() => {}); return result;
  }
  private async send(command: StudioNativeCommand, binary?: Uint8Array): Promise<StudioNativeReply> {
    // A cut can dispose the job while its upload/render is awaiting IPC.
    // Never let its continuation recreate the released native session.
    if(this.closed)throw new Error('Studio backend is closed');
    const reply = await this.bridge.request(this.session, command, binary);
    if(this.closed)throw new Error('Studio backend is closed');
    if (this.generation && reply.generation !== this.generation) {
      this.uploaded = -1; this.uploadedSceneRevision=-1;this.geometryKey = null; this.uploadedMaps = []; this.uploadedMapsRevision=-1;
      this.generation = reply.generation;
      throw new Error('Native Studio restarted; scene needs to be uploaded again');
    }
    this.generation = reply.generation; return reply;
  }
  private packedView() {
    if (!this.view) throw new Error('Set a Studio camera before rendering');
    const box = this.prepared?.bounds;
    const bounds = box && !box.isEmpty() ? [(box.min.x + box.max.x) / 2, (box.min.y + box.max.y) / 2, (box.min.z + box.max.z) / 2, Math.max(1e-3, box.min.distanceTo(box.max) / 2)] : [0, 0, 0, 1];
    return { ...this.view, camera: { ...this.view.camera, aspect: this.width / this.height }, bounds };
  }
  private async upload(): Promise<void> {
    if (this.uploaded === this.revision) return;
    const revision = this.revision;
    if (this.prepared) {
      const scene = this.prepared;
      const nativeKey=geometryKey(scene);
      if(this.supportsViewPatch && this.geometryKey===nativeKey && this.uploadedSceneRevision===this.sceneRevision){
        await this.send({op:'preparedView',key:nativeKey,view:this.packedView()});
      }else{
        const writer = new StudioBinaryWriter();
        const small = { key: nativeKey, sdata: writer.append(scene.sdata), materials: writer.append(scene.materials), view: this.packedView() };
        if (this.geometryKey === nativeKey) {
          await this.send({ op: 'preparedPatch', ...small, top: writer.append(scene.tlasNodes), instancePolicies:scene.instancePolicies }, writer.finish());
        } else {
          const nodes = new Uint8Array(scene.tlasNodes.byteLength + scene.blasNodes.byteLength);
          nodes.set(new Uint8Array(scene.tlasNodes)); nodes.set(new Uint8Array(scene.blasNodes), scene.tlasNodes.byteLength);
          await this.send({ op: 'prepared', ...small, positions: writer.append(scene.positions), tris: writer.append(scene.tris), nodes: writer.append(nodes), topNodes: scene.tlasNodes.byteLength / 64, instanceCount: scene.instanceCount, lightCount: scene.lightCount, objects: scene.objects.map(layer => ({ layer, part: '' })), instancePolicies: scene.instancePolicies }, writer.finish());
          this.geometryKey = nativeKey;
        }
        this.uploadedSceneRevision=this.sceneRevision;
      }
      if(this.mapsRevision!==this.uploadedMapsRevision){await this.uploadTextures(this.maps);this.uploadedMapsRevision=this.mapsRevision;}
    } else { throw new Error('Set a Studio scene before rendering'); }
    this.uploaded = revision;
  }
  private async uploadTextures(maps: StudioTextureSource[]): Promise<void> {
    if (!maps.length) { this.uploadedMaps = []; this.mapPixels=[]; return; }
    const pixels = (source: StudioTextureSource): Uint8Array => {
      if (source instanceof Uint8Array) return source;
      const context = source.getContext('2d') as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
      if (!context) throw new Error('Studio texture source has no 2D context');
      const data = context.getImageData(0, 0, MAP_SIZE, MAP_SIZE).data;
      return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    };
    if (this.uploadedMaps.length !== maps.length) {
      const writer = new StudioBinaryWriter(); this.mapPixels=maps.map(source=>pixels(source).slice()); this.mapPixels.forEach(pixels=>writer.append(pixels));
      await this.send({ op: 'textures', width: MAP_SIZE, height: MAP_SIZE, layers: maps.length }, writer.finish());
    } else {
      for (let layer = 0; layer < maps.length; layer++) {
        const source = maps[layer]!;
        // Compare contents: live layer canvases can change without changing identity.
        const bytes=pixels(source),before=this.mapPixels[layer];
        let equal=before?.length===bytes.length;
        if(equal){const a=new Uint32Array(bytes.buffer,bytes.byteOffset,bytes.byteLength/4),b=new Uint32Array(before!.buffer);for(let i=0;i<a.length;i++){if(a[i]!==b[i]){equal=false;break;}}}
        if(equal)continue;
        await this.send({ op: 'textures', width: MAP_SIZE, height: MAP_SIZE, layers: maps.length, layer }, bytes);
        this.mapPixels[layer]=bytes.slice();
      }
    }
    this.uploadedMaps = [...maps];
  }
  render(request: StudioRenderRequest): Promise<StudioFrame> {
    return this.serial(async () => {
      if (request.signal?.aborted) throw new Error('Studio frame cancelled');
      const width=Math.round(request.size.width),height=Math.round(request.size.height);
      if(width/height!==this.width/this.height)this.revision++;
      this.width = width; this.height = height;
      const start = performance.now(); await this.upload(); const uploadMs = performance.now() - start;
      const objects = this.prepared?.objects ?? (this.document ?? this.description?.())?.layer_ids ?? [];
      const index = request.layer === undefined ? undefined : objects.indexOf(request.layer);
      // World/fog layers own no triangles. A nonmatching mask keeps only the
      // optional backdrop, without treating their pixels as the whole scene.
      const layer = index === -1 ? objects.length : index;
      const renderKey=`${this.revision}:${this.time}:${this.width}:${this.height}:${request.tier}:${request.samples}:${request.denoise!==false}`;
      const reply = await this.send({ op: 'render', width: this.width, height: this.height, tier: request.tier, samples: request.samples, reset: request.accumulate === false && this.rendered!==renderKey, goal: true, denoise: request.denoise !== false, layer, background: request.background === true, sharedTexture: !!this.textureUnsubscribe, time: this.time });
      if (request.signal?.aborted) throw new Error('Studio frame cancelled');
      const meta = reply.result;
      const pixelsAt = performance.now();
      let source:HTMLCanvasElement;
      if(meta.transport==='sharedTexture' && typeof meta.surface==='number') {
        const frame=this.textureFrames.get(meta.surface);
        if(frame){source=frame;this.textureFrames.delete(meta.surface);}else{
          source=await new Promise<HTMLCanvasElement>((resolve,reject)=>{
            const timer=setTimeout(()=>{this.textureWaiters.delete(meta.surface);reject(new Error('Studio shared texture timed out'));},1000);
            this.textureWaiters.set(meta.surface,{resolve,reject,timer});
          });
        }
      } else {
        if (meta.format !== 'rgba8' || reply.binary.byteLength !== meta.width * meta.height * 4) throw new Error('Native Studio returned an invalid frame');
        // The host packs straight RGBA for WebCodecs/Canvas2D.
        const frame = new VideoFrame(reply.binary, { format: 'RGBA', codedWidth: meta.width, codedHeight: meta.height, timestamp: Math.round(this.time * 1_000_000), colorSpace: { primaries: 'bt709', transfer: 'iec61966-2-1', matrix: 'rgb', fullRange: true } });
        source = document.createElement('canvas'); source.width = this.width; source.height = this.height;
        source.getContext('2d', { alpha: true })!.drawImage(frame, 0, 0, source.width, source.height); frame.close();
      }
      this.lastFrame = { host: meta, roundTripMs: reply.roundTripMs, uploadMs, pixelsMs:meta.transport==='sharedTexture'?this.lastTextureMs:performance.now()-pixelsAt };
      this.rendered=renderKey;
      return { source, width: this.width, height: this.height, samples: meta.samples, version: `${this.revision}:${this.time}:${request.tier}:${meta.samples}:${request.layer ?? '*'}`, release: () => { source.width = 0; source.height = 0; } };
    });
  }
  pick(x: number, y: number): Promise<StudioPick | null> {
    return this.serial(async () => {
      await this.upload();
      const reply = await this.send({ op: 'pick', x: Math.min(this.width - 1, Math.max(0, x * this.width)), y: Math.min(this.height - 1, Math.max(0, y * this.height)), width: this.width, height: this.height });
      const p = reply.result;
      return p ? { layer: p.layer, part: p.part || null, point: vec(p.point), normal: vec(p.normal) } : null;
    });
  }
  bounds(layer?: string): Promise<StudioBounds | null> {
    return this.serial(async()=>{await this.upload();const reply=await this.send({op:'bounds',layer});return reply.result?{min:vec(reply.result.min),max:vec(reply.result.max)}:null;});
  }
  private disposeGraph(): void {
    this.graph?.traverse((o: any) => { o.geometry?.dispose(); for (const m of Array.isArray(o.material) ? o.material : [o.material]) m?.dispose(); });
    this.graph = null;
  }
  dispose(): void { if (this.closed) return; this.disposeGraph(); this.closed = true; this.textureUnsubscribe?.();this.textureUnsubscribe=null;for(const frame of this.textureFrames.values()){frame.width=0;frame.height=0;}this.textureFrames.clear();for(const waiter of this.textureWaiters.values()){clearTimeout(waiter.timer);waiter.reject(new Error('Studio backend is closed'));}this.textureWaiters.clear(); void this.bridge.release(this.session).catch(() => {}); }
}
