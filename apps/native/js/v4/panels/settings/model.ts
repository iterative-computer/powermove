import { HostClient } from "../../src/host-api";
import type { Json, EventDetail } from "../../src/protocol";
import {
  accountRows,
  generalRows,
  projectRows,
  sections,
  type Row,
  type Section,
  type ExtensionType,
} from "./catalogue";

export interface Extension {
  id: string;
  name: string;
  type: Exclude<ExtensionType, "All Types">;
  source: "Built in" | "Installed" | "AI-created" | "Custom";
  enabled: boolean;
  about: string;
  includes: string[];
  shortcuts: string[];
  files: string[];
  variables: Row[];
  // Opaque resource, never a filesystem path or web URL.
  icon?: string;
  canArchive: boolean;
}
export interface SpeechModel {
  id: string;
  name: string;
  size: string;
  state: "available" | "downloading" | "installed" | "error";
  progress?: number;
  error?: string;
}
export interface Publication {
  revision: number;
  project?: { id: string; name: string };
  multiWindow: boolean;
  rows?: Row[];
  extensions: readonly Extension[];
  extensionsState: "ready" | "loading" | "error" | "unavailable";
  speech: readonly SpeechModel[];
}
export type Action =
  | { type: "row"; id: string; action: string }
  | { type: "write"; id: string; value: Json }
  | { type: "extension.enabled"; id: string; enabled: boolean }
  | { type: "extension.archive"; id: string }
  | { type: "extensions.retry" }
  | { type: "extension.finder"; id: string }
  | { type: "store.open" }
  | {
      type: "speech";
      id: string;
      action: "download" | "cancel" | "remove" | "retry";
    };
/** Implemented by pm-services/host, not a second preferences or document store.
 * Resolves only after kernel/service acknowledgement; publishes the next snapshot.
 * Destructive actions must confirm in NSAlert (default Cancel), then archive.
 * Secrets never enter Publication/Row: secure input requires an opaque native lease.
 */
export interface SettingsServices {
  perform(action: Action, expectedRevision: number): Promise<Publication>;
}
export interface SettingsWindow {
  present(section: Section): void;
  closeAndRestoreFocus(): void;
  title(section: Section): void;
  scrollTo(section: Section, options: { reduceMotion: boolean }): void;
  focusSearch(): void;
  announce(message: string): void;
}
export interface Matches {
  rows: Set<string>;
  extensions: Set<string>;
  sections: Set<Section>;
  count: number;
}
const searchable = (row: Row) =>
  [row.label, row.detail, row.group, ...(row.options ?? []).map((o) => o.label)]
    .join(" ")
    .toLocaleLowerCase();
const emptyMatches = (): Matches => ({
  rows: new Set(),
  extensions: new Set(),
  sections: new Set(),
  count: 0,
});

