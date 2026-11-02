import type { Registry } from "./registry";

/** Mounts actual contribution builds in the V8 DOM mirror. The rendering host
 * still owns placement and paint acknowledgements; opening is not a paint ack.
 */
export class CompatPanels {
  private openPanels = new Map<string, { root: any; cleanup?: () => void }>();
  private hidden = new Set<string>();
  private subscription: { dispose(): void };
  constructor(
    private registry: Registry<any>,
    private document: any,
  ) {
    this.subscription = registry.onChange(({ id }) => {
      if (this.openPanels.has(id)) this.refresh(id);
    });
  }
  list = () => this.registry.ids();
  isOpen = (id: string) => this.openPanels.has(id);
  isHidden = (id: string) => this.hidden.has(id);
  open = (id: string, _dock?: unknown) => {
    this.hidden.delete(id);
    if (this.openPanels.has(id)) return;
    const definition = this.registry.get(id);
    if (!definition) return;
    const root = this.document.createElement("section");
    root.setAttribute("data-pm-contribution", id);
    this.document.body.append(root);
    const entry: { root: any; cleanup?: () => void } = { root };
    this.openPanels.set(id, entry);
    try {
      const cleanup = definition.build(root, { spec: { id, panel: id } });
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
  close = (id: string) => {
    this.hidden.add(id);
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
  }
}
