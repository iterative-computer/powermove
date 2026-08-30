import {
  buildDockDropTargets,
  clampPanelHeight,
  hitTestDockPlacement,
  resolveDropIndex,
  visibleDockPlan
} from '../../layout/geometry';
import type { PanelDefinition } from '../../kernel/api';
import { ensureKernel, registryView } from '../kernel-view';
import type { PMRegistry } from '../registry';

export { buildDockDropTargets, clampPanelHeight, hitTestDockPlacement, resolveDropIndex, visibleDockPlan };

export const LEGACY_OWNER = 'legacy';

/** Bootstrap the panel registry before the concrete Svelte layout is installed. */
export function install(PM: PMRegistry): void {
  const kernel = ensureKernel(PM);
  /* Panels temporarily removed because their extension was disabled are not a
     user layout choice. Remember only those removals so re-enabling the last
     provider restores the panel, while panels the user had already hidden stay
     hidden. */
  const suspended = new Set<string>();

  /* One live registration per id, so re-registering a panel (HMR, a built-in
     re-running its install) replaces instead of stacking under itself. */
  const owned = new Map<string, { dispose(): void }>();
  const put = (id: string, definition: Record<string, any>): void => {
    /* Register the replacement BEFORE disposing the old entry: dispose-first
       leaves a moment with no definition, and the onChange handler below reads
       that as "panel gone" and hides it via WS.mutate — which re-activates the
       workspace, which re-registers custom panels, which loops (a mutate storm
       that live-reset splitter drags). Registering first keeps the id resolvable
       at every point in the swap. */
    const previous = owned.get(id);
    owned.set(id, kernel.panels.register(LEGACY_OWNER, Object.assign({ id, title: id }, definition, { id }) as PanelDefinition));
    previous?.dispose();
  };

  PM.registerPanel = (id: string, definition: Record<string, any>): void => put(id, definition);
  PM.PANELS = registryView<PanelDefinition, PanelDefinition>(kernel.panels, {
    read: (item) => item,
    write: (id, value) => {
      if (!value || typeof value !== 'object') return false;
      put(id, value as Record<string, any>);
      return true;
    },
    remove: (id) => {
      owned.get(id)?.dispose();
      owned.delete(id);
      return true;
    }
  });
  PM.panelInst = {};

  /* An override (or the extension that owned a panel going away) has to reach
     the DOM: rebind the mounted instance to the new definition and re-run its
     build. When the last definition for a mounted panel disappears there is
     nothing to build, so hide it — the workspace keeps the spec, and the panel
     comes back if something registers the id again. */
  kernel.panels.onChange((change) => {
    const inst = PM.panelInst?.[change.id];
    const definition = kernel.panels.get(change.id);
    if (change.kind === 'add') {
      if (!definition) return;
      if (inst) {
        inst.def = definition;
        PM.Layout?.refresh?.(change.id);
      }
      if (!suspended.delete(change.id)) return;
      if (!PM.Layout?.restorePanel || !PM.WS?.mutate) return;
      PM.WS.mutate((workspace: any) => PM.Layout.restorePanel(workspace, change.id));
      return;
    }
    if (definition) {
      if (!inst?.el?.isConnected) return;
      inst.def = definition;
      PM.Layout?.refresh?.(change.id);
      return;
    }
    if (!PM.Layout?.hidePanel || !PM.WS?.mutate) return;
    PM.WS.mutate((workspace: any) => {
      if (PM.Layout.hidePanel(workspace, change.id)) suspended.add(change.id);
    });
  });

  PM.Layout = {
    buildDockDropTargets,
    clampPanelHeight,
    hitTestDockPlacement,
    resolveDropIndex,
    visibleDockPlan
  };
}