/** This controller lives only on the V8 worker; all native effects are enqueued. */
export class SettingsModel {
  publication: Publication;
  active: Section;
  query = "";
  extensionQuery = "";
  extensionType: ExtensionType = "All Types";
  detailId?: string;
  open = false;
  reduceMotion = false;
  searching = false;
  matches = emptyMatches();
  range: [number, number] = [0, 0];
  anchorId?: string;
  readonly pending = new Set<string>();
  readonly failures = new Map<string, unknown>(); // inspection only; never rendered as product copy
  private profile = new Map<string, Json>();
  private listeners = new Set<() => void>();
  private epoch = 0;
  private searchEpoch = 0;
  private alive = true;
  private viewportFirst = 0;
  private viewportCount = 0;
  private filtered: readonly Extension[] = [];
  constructor(
    readonly host: HostClient,
    publication: Publication,
    readonly window: SettingsWindow,
    readonly services?: SettingsServices,
  ) {
    this.publication = publication;
    this.active = publication.project ? "project" : "general";
    this.reindex();
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private changed() {
    if (this.alive) for (const listener of this.listeners) listener();
  }
  get rows(): Row[] {
    const base = [
      ...(this.publication.project ? projectRows() : []),
      ...generalRows(this.publication.multiWindow),
      ...accountRows(),
    ];
    const overrides = new Map(
      (this.publication.rows ?? []).map((r) => [r.id, r]),
    );
    const rows = base.map((row) => overrides.get(row.id) ?? row);
    for (const row of this.publication.rows ?? [])
      if (!base.some((r) => r.id === row.id)) rows.push(row);
    return rows
      .filter((r) => r.section !== "project" || !!this.publication.project)
      .map((r) => ({
        ...r,
        ...(r.profileKey && this.profile.has(r.profileKey)
          ? { value: this.profile.get(r.profileKey) }
          : {}),
      }));
  }
  get availableSections() {
    return sections.filter(
      (s) => s !== "project" || !!this.publication.project,
    );
  }
  get visibleSections() {
    return this.availableSections.filter(
      (s) => !this.query.trim() || this.matches.sections.has(s),
    );
  }
  get extensions() {
    return this.filtered;
  }
  get detail() {
    return this.publication.extensions.find((e) => e.id === this.detailId);
  }
  get visibleExtensions() {
    return this.filtered.slice(...this.range);
  }
  async hydrate() {
    if (!this.alive) return;
    const epoch = this.epoch;
    await Promise.all(
      generalRows(this.publication.multiWindow)
        .filter((r) => r.profileKey)
        .map(async (row) => {
          const key = row.profileKey!;
          // Per-key pending guard prevents a late initial read overwriting a user edit.
          const initial = this.profile.get(key);
          try {
            const ack = await this.host.getSetting(key);
            if (
              !this.alive ||
              epoch !== this.epoch ||
              this.pending.has(row.id) ||
              this.profile.get(key) !== initial
            )
              return;
            if (ack.value !== undefined) this.profile.set(key, ack.value);
          } catch (error) {
            if (this.alive && epoch === this.epoch)
              this.failures.set(row.id, error);
          }
        }),
    );
    this.changed();
  }
  show(target?: Section) {
    if (!this.alive) return;
    this.open = true;
    this.query = "";
    this.reindex();
    this.active =
      target && this.availableSections.includes(target)
        ? target
        : this.publication.project
          ? "project"
          : "general";
    this.window.present(this.active);
    this.window.scrollTo(this.active, { reduceMotion: true });
    this.changed();
  }
  close() {
    if (!this.alive || !this.open) return;
    this.open = false;
    this.window.closeAndRestoreFocus();
    this.changed();
  }
  select(section: Section) {
    if (!this.alive || !this.visibleSections.includes(section)) return;
    this.active = section;
    this.window.title(section);
    this.window.scrollTo(section, { reduceMotion: this.reduceMotion });
    this.changed();
  }
  trackSection(section: Section) {
    if (!this.alive) return;
    if (!this.visibleSections.includes(section) || this.active === section)
      return;
    this.active = section;
    this.window.title(section);
    this.changed();
  }
  /** Called only by the source-list handler; typing arrows in fields remains native. */
  navigate(key: string) {
    const visible = this.visibleSections;
    const index = Math.max(0, visible.indexOf(this.active));
    const next =
      key === "Home"
        ? 0
        : key === "End"
          ? visible.length - 1
          : key === "ArrowUp"
            ? Math.max(0, index - 1)
            : key === "ArrowDown"
              ? Math.min(visible.length - 1, index + 1)
              : -1;
    if (next >= 0 && visible[next]) this.select(visible[next]);
  }
  key(event: EventDetail): boolean {
    if (!this.alive) return false;
    const command = event.metaKey || event.ctrlKey;
    const exact = command && !event.altKey && !event.shiftKey;
    if (exact && event.key === ",") {
      if (!event.repeat) this.open ? this.close() : this.show();
      return true;
    }
    if (exact && event.key?.toLowerCase() === "w") {
      this.close();
      return true;
    }
    if (exact && event.key?.toLowerCase() === "f") {
      this.window.focusSearch();
      return true;
    }
    if (event.key !== "Escape") return false;
    if (this.query) void this.search("");
    else if (this.detailId) {
      this.detailId = undefined;
      this.changed();
    } else this.close();
    return true;
  }
  async search(query: string) {
    this.query = query;
    this.searching = true;
    const epoch = ++this.searchEpoch;
    this.changed(); // keep previous results while the worker computes
    await Promise.resolve();
    if (!this.alive || epoch !== this.searchEpoch) return;
    this.reindex();
    this.searching = false;
    this.window.announce(
      this.matches.count
        ? `${this.matches.count} settings found`
        : "No settings found",
    );
    this.changed();
  }
  filterExtensions(query: string, type: ExtensionType = this.extensionType) {
    this.extensionQuery = query;
    this.extensionType = type;
    this.reindex();
    this.changed();
  }
  private reindex() {
    const query = this.query.trim().toLocaleLowerCase();
    const matches = emptyMatches();
    for (const row of this.rows)
      if (!query || searchable(row).includes(query)) {
        matches.rows.add(row.id);
        matches.sections.add(row.section);
        matches.count++;
      }
    for (const extension of this.publication.extensions) {
      const text = [
        extension.name,
        extension.type,
        extension.source,
        extension.about,
        ...extension.includes,
        ...extension.shortcuts,
        ...extension.files,
        ...extension.variables.map(searchable),
      ]
        .join(" ")
        .toLocaleLowerCase();
      if (!query || text.includes(query)) {
        matches.extensions.add(extension.id);
        matches.sections.add("extensions");
        matches.count++;
      }
    }
    for (const model of this.publication.speech)
      if (
        !query ||
        `${model.name} ${model.size}`.toLocaleLowerCase().includes(query)
      ) {
        matches.rows.add(`speech.${model.id}`);
        matches.sections.add("transcription");
        matches.count++;
      }
    if (!query)
      for (const section of this.availableSections)
        matches.sections.add(section);
    this.matches = matches;
    const extensionQuery = this.extensionQuery.trim().toLocaleLowerCase();
    this.filtered = this.publication.extensions.filter(
      (e) =>
        matches.extensions.has(e.id) &&
        (this.extensionType === "All Types" || e.type === this.extensionType) &&
        `${e.name} ${e.type} ${e.source} ${e.about}`
          .toLocaleLowerCase()
          .includes(extensionQuery),
    );
    if (this.anchorId) {
      const at = this.filtered.findIndex((e) => e.id === this.anchorId);
      if (at >= 0) this.viewportFirst = at;
    }
    this.range = [
      Math.max(0, this.viewportFirst - this.viewportCount),
      Math.min(
        this.filtered.length,
        this.viewportFirst + this.viewportCount * 2,
      ),
    ];
    if (this.range[0] > this.range[1]) this.range[0] = this.range[1];
  }
  /** Host supplies logical row indices: viewport plus one viewport above/below. */
  viewport(first: number, count: number) {
    if (![first, count].every(Number.isSafeInteger) || first < 0 || count < 0)
      throw new RangeError("Invalid viewport");
    const start = Math.min(first, this.filtered.length);
    this.viewportFirst = start;
    this.viewportCount = count;
    this.anchorId = this.filtered[start]?.id;
    this.range = [
      Math.max(0, start - count),
      Math.min(this.filtered.length, start + count * 2),
    ];
    this.changed();
  }
  inspect() {
    return {
      revision: this.publication.revision,
      activeSection: this.active,
      query: this.query,
      detailId: this.detailId,
      open: this.open,
      extensionCount: this.filtered.length,
      range: [...this.range],
      pending: [...this.pending],
      failedIds: [...this.failures.keys()],
    };
  }
  closeDetail() {
    this.detailId = undefined;
    this.changed();
  }
  viewExtension(id: string) {
    if (!this.filtered.some((e) => e.id === id)) return;
    this.detailId = id;
    this.changed();
  }
  publish(publication: Publication) {
    if (!this.alive || publication.revision <= this.publication.revision)
      return false;
    if (publication.project?.id !== this.publication.project?.id) {
      this.epoch++;
      this.pending.clear();
      this.failures.clear();
    }
    this.publication = publication;
    if (!this.availableSections.includes(this.active)) this.active = "general";
    if (!this.publication.extensions.some((e) => e.id === this.detailId))
      this.detailId = undefined;
    this.reindex();
    this.window.title(this.active);
    this.changed();
    return true;
  }
  async write(id: string, value: Json) {
    if (!this.alive) return false;
    const row = this.rows.find((r) => r.id === id);
    if (
      !row ||
      row.disabled ||
      row.busy ||
      this.pending.has(id) ||
      row.control === "secure" ||
      !validValue(row, value)
    )
      return false;
    if (!row.profileKey) return this.perform({ type: "write", id, value });
    const epoch = this.epoch;
    this.pending.add(id);
    this.failures.delete(id);
    this.changed();
    try {
      const ack = await this.host.setSetting(row.profileKey, value);
      if (!this.alive || epoch !== this.epoch) return false;
      this.profile.set(row.profileKey, value);
      this.reindex();
      return true;
    } catch (error) {
      if (this.alive && epoch === this.epoch) this.failures.set(id, error);
      return false;
    } finally {
      if (this.alive && epoch === this.epoch) {
        this.pending.delete(id);
        this.changed();
      }
    }
  }
  async perform(action: Action) {
    if (!this.services || !this.alive) return false;
    const id = "id" in action ? action.id : action.type;
    if (this.pending.has(id)) return false;
    if (action.type === "write") {
      const row = this.rows.find((r) => r.id === id);
      if (
        !row ||
        row.disabled ||
        row.busy ||
        row.control === "secure" ||
        row.profileKey ||
        !validValue(row, action.value)
      )
        return false;
    }
    if (action.type === "speech") {
      const item = this.publication.speech.find((item) => item.id === id);
      const states = {
        download: "available",
        cancel: "downloading",
        remove: "installed",
        retry: "error",
      };
      if (!item || item.state !== states[action.action]) return false;
    }
    if (action.type === "row") {
      const row = this.rows.find((r) => r.id === id);
      if (!row || row.disabled || row.busy || row.action !== action.action)
        return false;
    }
    if (action.type.startsWith("extension.")) {
      const extension = this.publication.extensions.find((e) => e.id === id);
      if (
        !extension ||
        (action.type === "extension.archive" && !extension.canArchive)
      )
        return false;
    }
    const epoch = this.epoch;
    this.pending.add(id);
    this.failures.delete(id);
    this.changed();
    try {
      const next = await this.services.perform(
        action,
        this.publication.revision,
      );
      if (!this.alive || epoch !== this.epoch) return false;
      return this.publish(next);
    } catch (error) {
      if (this.alive && epoch === this.epoch) this.failures.set(id, error);
      return false;
    } finally {
      if (this.alive && epoch === this.epoch) {
        this.pending.delete(id);
        this.changed();
      }
    }
  }
  dispose() {
    this.alive = false;
    this.epoch++;
    this.searchEpoch++;
    this.listeners.clear();
    this.pending.clear();
    this.profile.clear();
  }
}
function validValue(row: Row, value: Json) {
  if (row.control === "choice")
    return (
      row.options?.some(
        (option) =>
          !option.disabled &&
          JSON.stringify(option.v) === JSON.stringify(value),
      ) ?? false
    );
  if (row.control === "number")
    return (
      typeof value === "number" &&
      Number.isFinite(value) &&
      (row.min === undefined || value >= row.min) &&
      (row.max === undefined || value <= row.max)
    );
  if (row.control === "switch") return typeof value === "boolean";
  if (row.control === "text" || row.control === "colour")
    return typeof value === "string";
  return false;
}
