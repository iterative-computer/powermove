import type { Scene3dMeta } from '../../../shared/engine-protocol';

type Frame = { seq: number; revision: number; scene3dMeta?: Scene3dMeta[] };

/** Apply only diagnostics of the frame actually shown. These are native
 * scene results, including decoded-map failures; no renderer scene is built.
 * Metadata patches have the compositor's shape and preserve extension-owned
 * fields (and its historical sceneError value after a successful redraw). */
export class EngineSceneMetadata {
  private session: unknown;
  private project: unknown;
  private lastSeq = -1;
  private lastRevision = -1;
  private errors = new WeakMap<object, string>();

  constructor(private readonly PM: any) {}

  apply(frame: Frame, session: unknown): void {
    if (!this.PM.EngineDoc?.enabled) return;
    if (this.project !== this.PM.proj) this.errors = new WeakMap();
    if (this.session !== session || this.project !== this.PM.proj) {
      this.session = session;
      this.project = this.PM.proj;
      this.lastSeq = this.lastRevision = -1;
    }
    if (frame.seq <= this.lastSeq || frame.revision < this.lastRevision) return;
    this.lastSeq = frame.seq;
    this.lastRevision = frame.revision;
    const ui = this.PM.UIState;
    if (!ui?.setShaderMeta) return;
    for (const report of frame.scene3dMeta ?? []) {
      const comp = this.composition(report.composition);
      if (!comp) continue;
      for (const id of report.layers) {
        const layer = comp.layers?.find((layer: any) => layer.id === id);
        if (!layer) continue;
        const definition = this.PM.layerDefinition?.(layer.d?.definition) ??
          this.PM.Kernel?.layerTypes?.get?.(layer.d?.definition) ?? null;
        if (report.error !== null) this.errors.set(layer, report.error);
        const previousError = this.errors.get(layer);
        ui.setShaderMeta(layer, report.error === null
          ? { definition, missing: false, sceneWarnings: report.warnings,
            // Generic native failure tracking clears its error on recovery.
            // Today's compositor keeps this historical sceneError string.
            ...(previousError !== undefined ? { sceneError: previousError } : {}) }
          : { definition, missing: true, sceneError: report.error });
      }
    }
  }

  private composition(id: string): any {
    const project = this.PM.proj;
    if (project?.compId === id || project?.id === id) return project;
    const direct = project?.comps?.[id];
    if (direct) return direct;
    return Object.values(project?.comps ?? {}).find((comp: any) => comp?.id === id || comp?.compId === id);
  }
}
