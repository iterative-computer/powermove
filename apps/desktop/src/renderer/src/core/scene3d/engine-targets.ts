/*
 * What transform targets (the gizmo, modal G/R/S, the snap and pivot
 * operators) read of the 3D scene: each object layer's `matrixWorld` and,
 * for a flat group's pivot, its members' boxes (core/scene3d/targets.ts).
 *
 * With the native evaluator on, pm-wasm answers both (`objectWorlds3d`,
 * `objectBoxes3d`) through the engine document's answer path; a fallback to
 * today's three.js runtime is counted there and refused by strict hosts.
 * Off, the composition's three.js runtime answers, as today.
 */
import * as THREE from 'three';
import { compositionRuntime } from './service';
import { layer3DRole } from './layers';
import { runtimeBoxes } from './navigation';
import type { SceneRuntime } from './runtime';

/** The part of a scene runtime transform targets use. */
export interface TargetScene {
  objects: { get(id: string): THREE.Object3D | null | undefined };
  /** `box.expandByObject(runtime.objects.get(id))`. */
  expandBy(box: THREE.Box3, id: string): void;
}

const comp = (PM: any) => PM.curComp?.() || PM.proj;

/** Today's answer: `runtime.objects.get(id).matrixWorld` per id. */
export function runtimeWorlds(PM: any, ids: readonly string[], time: number = PM.time): Record<string, number[] | null> {
  const runtime = compositionRuntime(PM, time), out: Record<string, number[] | null> = {};
  for (const id of ids) {
    const object = runtime.objects.get(id);
    if (!object) { out[id] = null; continue; }
    object.updateWorldMatrix(true, false);
    out[id] = Array.from(object.matrixWorld.elements);
  }
  return out;
}

class RuntimeTargets implements TargetScene {
  constructor(private runtime: SceneRuntime) {}
  get objects() { return this.runtime.objects; }
  expandBy(box: THREE.Box3, id: string): void {
    const node = this.runtime.objects.get(id);
    if (node) box.expandByObject(node);
  }
}

class EngineTargets implements TargetScene {
  private worlds: Record<string, number[] | null> | null = null;
  private boxes: Record<string, number[] | null> | null = null;
  private nodes = new Map<string, THREE.Object3D | null>();
  constructor(private PM: any, private engine: any, private time: number) {}
  private ids(): string[] {
    return comp(this.PM).layers.filter((l: any) => layer3DRole(l) === 'object').map((l: any) => l.id);
  }
  readonly objects = {
    get: (id: string): THREE.Object3D | null => {
      if (this.nodes.has(id)) return this.nodes.get(id)!;
      this.worlds ??= this.engine.objectWorlds3d(this.ids(), this.time, () => runtimeWorlds(this.PM, this.ids(), this.time));
      const world = this.worlds![id] ?? null;
      let node: THREE.Object3D | null = null;
      if (world) {
        // The engine's matrixWorld exactly, as the runtime's object holds it
        // (`matrixAutoUpdate` off, the matrix written directly).
        node = new THREE.Object3D();
        node.matrixAutoUpdate = false;
        node.matrix.fromArray(world);
        node.matrix.decompose(node.position, node.quaternion, node.scale);
        node.matrixWorld.copy(node.matrix);
      }
      this.nodes.set(id, node);
      return node;
    }
  };
  expandBy(box: THREE.Box3, id: string): void {
    this.boxes ??= this.engine.objectBoxes3d(this.ids(), this.time, () => runtimeBoxes(this.PM, this.ids(), this.time));
    const b = this.boxes![id];
    if (b) box.union(new THREE.Box3(new THREE.Vector3(b[0], b[1], b[2]), new THREE.Vector3(b[3], b[4], b[5])));
  }
}

/** The scene transform targets read at `time`: the engine's with the native evaluator on, else the three.js runtime. */
export function targetScene(PM: any, time: number = PM.time): TargetScene {
  const engine = PM?.EngineDoc?.enabled ? PM.EngineDoc.api : null;
  if (engine?.objectWorlds3d && engine?.objectBoxes3d) return new EngineTargets(PM, engine, time);
  return new RuntimeTargets(compositionRuntime(PM, time));
}

/** A runtime as a target scene (tests and callers holding a runtime). */
export const runtimeTargets = (runtime: SceneRuntime): TargetScene => new RuntimeTargets(runtime);
