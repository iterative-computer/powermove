/* Ordering constraint: installs run when the classic bundle tag executes,
   preserving relative order with the remaining classic scripts. */
import type { PMRegistry } from './registry';
import { install as installEasing } from './core/easing';
import { install as installProjects } from './core/projects';

declare global {
  interface Window {
    PM?: PMRegistry;
  }
}

// Match js/core/util.js: reuse its registry when present, otherwise create it.
const PM = (window.PM = window.PM || {});

// Keep this list in index.html script order as modules are converted.
installEasing(PM);
installProjects(PM);
