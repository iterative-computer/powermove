/* Main's tracking of the editor renderers' font cache seeds (native engine
   plan, amendment 19; shared/engine-fonts-seeds.ts, docs/engine/text.md).

   Every window's renderer creates system-font cache entries for the UI text
   it lays out (its density, the panels and menus shown, page zoom, each
   display it has been on). The renderer reports what it laid out
   (`engine-fonts:ui-fonts`, renderer engine-doc/ui-font-census.ts); main
   resolves each family stack against the families Chromium sees (the
   registry's available family names), answers `engine-fonts:seeds` per
   window (pm-wasm in that window measures as its renderer does), tells a
   window when its seeds change, and sends every window's seeds to the
   engines' font services: their render threads draw each window's views,
   and its exports, with that window's seeds (the window's id comes with its
   engine ports, and with the job sessions main opens for it). */
import { app, webContents, type App, type BrowserWindow, type WebContents } from 'electron';

import { engineSeedsJson, FontSeedTracker, parseUiFontCensus, seedsJson } from '../shared/engine-fonts-seeds';
import { ENGINE_FONTS_EVENTS } from '../shared/engine-fonts-ipc';

export { FontSeedTracker };

export interface SeedWatchDeps {
  app: Pick<App, 'on' | 'removeListener'>;
  windows(): BrowserWindow[];
  /** A window's webContents by id (to tell it its seeds changed). */
  contents(id: number): Pick<WebContents, 'isDestroyed' | 'send'> | undefined;
}

/**
 * Forgets each window's seeds when it closes, and tells the windows and the
 * engines when seeds change. `onEngineSeeds` gets the engines' seeds
 * (`{ seeds, owners }` JSON, engineSeedsJson) now and whenever they change.
 * Returns the uninstall.
 */
export function watchWindowSeeds(tracker: FontSeedTracker, deps: SeedWatchDeps, onEngineSeeds: (json: string) => void): () => void {
  const watched = new WeakSet<BrowserWindow>();
  const watch = (window: BrowserWindow): void => {
    if (watched.has(window) || window.isDestroyed()) return;
    watched.add(window);
    const id = window.webContents.id;
    window.once('closed', () => tracker.forget(id));
    // A new renderer process starts with an empty font cache (a reload in
    // the same process keeps it).
    window.webContents.on('render-process-gone', () => tracker.forget(id));
  };
  const created = (_event: unknown, window: BrowserWindow): void => watch(window);
  const stop = tracker.onChange((owner) => {
    onEngineSeeds(engineSeedsJson(tracker.engineSeeds()));
    if (owner === null) return;
    const contents = deps.contents(owner);
    if (contents && !contents.isDestroyed()) contents.send(ENGINE_FONTS_EVENTS.seedsChanged, seedsJson(tracker.seedsFor(owner)));
  });
  deps.app.on('browser-window-created', created);
  for (const window of deps.windows()) watch(window);
  onEngineSeeds(engineSeedsJson(tracker.engineSeeds()));
  return () => {
    stop();
    deps.app.removeListener('browser-window-created', created);
  };
}

/** The font service main asks which families Chromium sees. */
export interface SeedFontSource {
  setSeeds(json: string): Promise<unknown>;
  familyNames(): Promise<string[]>;
  generation(): Promise<number>;
}

/**
 * Resolves renderers' censuses in order against the available families,
 * fetched again when the registry's generation moves (a font was installed
 * or removed). Electron-free so it is testable.
 */
export class CensusIntake {
  private generation: number | null = null;
  private chain: Promise<void> = Promise.resolve();

  constructor(private readonly tracker: FontSeedTracker, private readonly source: Pick<SeedFontSource, 'familyNames' | 'generation'>, private readonly log: (level: 'warn', message: string) => void) {}

  /** A renderer's census message; returns once it is applied. Malformed
   *  messages are refused. */
  report(sender: number, json: unknown): Promise<boolean> {
    const census = parseUiFontCensus(json);
    if (!census) return Promise.resolve(false);
    const done = this.chain.then(async () => {
      await this.refreshInstalled();
      this.tracker.report(sender, census);
    });
    this.chain = done.catch(() => undefined);
    return done.then(() => true);
  }

  private async refreshInstalled(): Promise<void> {
    try {
      const generation = await this.source.generation();
      if (generation === this.generation) return;
      const names = new Set((await this.source.familyNames()).filter((n) => !n.startsWith('.')).map((n) => n.toLowerCase()));
      this.generation = generation;
      this.tracker.setInstalled((family) => names.has(family));
    } catch (error) {
      // Without the list every named family counts as missing, as in a
      // renderer with no fonts installed beyond the system's.
      this.log('warn', `[engine-fonts] available families: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
}

/** Main's seeds for "Use New Engine": the windows of this app, the engines'
 *  seeds to `engines` (the font clients of both engines, through the
 *  interactive one's mirror), each window's own seeds for its
 *  `engine-fonts:seeds` channel, and its renderer's census reports. */
export function startEngineFontSeeds(engines: SeedFontSource, log: (level: 'warn', message: string) => void, windows: () => BrowserWindow[]): { seedsFor(sender: number): string; reportUiFonts(sender: number, json: unknown): Promise<boolean>; stop(): void } {
  const tracker = new FontSeedTracker();
  const intake = new CensusIntake(tracker, engines, log);
  const stop = watchWindowSeeds(tracker, {
    app,
    windows,
    contents: (id) => webContents.fromId(id) ?? undefined
  }, (json) => {
    engines.setSeeds(json).catch((error: unknown) => log('warn', `[engine-fonts] seeds: ${error instanceof Error ? error.message : String(error)}`));
  });
  return { seedsFor: (sender) => seedsJson(tracker.seedsFor(sender)), reportUiFonts: (sender, json) => intake.report(sender, json), stop };
}
