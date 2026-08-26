import { flushSync, mount, unmount } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import DockLayout from './DockLayout.svelte';
import {
  buildDockDropTargets,
  clampPanelHeight,
  hitTestDockPlacement,
  resolveDropIndex,
  visibleDockPlan
} from './geometry';
import {
  addPanel,
  ensureDock,
  findPanel,
  hasPanel,
  hidePanel,
  movePanel,
  movePanelBy,
  removePanel,
  restorePanel,
  setPanelCollapsed,
  type Workspace
} from './model';

type LayoutInstance = ReturnType<typeof mount> & {
  apply(workspace: Workspace): void;
  commit(): void;
};

let mounted: LayoutInstance | null = null;

function applyTheme(PM: PMRegistry, theme: Record<string, any>): void {
  const root = document.documentElement.style;
  const map: Record<string, string> = {
    accent: '--accent',
    bg: '--bg-window',
    panel: '--bg-panel',
    text: '--tx',
    line: '--line',
    font: '--f-ui',
    mono: '--f-mono'
  };
  for (const key of ['accent', 'bg', 'panel', 'text', 'line', 'font', 'mono']) {
    if (theme[key]) root.setProperty(map[key]!, theme[key]);
    else root.removeProperty(map[key]!);
  }
  if (theme.accent) {
    /* Derive the accent family from the brand color instead of flattening it:
       text needs to pull toward the theme's ink for contrast, and the wash
       stays at the token's 12% so accent never reads hotter than designed. */
    root.setProperty('--accent-dim', `color-mix(in srgb, ${theme.accent} 12%, transparent)`);
    root.setProperty('--accent-tx', `color-mix(in oklab, ${theme.accent} 82%, var(--tx))`);
  } else {
    root.removeProperty('--accent-dim');
    root.removeProperty('--accent-tx');
  }
  if (theme.radius != null) {
    /* Scale the whole radius family from one base so nested corners stay
       concentric (outer = inner + padding) at any project radius. */
    root.setProperty('--r-base', `${Math.max(2, theme.radius) / 3}px`);
  } else {
    root.removeProperty('--r-base');
  }
}

export function installSvelteLayout(PM: PMRegistry): void {
  const root = document.getElementById('body');
  if (!root) return;

  if (mounted) void unmount(mounted);
  root.replaceChildren();

  const layout: Record<string, any> = {
    root,
    ws: null,
    buildDockDropTargets,
    clampPanelHeight,
    hitTestDockPlacement,
    resolveDropIndex,
    visibleDockPlan,
    removePanel,
    hidePanel,
    restorePanel,
    addPanel,
    movePanel,
    movePanelBy,
    ensureDock,
    hasPanel,
    findPanel
  };
  PM.Layout = layout;
  let applying = false;
  layout.apply = (workspace: Workspace): void => {
    // A synchronous layout listener may request another apply; the active pass owns this render.
    if (applying) return;
    applying = true;
    try {
      layout.ws = workspace;
      flushSync(() => mounted?.apply(workspace));
      applyTheme(PM, workspace.theme || {});
      document.documentElement.dataset.density = workspace.density || 'normal';
      PM.bus.emit('layout');
      window.requestAnimationFrame(() => {
        PM.bus.emit('layout:applied');
        window.requestAnimationFrame(() => {
          PM.bus.emit('layout:applied');
          PM.invalidate();
        });
      });
    } finally {
      applying = false;
    }
  };
  layout.refresh = (id: string): void => {
    const inst = PM.panelInst[id];
    if (!inst?.el || !inst.el.isConnected) return;
    inst.body.textContent = '';
    try {
      inst.def.build?.(inst.body, inst);
    } catch (error) {
      console.error(error);
    }
    inst.def.header?.(inst.header, inst);
  };
  layout.applyTheme = (theme: Record<string, any>): void => applyTheme(PM, theme);
  layout.setCollapsed = (id: string, collapsed: boolean, emit = true) => setPanelCollapsed(PM, id, collapsed, emit);

  mounted = mount(DockLayout, { target: root, props: { PM } }) as LayoutInstance;
  flushSync();
}

/** Test/HMR seam. Workspace state remains untouched. */
export async function unmountSvelteLayout(): Promise<void> {
  if (mounted) await unmount(mounted);
  mounted = null;
}
