/*
 * Phase 4 shim: host a Svelte panel inside the LEGACY dock layout.
 *
 * The legacy layout calls def.build(body, inst) with a live element and, for
 * `persist` panels, parks that element across layout applies instead of
 * rebuilding — so the component mounts exactly once and survives dock moves.
 * The DOM contract (.panel[data-panel], #panel-<id>, .ptitle) stays owned by
 * the legacy layout until Phase 5 replaces it with DockLayout + PanelPool.
 */
import { mount, unmount, type Component } from 'svelte';

export interface PanelProps {
  panelId: string;
  spec: Record<string, any>;
}

export interface SveltePanelDef {
  title: string;
  component: Component<PanelProps>;
  size?: number;
  min?: number;
  flush?: boolean;
  noscroll?: boolean;
  headless?: boolean;
  hideMoveHandle?: boolean;
  moveSlot?: string;
}

type LegacyPM = Record<string, any>;

const mounted = new Map<string, { unmount(): void }>();

export function registerSveltePanel(PM: LegacyPM, id: string, def: SveltePanelDef): void {
  const { component, ...rest } = def;
  PM.registerPanel(id, {
    ...rest,
    persist: true,
    build(body: HTMLElement, inst: Record<string, any>) {
      mounted.get(id)?.unmount();
      const instance = mount(component, { target: body, props: { panelId: id, spec: inst.spec ?? {} } });
      mounted.set(id, { unmount: () => void unmount(instance) });
    }
  });
}

/** Test/HMR seam. */
export function unmountSveltePanels(): void {
  for (const [, m] of mounted) m.unmount();
  mounted.clear();
}
