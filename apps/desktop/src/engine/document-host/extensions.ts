/*
 * The user's extensions in a document host worker (docs/engine/document-host.md,
 * "Extensions"). The worker gives the kernel an extensions bridge over the
 * embedder's `extensions` service, so the kernel's own loader boots them as the
 * editor does: built-ins first, then the user's records through `planLoad`
 * (enabled state, `replaces` for forks and overrides of built-ins, `dependsOn`,
 * blocked health states, trust), and reloads on `extensionsChanged`.
 *
 * Bundles: the host serves each record's `bundleUrl` (`app://powermove/ext/…`);
 * the worker fetches the module source through the service once per bundle
 * hash and points the record at a file of its own, which the loader imports
 * (local extensions) or hands to the contained sandbox (Store extensions,
 * sandbox.ts).
 *
 * Health stays in the worker: the editor is the authority for what the Mods
 * panel shows, so a worker never reports an extension broken to the host; its
 * stats carry the worker's own health.
 */
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import type { ExtensionRecord, ExtensionsBridge, ExtensionsChangedEvent } from '../../shared/extensions';

export interface ExtensionServices {
  list(): Promise<ExtensionRecord[] | null>;
  bundle(url: string): Promise<string>;
  values(id: string): Promise<Record<string, string>>;
  log(line: string): void;
}

/** Records the embedder registered (`registerExtension`) win over the host's list by id. */
export class WorkerExtensions {
  private readonly dir = mkdtempSync(path.join(tmpdir(), 'powermove-document-host-'));
  private readonly files = new Map<string, string>();
  private readonly listeners = new Set<(event: ExtensionsChangedEvent) => void>();
  private readonly registered = new Map<string, ExtensionRecord>();
  /** Whether the host has an extensions service (decided on the first list). */
  available = false;

  constructor(private readonly services: ExtensionServices, private readonly builtinIds: readonly string[]) {}

  /** Asks the host once: with a service, the editor's bridge; without, the built-ins only. */
  async probe(): Promise<boolean> {
    this.available = (await this.services.list().catch(() => null)) !== null;
    return this.available;
  }

  /** Where a bundle's source lives in this worker (a file URL), fetched once per URL. */
  private async localize(record: ExtensionRecord): Promise<ExtensionRecord> {
    const url = record.bundleUrl;
    if (!url || url.startsWith('file:') || record.enabled === false) return record;
    let file = this.files.get(url);
    if (!file) {
      try {
        const source = await this.services.bundle(url);
        const name = `${(record.bundleHash ?? String(this.files.size)).replace(/[^\w.-]/g, '_')}.mjs`;
        const folder = path.join(this.dir, record.id.replace(/[^\w.-]/g, '_'));
        mkdirSync(folder, { recursive: true });
        file = path.join(folder, name);
        writeFileSync(file, source);
        this.files.set(url, file);
      } catch (error) {
        // The loader records `activation-error` ("no bundle to load") for it, as for a missing build.
        this.services.log(`[engine] extension ${record.id}: its bundle could not be read from the host (${(error as Error)?.message ?? error})`);
        return { ...record, bundleUrl: null };
      }
    }
    return { ...record, bundleUrl: pathToFileURL(file).href };
  }

  /** The bundle file a record points at in this worker, or null. */
  static fileOf(record: ExtensionRecord): string | null {
    return record.bundleUrl?.startsWith('file:') ? fileURLToPath(record.bundleUrl.split('?')[0]!) : null;
  }

  private async list(): Promise<ExtensionRecord[]> {
    const host = (await this.services.list()) ?? [];
    const byId = new Map(host.map((record) => [record.id, record]));
    for (const [id, record] of this.registered) byId.set(id, record);
    // A host list always carries the built-ins (main's registry does); keep them if one does not.
    for (const id of this.builtinIds) {
      if (!byId.has(id)) byId.set(id, { id, scope: 'builtin', manifest: { id, name: id, version: '1.0.0', apiVersion: 1 } as never, dir: `builtin:${id}`, enabled: true, bundleUrl: null, bundleHash: null, health: { state: 'ok' }, updatedAt: 0 });
    }
    return Promise.all([...byId.values()].map((record) => this.localize(record)));
  }

  /** The editor's extensions bridge, for the kernel (`window.powermove.extensions`). */
  bridge(): ExtensionsBridge {
    const refused = (what: string) => () => Promise.reject(new Error(`${what} is done in the editor, not in the document host.`));
    return {
      list: () => this.list(),
      setEnabled: refused('Turning extensions on and off'),
      remove: refused('Removing extensions'),
      reload: async ({ id }) => { this.emit({ ids: [id], reason: 'reload' }); return this.list(); },
      create: refused('Creating extensions'),
      reveal: refused('Showing extensions'),
      readSource: refused('Reading extension sources'),
      reportHealth: ({ id, health }) => {
        if (health.state !== 'ok') this.services.log(`[engine] extension ${id}: ${health.state}${'error' in health && health.error ? ` (${String(health.error)})` : ''}`);
      },
      onChanged: (listener) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; },
    };
  }

  vars(): { values(request: { id: string }): Promise<Record<string, string>> } {
    return { values: ({ id }) => this.services.values(id) };
  }

  emit(event: ExtensionsChangedEvent): void {
    for (const listener of this.listeners) listener(event);
  }

  /** Adds or replaces an embedder-registered record; the loader picks it up on the change event. */
  register(record: ExtensionRecord): void {
    // A module URL is imported once per process: a re-registered file gets a
    // fresh URL, so a hot reload runs the new code.
    const generation = (this.generations.get(record.id) ?? 0) + 1;
    this.generations.set(record.id, generation);
    const url = record.bundleUrl?.startsWith('file:') ? `${record.bundleUrl.split('?')[0]}?g=${generation}` : record.bundleUrl;
    this.registered.set(record.id, { ...record, bundleUrl: url });
    for (const [key, file] of this.files) if (key === record.bundleUrl) { this.files.delete(key); rmSync(file, { force: true }); }
  }
  private readonly generations = new Map<string, number>();

  dispose(): void {
    rmSync(this.dir, { recursive: true, force: true });
  }
}
