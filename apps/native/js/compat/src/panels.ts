import type { Registry } from "./registry";

/** v1–3 inspector sections are DOM, and the Inspector is a native surface: they
 * dock together in one panel beside it and rebuild with the selection. */
export const SECTIONS_PANEL = "extension-properties";

export const panelStyle = (rect: [number, number, number, number] | null) => {
  if (!rect) return "display:none";
  const [x, y, w, h] = rect;
  return `position:fixed;left:${x}px;top:${y}px;width:${w}px;height:${h}px;overflow:auto;box-sizing:border-box`;
};

/** Mounts actual contribution builds in the V8 DOM mirror. The rendering host
 * still owns placement and paint acknowledgements; opening is not a paint ack.
 */
export class CompatPanels {
  private openPanels = new Map<string, { root: any; cleanup?: () => void }>();
  private hidden = new Set<string>();
  /** Open in the shell. The shell restores its panels before extensions have
   * activated, so a panel opens when its definition arrives. */
  private wanted = new Set<string>();
  /** The shell's last placement, which can arrive before the panel opens. */
  private rects = new Map<string, [number, number, number, number] | null>();
  private subscription: { dispose(): void };
  constructor(
    private registry: Registry<any>,
    private document: any,
    private sections?: Registry<any>,
    private layerIds: () => string[] = () => [],
  ) {
    this.subscription = registry.onChange(({ id }) => {
      if (this.openPanels.has(id)) this.refresh(id);
      else if (this.wanted.has(id)) this.open(id);
    });
    sections?.onChange(() => this.refresh(SECTIONS_PANEL));
  }
  /** Sections rebuild against the new selection, as the Inspector does. */
  selectionChanged = () => this.refresh(SECTIONS_PANEL);
  private buildSections(root: any): () => void {
    const cleanups: (() => void)[] = [];
    const context = { layerIds: this.layerIds() };
    for (const section of this.sections?.list() ?? []) {
      if (section.when && !section.when(context)) continue;
      const wrapper = this.document.createElement("section");
      const title = this.document.createElement("h3");
      title.textContent = section.title;
      const body = this.document.createElement("div");
      wrapper.append(title, body);
      root.append(wrapper);
      const cleanup = section.build(body, context);
      if (typeof cleanup === "function") cleanups.push(cleanup);
      else if (typeof cleanup?.dispose === "function")
        cleanups.push(() => cleanup.dispose());
    }
    return () => {
      for (const cleanup of cleanups) cleanup();
    };
  }
  list = () => this.registry.ids();
  isOpen = (id: string) => this.openPanels.has(id);
  isHidden = (id: string) => this.hidden.has(id);
  open = (id: string, _dock?: unknown) => {
    this.hidden.delete(id);
    this.wanted.add(id);
    if (this.openPanels.has(id)) return;
    const definition =
      id === SECTIONS_PANEL
        ? { build: (root: any) => this.buildSections(root) }
        : this.registry.get(id);
    if (!definition) return;
    const root = this.document.createElement("section");
    root.setAttribute("data-pm-contribution", id);
    // Unplaced, it neither paints nor takes pointers: the shell owns geometry.
    root.setAttribute("style", panelStyle(this.rects.get(id) ?? null));
    // The body is the extension's to style, as in the shipped app's panel body.
    const body = this.document.createElement("div");
    body.setAttribute(
      "style",
      "position:relative;width:100%;height:100%;box-sizing:border-box;overflow:auto",
    );
    // Connected first: the remote document mirrors what is in the tree.
    this.document.body.append(root);
    root.append(body);
    const entry: { root: any; cleanup?: () => void } = { root };
    this.openPanels.set(id, entry);
    try {
      const cleanup = definition.build(body, { spec: { id, panel: id } });
      if (typeof cleanup === "function") entry.cleanup = cleanup;
      else if (typeof cleanup?.dispose === "function")
        entry.cleanup = () => cleanup.dispose();
    } catch (error) {
      this.remove(id);
      throw error;
    }
  };
  private remove(id: string) {
    const entry = this.openPanels.get(id);
    if (!entry) return;
    this.openPanels.delete(id);
    try {
      entry.cleanup?.();
    } finally {
      entry.root.remove();
    }
  }
  /** Window points of the shell's panel body. The root is host-owned geometry;
   * `null` is a tab behind another, which neither paints nor takes pointers. */
  place = (id: string, rect: [number, number, number, number] | null) => {
    this.rects.set(id, rect);
    this.openPanels.get(id)?.root.setAttribute("style", panelStyle(rect));
  };
  close = (id: string) => {
    this.hidden.add(id);
    this.wanted.delete(id);
    this.rects.delete(id);
    this.remove(id);
  };
  refresh = (id: string) => {
    if (!this.openPanels.has(id)) return;
    this.remove(id);
    this.open(id);
  };
  dispose() {
    this.subscription.dispose();
    for (const id of this.openPanels.keys()) this.remove(id);
    this.hidden.clear();
    this.wanted.clear();
    this.rects.clear();
  }
}
