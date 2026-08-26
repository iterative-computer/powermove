/* Application-engine bootstrap. The remaining imperative engines install into
   one PM registry before the Svelte chrome mounts and boots the project. */
import type { PMRegistry } from './registry';
import { installLegacyRuntime } from '../runtime/install-legacy';
import { installSveltePanels } from '../panels/install';
import { installShell } from '../shell/install';
import { installSvelteLayout } from '../layout/install';
import { installSvelteOverlays } from '../overlays/install';

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

// Reuse the registry when present, otherwise create it.
const PM: PMRegistry = (window.PM = window.PM || {});

// Engine dependency order — do not reorder.
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
  ['ui/library', installLibraryUi],
  ['ui/shortcuts', installShortcuts],
  ['core/workspace', installWorkspace],
  ['core/exporter', installExporter],
  ['assistant/harness', installHarness],
  ['core/scripting', installScripting],
  ['assistant/spatial', installSpatial],
  ['ui/projects', installProjectsUi],
  ['ui/toolbar', installToolbar],
  // Install the Svelte-owned chrome before the app boots and applies the
  // workspace layout.
  ['shell', installShell],
  ['layout/svelte', installSvelteLayout],
  ['overlays/svelte', installSvelteOverlays],
  ['runtime/bridge', installLegacyRuntime],
  ['panels/svelte', installSveltePanels],
  ['app', installApp]
];

for (const [name, install] of INSTALLS) {
  try {
    install(PM);
  } catch (error) {
    // Keep independent engine failures isolated while the registry boots.
    console.error(`[legacy] install failed: ${name}`, error);
  }
}
