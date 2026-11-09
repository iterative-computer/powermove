import * as THREE from 'three';
import { patchStudioScene, type StudioBackend, type StudioBounds, type StudioCameraDescription, type StudioCameraOptions, type StudioFrame, type StudioRenderRequest, type StudioSceneDescription, type StudioScenePatch, type StudioTextureSource } from '../scene3d/studio-backend';
import { type StudioScene } from './extract';
import { initStudioGpu, studioGpuFailure, StudioTarget, type StudioView } from './gpu';
import { compileSceneDescription, viewFromDescription } from './scene-description';

/** The page runs the same Studio WGSL as Rust; three.js supplies geometry/math only. */
export class WebGpuStudioBackend implements StudioBackend {
  readonly id = 'webgpu';
  private scene: StudioScene | null = null;
  private maps: StudioTextureSource[] = [];
  private view: StudioView | null = null;
  private description: (() => StudioSceneDescription) | null = null;
  private document: StudioSceneDescription | null = null;
  private graph: THREE.Scene | null = null;
  private time = 0;
  private revision = 0;
  private viewKey = '';
  private targets = new Map<string, { target: StudioTarget; revision: number; viewKey: string; time: number; resolved: number; denoise: boolean }>();
  async initialize(): Promise<void> { if (!await initStudioGpu()) throw new Error('Studio could not start WebGPU'); }
  setPreparedScene(scene: StudioScene, maps: StudioTextureSource[], description: () => StudioSceneDescription): void {
    this.scene = scene; this.maps = maps; this.description = description; this.document = null; this.revision++;
  }
  setScene(document: StudioSceneDescription): void {
    this.disposeGraph();
    const compiled = compileSceneDescription(document);
    this.graph = compiled.graph; this.scene = compiled.scene; this.maps = compiled.maps;
    this.document = document; this.description = () => document; this.revision++;
    this.setView(viewFromDescription(document, this.view?.camera.aspect ?? 1));
  }
  patchScene(patch: StudioScenePatch): void {
    const current = this.document ?? this.description?.();
    if (!current) throw new Error('Set a Studio scene before patching it');
    this.setScene(patchStudioScene(current, patch));
  }
  setTime(seconds: number): void { this.time = seconds; }
  setCamera(camera: StudioCameraDescription, options: StudioCameraOptions): void {
    const current = this.document ?? this.description?.();
    if (!current) throw new Error('Set a Studio scene before setting its camera');
    this.setView(viewFromDescription({ ...current, scene: { ...current.scene, camera }, camera_options: options }, this.view?.camera.aspect ?? 1));
  }
  setView(view: StudioView): void {
    this.view = view; this.viewKey = JSON.stringify(view);
  }
  render(request: StudioRenderRequest): StudioFrame {
    if(studioGpuFailure())throw new Error(studioGpuFailure()!);
    if (request.signal?.aborted) throw new Error('Studio frame cancelled');
    if (!this.scene || !this.view) throw new Error('Set a Studio scene and camera before rendering');
    const width = Math.max(2, Math.min(8192, Math.round(request.size.width))), height = Math.max(2, Math.min(8192, Math.round(request.size.height)));
    const key = `${request.tier}:${width}:${height}`;
    let entry = this.targets.get(key);
    if (!entry) {
      // A tier keeps just its current resolution; resizing must not retain GPU targets.
      for (const [oldKey, old] of this.targets) if (oldKey.startsWith(`${request.tier}:`)) { old.target.destroy(); this.targets.delete(oldKey); }
      const target = new StudioTarget(width, height); target.live = request.tier === 'live';
      entry = { target, revision: -1, viewKey: '', time: NaN, resolved: -1, denoise: false }; this.targets.set(key, entry);
    }
    const t = entry.target;
    if (entry.revision !== this.revision) { t.setScene(this.scene, this.maps); entry.revision = this.revision; entry.viewKey = ''; entry.resolved = -1; }
    const changed = entry.viewKey !== this.viewKey || entry.time !== this.time;
    if (changed) { t.setView({ ...this.view, camera: { ...this.view.camera, aspect: width / height } }); entry.viewKey = this.viewKey; entry.time = this.time; entry.resolved = -1; }
    const goal = Math.max(1, Math.min(4096, Math.ceil(request.samples)));
    if (changed || t.samples < goal) t.trace(Math.max(0, goal - t.samples), request.tier === 'live' ? 2 : 8, changed, request.tier === 'live' ? 6 : 64, Math.round(this.time * 9600));
    const denoise = request.denoise !== false;
    if (entry.resolved !== t.samples || entry.denoise !== denoise) { t.resolve(request.tier === 'live' ? 4 : 0, request.tier === 'final' && denoise); entry.resolved = t.samples; entry.denoise = denoise; }
    const found = request.layer ? this.scene.objects.indexOf(request.layer) : -1;
    const index = request.layer && found < 0 ? this.scene.objects.length : found;
    return { source: t.present(`${request.layer ?? '*'}:${request.background === true}`, index, request.background === true), width, height, samples: t.samples, version: `${this.revision}:${this.time}:${this.viewKey}:${key}:${t.samples}` };
  }
  bounds(layer?: string): StudioBounds | null {
    if (!layer && this.scene && !this.scene.bounds.isEmpty()) return { min: {x:this.scene.bounds.min.x,y:this.scene.bounds.min.y,z:this.scene.bounds.min.z}, max: {x:this.scene.bounds.max.x,y:this.scene.bounds.max.y,z:this.scene.bounds.max.z} };
    const doc = this.document ?? this.description?.(); if (!doc) return null;
    const box = new THREE.Box3();
    doc.scene.objects.forEach((o, i) => { if (!layer || doc.object_ids[i]?.layer === layer) for (const p of o.mesh.positions) box.expandByPoint(new THREE.Vector3(p.x, p.y, p.z)); });
    return box.isEmpty() ? null : { min: { x: box.min.x, y: box.min.y, z: box.min.z }, max: { x: box.max.x, y: box.max.y, z: box.max.z } };
  }
  async liveHistory(){for(const [key,entry] of this.targets)if(key.startsWith('live:'))return entry.target.historyStats();return null;}
  private disposeGraph(): void {
    this.graph?.traverse((o: any) => { o.geometry?.dispose(); for (const m of Array.isArray(o.material) ? o.material : [o.material]) m?.dispose(); }); this.graph = null;
  }
  dispose(): void { for (const { target } of this.targets.values()) target.destroy(); this.targets.clear(); this.disposeGraph(); this.scene = null; this.maps = []; this.description = null; this.document = null; }
}
