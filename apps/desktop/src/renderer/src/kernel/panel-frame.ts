/*
 * The kernel's `frame` panel variant.
 *
 * A sandboxed (store) extension cannot hand the host a component or a `build`
 * function: its code lives in another document. Its panels are registered with
 * a kernel-owned `build` that docks a view iframe into the panel body instead
 * (sandbox-view.ts). The definition is marked so the layout and the Library can
 * treat it as such: host chrome only (no `header`, `moveSlot`, `headless`), and
 * no DOM-clone preview in the Library (a cloned iframe would load a second,
 * unconnected document), which shows the icon on the extension's art instead.
 */
export interface PanelFrame {
  /** Dock the view into `body`; the returned disposer tears it down and releases its ports. */
  mount(body: HTMLElement, inst: { spec: Record<string, unknown> }): { dispose(): void };
}

export interface FramePanelInfo {
  /** The sandboxed extension that owns the panel. */
  extensionId: string;
}

export const PANEL_FRAME: unique symbol = Symbol.for('powermove.panel-frame') as never;

export function panelFrameOf(def: unknown): FramePanelInfo | undefined {
  return def && typeof def === 'object' ? (def as { [PANEL_FRAME]?: FramePanelInfo })[PANEL_FRAME] : undefined;
}
