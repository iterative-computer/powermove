import type { PMRegistry } from '../registry';

import { install as installDiag } from '../core/diag';
import { install as installUtil } from '../core/util';
import { install as installUiState } from '../core/ui-state';
import { install as installElectronShim } from '../host/electron-shim';
import { install as installFonts } from '../core/fonts';
import { install as installEasing } from '../core/easing';
import { install as installModel } from '../core/model';
import { install as installSelection } from '../core/selection';
import { install as installAnim } from '../core/anim';
import { install as installHistory } from '../core/history';
import { install as installLibrary } from '../core/library';
import { install as installProjects } from '../core/projects';
import { install as installEditing } from '../core/editing';
import { install as installCapabilities } from '../core/capabilities';
import { install as installMedia } from '../core/media';
import { install as installAudio } from '../core/audio';
import { install as installShaders } from '../gl/shaders';
import { install as installRaster } from '../gl/raster';
import { install as installCompositor } from '../gl/compositor';
import { install as installEngine } from '../core/engine';
import { install as installControls } from '../ui/controls';
import { install as installLayout } from '../ui/layout';
import { install as installViewer } from '../ui/viewer';
import { install as installTimeline } from '../ui/timeline';
import { install as installLibraryUi } from '../ui/library';
import { install as installShortcuts } from '../ui/shortcuts';
import { install as installWorkspace } from '../core/workspace';
import { install as installExporter } from '../core/exporter';
import { install as installHarness } from '../assistant/harness';
import { install as installScripting } from '../core/scripting';
import { install as installSpatial } from '../assistant/spatial';
import { install as installProjectsUi } from '../ui/projects';
import { install as installToolbar } from '../ui/toolbar';
import { install as installApp } from '../app';

const INSTALLS = [
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
  ['app', installApp],
] as const;

export type LegacyInstallName = (typeof INSTALLS)[number][0];

/**
 * Build the same mutable PM registry used by bootstrap and run selected real
 * installers in bootstrap order. Tests provide Web API fakes before calling
 * this helper when a subsystem needs them.
 */
export function makePM(...installNames: LegacyInstallName[]): PMRegistry {
  let nextId = 0;
  const values = new Map<string, unknown>();
  const listeners = new Map<string, Set<(...args: any[]) => void>>();
  const PM: PMRegistry = {
    version: 1,
    uid: (prefix: string) => `${prefix}-${++nextId}`,
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    round: (value: number, places = 0) => Number(value.toFixed(places)),
    snapF: (time: number, fps: number) => Math.round(time * fps) / fps,
    bus: {
      on(name: string, listener: (...args: any[]) => void) {
        const group = listeners.get(name) || new Set();
        group.add(listener);
        listeners.set(name, group);
        return () => group.delete(listener);
      },
      off(name: string, listener: (...args: any[]) => void) {
        listeners.get(name)?.delete(listener);
      },
      emit(name: string, ...args: any[]) {
        for (const listener of listeners.get(name) || []) listener(...args);
      },
    },
    invalidate() {},
    toast() {},
    store: {
      get: (key: string, fallback: unknown) => values.has(key) ? values.get(key) : fallback,
      set: (key: string, value: unknown) => values.set(key, value),
      del: (key: string) => values.delete(key),
    },
    assets: { map: new Map(), get() { return null; } },
    FX: {},
    GL: { dropProgram() {} },
    SHADER_TEMPLATE: 'void main(){}',
  };

  const selected = new Set<LegacyInstallName>(installNames);
  if (selected.size !== installNames.length) {
    throw new Error('makePM received a duplicate installer name');
  }

  for (const [name, install] of INSTALLS) {
    if (selected.has(name)) install(PM);
  }

  return PM;
}
