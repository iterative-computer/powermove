/* Main's tracking of the editor renderers' font cache seeds (native engine
   plan, amendment 19; shared/engine-fonts-seeds.ts, docs/engine/text.md).

   Every window's renderer creates system-ui font cache entries for the UI
   text at the scale factor of each display the window has been on. Main
   watches its windows' displays, answers `engine-fonts:seeds` per window
   (pm-wasm in that window measures as its renderer does), tells a window
   when its seeds change, and sends every window's seeds to the engines'
   font services: their render threads draw each window's views, and its
   exports, with that window's seeds (the window's id comes with its engine
   ports, and with the job sessions main opens for it). */
import { app, BrowserWindow, screen, webContents, type App, type Screen, type WebContents } from 'electron';

import { engineSeedsJson, FontSeedTracker, seedsJson } from '../shared/engine-fonts-seeds';
import { ENGINE_FONTS_EVENTS } from '../shared/engine-fonts-ipc';

export { FontSeedTracker };

export interface SeedWatchDeps {
  app: Pick<App, 'on' | 'removeListener'>;
  screen: Pick<Screen, 'getDisplayMatching' | 'on' | 'removeListener'>;
  windows(): BrowserWindow[];
  /** A window's webContents by id (to tell it its seeds changed). */
  contents(id: number): Pick<WebContents, 'isDestroyed' | 'send'> | undefined;
}

/**
 * Tracks every window's displays in `tracker`. `onEngineSeeds` gets the
 * engines' seeds (`{ seeds, owners }` JSON, engineSeedsJson) now and whenever
 * they change. Returns the uninstall.
 */
export function watchWindowSeeds(tracker: FontSeedTracker, deps: SeedWatchDeps, onEngineSeeds: (json: string) => void): () => void {
  const watched = new WeakSet<BrowserWindow>();
  const updates = new Map<number, () => void>();
  const watch = (window: BrowserWindow): void => {
    if (watched.has(window) || window.isDestroyed()) return;
    watched.add(window);
    const id = window.webContents.id;
    const update = (): void => {
      if (window.isDestroyed()) return;
      tracker.observe(id, deps.screen.getDisplayMatching(window.getBounds()).scaleFactor);
    };
    updates.set(id, update);
    window.on('moved', update);
    window.on('resized', update);
    window.once('closed', () => {
      updates.delete(id);
      tracker.forget(id);
    });
    update();
  };
  const created = (_event: unknown, window: BrowserWindow): void => watch(window);
  const metrics = (): void => {
    for (const update of updates.values()) update();
  };
  const stop = tracker.onChange((owner) => {
    onEngineSeeds(engineSeedsJson(tracker.engineSeeds()));
    if (owner === null) return;
    const contents = deps.contents(owner);
    if (contents && !contents.isDestroyed()) contents.send(ENGINE_FONTS_EVENTS.seedsChanged, seedsJson(tracker.seedsFor(owner)));
  });
  deps.app.on('browser-window-created', created);
  deps.screen.on('display-metrics-changed', metrics);
  deps.screen.on('display-added', metrics);
  deps.screen.on('display-removed', metrics);
  for (const window of deps.windows()) watch(window);
  onEngineSeeds(engineSeedsJson(tracker.engineSeeds()));
  return () => {
    stop();
    deps.app.removeListener('browser-window-created', created);
    deps.screen.removeListener('display-metrics-changed', metrics);
    deps.screen.removeListener('display-added', metrics);
    deps.screen.removeListener('display-removed', metrics);
  };
}

/** Main's seeds for "Use New Engine": the windows of this app, the engines'
 *  seeds to `engines` (the font clients of both engines, through the
 *  interactive one's mirror), and each window's own seeds for its
 *  `engine-fonts:seeds` channel. */
export function startEngineFontSeeds(engines: { setSeeds(json: string): Promise<unknown> }, log: (level: 'warn', message: string) => void): { seedsFor(sender: number): string; stop(): void } {
  const tracker = new FontSeedTracker();
  const stop = watchWindowSeeds(tracker, {
    app,
    screen,
    windows: () => BrowserWindow.getAllWindows(),
    contents: (id) => webContents.fromId(id) ?? undefined
  }, (json) => {
    engines.setSeeds(json).catch((error: unknown) => log('warn', `[engine-fonts] seeds: ${error instanceof Error ? error.message : String(error)}`));
  });
  return { seedsFor: (sender) => seedsJson(tracker.seedsFor(sender)), stop };
}
