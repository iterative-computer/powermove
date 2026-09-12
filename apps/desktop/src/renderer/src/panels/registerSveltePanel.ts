/* Adapter from the panel registry's build contract to a Svelte component.
   DockLayout's panel pool keeps each mounted component alive across moves. */
import { flushSync, mount, unmount, type Component } from 'svelte';
import { PANEL_ICONS } from './panel-icons';

export interface PanelProps {
  panelId: string;
  spec: Record<string, any>;
}

export interface SveltePanelDef {
  title: string;
  icon?: string;
  component: Component<PanelProps>;
  size?: number;
  min?: number;
  flush?: boolean;
  noscroll?: boolean;
  headless?: boolean;
  hideMoveHandle?: boolean;
  moveSlot?: string;
  /** Optional registry header hook retained for imperative panel consumers. */
  header?: (hdr: HTMLElement, inst: Record<string, any>) => void;
}

type LegacyPM = Record<string, any>;

const mounted = new Map<string, { unmount(): void }>();

export function registerSveltePanel(PM: LegacyPM, id: string, def: SveltePanelDef): void {
  const { component, ...rest } = def;
  PM.registerPanel(id, {
    icon: PANEL_ICONS[id],
    ...rest,
    persist: true,
    build(body: HTMLElement, inst: Record<string, any>) {
      mounted.get(id)?.unmount();
      const instance = mount(component, { target: body, props: { panelId: id, spec: inst.spec ?? {} } });
      /* Flush onMount effects so canvas attachment, GL initialization, and
         post-build move-slot injection are synchronous for registry callers. */
      flushSync();
      mounted.set(id, { unmount: () => void unmount(instance) });
    }
  });
}

/** Test/HMR seam. */
export function unmountSveltePanels(): void {
  for (const [, m] of mounted) m.unmount();
  mounted.clear();
}
