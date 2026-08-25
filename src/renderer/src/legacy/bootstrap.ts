/* Legacy application bootstrap.
   Every former <script> tag in index.html is now an install(PM) module; they
   run here synchronously, in the exact original script order, when the
   classic host/legacy-bundle.js tag executes. The preload bridge
   (window.powermove) already exists at that point. */
import type { PMRegistry } from './registry';

import { install as installDiag } from './core/diag';
import { install as installUtil } from './core/util';
import { install as installUiState } from './core/ui-state';
import { install as installElectronShim } from './host/electron-shim';
import { install as installFonts } from './core/fonts';
import { install as installEasing } from './core/easing';
import { install as installModel } from './core/model';
import { install as installSelection } from './core/selection';
import { install as installAnim } from './core/anim';
import { install as installHistory } from './core/history';
import { install as installLibrary } from './core/library';
import { install as installProjects } from './core/projects';
import { install as installEditing } from './core/editing';
import { install as installCapabilities } from './core/capabilities';
import { install as installMedia } from './core/media';
import { install as installAudio } from './core/audio';
import { install as installShaders } from './gl/shaders';
import { install as installRaster } from './gl/raster';
import { install as installCompositor } from './gl/compositor';
import { install as installEngine } from './core/engine';
import { install as installControls } from './ui/controls';
import { install as installLayout } from './ui/layout';
import { install as installViewer } from './ui/viewer';
import { install as installTimeline } from './ui/timeline';
import { install as installInspector } from './ui/inspector';
import { install as installPanels } from './ui/panels';
import { install as installLibraryUi } from './ui/library';
import { install as installShortcuts } from './ui/shortcuts';
import { install as installWorkspace } from './core/workspace';
import { install as installExporter } from './core/exporter';
import { install as installHarness } from './assistant/harness';
import { install as installScripting } from './core/scripting';
import { install as installSpatial } from './assistant/spatial';
import { install as installProjectsUi } from './ui/projects';
import { install as installToolbar } from './ui/toolbar';
import { install as installApp } from './app';

declare global {
  interface Window {
    PM?: PMRegistry;
  }
}

// Match js/core/util.js: reuse the registry when present, otherwise create it.
const PM: PMRegistry = (window.PM = window.PM || {});

// Original index.html script order — do not reorder.
const INSTALLS: Array<[string, (PM: PMRegistry) => void]> = [
  ['core/diag', installDiag],
  ['core/util', installUtil],
  ['core/ui-state', installUiState],
  ['host/electron-shim', installElectronShim],
  ['core/fonts', installFonts],
  ['core/easing', installEasing],
  ['core/model', installModel],
  ['core/selection', installSelection],
  ['core/anim', installAnim],
  ['core/history', installHistory],
  ['core/library', installLibrary],
  ['core/projects', installProjects],
  ['core/editing', installEditing],
  ['core/capabilities', installCapabilities],
  ['core/media', installMedia],
  ['core/audio', installAudio],
  ['gl/shaders', installShaders],
  ['gl/raster', installRaster],
  ['gl/compositor', installCompositor],
  ['core/engine', installEngine],
  ['ui/controls', installControls],
  ['ui/layout', installLayout],
  ['ui/viewer', installViewer],
  ['ui/timeline', installTimeline],
  ['ui/inspector', installInspector],
  ['ui/panels', installPanels],
  ['ui/library', installLibraryUi],
  ['ui/shortcuts', installShortcuts],
  ['core/workspace', installWorkspace],
  ['core/exporter', installExporter],
  ['assistant/harness', installHarness],
  ['core/scripting', installScripting],
  ['assistant/spatial', installSpatial],
  ['ui/projects', installProjectsUi],
  ['ui/toolbar', installToolbar],
  ['app', installApp]
];

for (const [name, install] of INSTALLS) {
  try {
    install(PM);
  } catch (error) {
    // A classic script tag would have logged and continued; do the same.
    console.error(`[legacy] install failed: ${name}`, error);
  }
}
