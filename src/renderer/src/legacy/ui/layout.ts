import {
  buildDockDropTargets,
  clampPanelHeight,
  hitTestDockPlacement,
  resolveDropIndex,
  visibleDockPlan
} from '../../layout/geometry';
import type { PMRegistry } from '../registry';

export { buildDockDropTargets, clampPanelHeight, hitTestDockPlacement, resolveDropIndex, visibleDockPlan };

/** Bootstrap the panel registry before the concrete Svelte layout is installed. */
export function install(PM: PMRegistry): void {
  PM.PANELS = {};
  PM.panelInst = {};
  PM.registerPanel = (id: string, definition: Record<string, any>): void => {
    PM.PANELS[id] = Object.assign({ id, title: id }, definition);
  };
  PM.Layout = {
    buildDockDropTargets,
    clampPanelHeight,
    hitTestDockPlacement,
    resolveDropIndex,
    visibleDockPlan
  };
}
