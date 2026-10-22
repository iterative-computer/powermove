import { paintViewerCard, viewerCardSize, viewerCardTarget } from '../../../extensions/viewer/library-card';

export { paintViewerCard, viewerCardSize, viewerCardTarget };

/** Source canvases an engine-filled viewer card does not copy: the live #gl
 * and the native presentation canvas both hold engine pixels. */
export function engineViewerCanvas(id: string): boolean {
  return id === 'gl' || id === 'engine-present';
}

/** The cloned presentation canvas is stage-level: whatever its copy holds
 * would sit above the card's cover, so an engine-filled card hides it. */
export function hideViewerPresentation(clone: HTMLElement): void {
  for (const canvas of clone.querySelectorAll<HTMLCanvasElement>('canvas[data-library-source-id="engine-present"]')) {
    canvas.style.visibility = 'hidden';
  }
}

/** Who fills an engine-on viewer card. The live #gl's pixels belong to the
 * engine and do not copy. The current viewer's callback (`enginePreview`)
 * fills its own card; a viewer with no callback (a frozen fork) is filled by
 * the host; a fork that renders its card itself owns it, as at base. */
export function viewerCardFill(id: string, engine: boolean, library: { render?: unknown } | null | undefined): { skipGl: boolean; host: boolean } {
  const render = library?.render as { enginePreview?: boolean } | undefined;
  const engineViewer = id === 'viewer' && engine;
  const host = engineViewer && typeof render !== 'function';
  return { skipGl: host || (engineViewer && !!render?.enginePreview), host };
}

/** The legacy panel definition cannot be modified when it is a frozen fork.
 * The host supplies engine pixels to its cloned #gl while preserving its DOM,
 * at the live viewer's own render size. Only for a viewer with no
 * `library.render` of its own: a fork that draws its card owns it. The fill
 * outlives a detached clone, which the Library caches off-shelf. */
const previewTickets = new WeakMap<HTMLElement, number>();
export function fillFrozenViewerCard(PM: Record<string, any>, source: HTMLElement, clone: HTMLElement): void {
  const engine = PM.EngineRender;
  const project = PM.proj;
  const target = viewerCardTarget(clone);
  const size = target && project && viewerCardSize(target.inner, project, window.devicePixelRatio || 1,
    Number(PM.quality ?? 0), PM.previewResolution ?? null);
  if (!engine || !project || !size) return;
  const ticket = (previewTickets.get(clone) ?? 0) + 1;
  previewTickets.set(clone, ticket);
  const composition = project.compId;
  const alive = () => source.isConnected && PM.proj === project && project.compId === composition
    && PM.panelInst?.viewer?.el === source && PM.EngineRender === engine && previewTickets.get(clone) === ticket;
  const blocked = () => PM.playing || PM.interactionActive?.() || PM.Export?.busy;
  const retry = () => { if (alive()) window.setTimeout(capture, 200); };
  const capture = () => {
    if (!alive()) return;
    if (blocked()) { retry(); return; }
    const revision = engine.currentRevision();
    const time = PM.time;
    // The paused live viewer's frame: motion blur at 12 samples, no draft 3D.
    void engine.thumbnails.request({
      key: `library-viewer:${project.id}`, time, width: size.width, height: size.height,
      motionBlurSamples: 12, minRevision: revision ?? undefined,
    }).then((snapshot: any) => {
      if (!alive()) return;
      if (!snapshot || blocked() || PM.time !== time || engine.currentRevision() !== revision) { retry(); return; }
      const frame = engine.toCanvas(snapshot);
      if (alive() && !blocked() && PM.time === time && engine.currentRevision() === revision) paintViewerCard(clone, frame);
      else retry();
    }).catch((error: unknown) => console.warn('Viewer Library preview failed', error));
  };
  capture();
}
