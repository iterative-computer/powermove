/** Effects spec §§1–6. Worker-owned browsing state, never a document mirror. */
export interface EffectItem {
  readonly id: string;
  readonly label: string;
  readonly group: string;
  readonly kind: "effect" | "preset";
  readonly installed: boolean;
  readonly failed?: boolean;
}
export interface Registry {
  readonly items: readonly EffectItem[];
  readonly groups: readonly string[];
}
export interface Target {
  readonly id: string;
  readonly effects: readonly string[];
}
export type Scope = "all" | "recent" | `group:${string}`;
export type Row =
  | {
      key: string;
      type: "group";
      name: string;
      count: number;
      expanded: boolean;
    }
  | { key: string; type: "item"; item: EffectItem; parent: string | null };
export interface SearchRequest {
  readonly items: readonly EffectItem[];
  readonly query: string;
}
export type Search = (request: SearchRequest) => Promise<readonly string[]>;
/** Runs only in the v4 worker realm, including for >2,000 items (N8). */
export const workerSearch: Search = async ({ items, query }) => {
  const needle = query.trim().toLowerCase();
  return items
    .filter((item) =>
      [item.label, item.id].some((text) => text.toLowerCase().includes(needle)),
    )
    .map((item) => item.id);
};
export interface ModelPorts {
  apply(item: EffectItem, target: Target): Promise<void>;
  reveal(item: EffectItem, target: Target): Promise<void>;
  persist(
    key: "pm.fx.recent" | "pm.fx.collapsed",
    value: string[],
  ): Promise<void>;
  search: Search;
  // DESIGN-TODO(opus): failure presentation/copy/retry/focus for apply, search
  // and settings is unspecified. Keep errors observable without invented UI.
  report(error: unknown): void;
}
export const ids = {
  panel: "fxbrowser",
  search: "fxbrowser:search",
  scope: "fxbrowser:scope",
  list: "fxbrowser:list",
  row: (key: string) => `fxbrowser:row:${key}`,
  add: (key: string) => `fxbrowser:add:${key}`,
};
const groupKey = (name: string) => `group:${encodeURIComponent(name)}`;
const itemKey = (section: string, id: string) =>
  `item:${encodeURIComponent(section)}:${encodeURIComponent(id)}`;
const unique = (values: readonly string[]) => [...new Set(values)];
export function readIds(value: unknown): string[] {
  // Migration values are read without deleting malformed or unknown source data.
  if (typeof value === "string") {
    try {
      value = JSON.parse(value);
    } catch {
      return [];
    }
  }
  return Array.isArray(value)
    ? unique(value.filter((id): id is string => typeof id === "string"))
    : [];
}
export function labelParts(label: string, query: string) {
  const needle = query.trim().toLowerCase();
  const index = needle ? label.toLowerCase().indexOf(needle) : -1;
  if (index < 0) return { before: label, match: "", after: "" };
  // Case conversion can expand a character (İ → i + combining dot). Map folded
  // UTF-16 offsets back to authored character boundaries instead of slicing the
  // original at an offset in a different string. Whole-label folding also keeps
  // context-sensitive Greek sigma matching consistent with workerSearch.
  let foldedOffset = 0,
    authoredOffset = 0,
    start: number | undefined,
    end = label.length;
  for (const character of label) {
    const next = foldedOffset + character.toLowerCase().length;
    if (start === undefined && next > index) start = authoredOffset;
    if (next >= index + needle.length) {
      end = authoredOffset + character.length;
      break;
    }
    foldedOffset = next;
    authoredOffset += character.length;
  }
  return {
    before: label.slice(0, start),
    match: label.slice(start, end),
    after: label.slice(end),
  };
}
export class EffectsModel {
  private items: readonly EffectItem[] = [];
  private byId = new Map<string, EffectItem>();
  private groups: readonly string[] = [];
  private grouped = new Map<string, EffectItem[]>();
  private listeners = new Set<() => void>();
  private epoch = 0;
  private retired = false;
  private matches: readonly string[] = [];
  private cachedRows: readonly Row[] = [];
  private settingsQueue: Promise<void> = Promise.resolve();
  private recentVersion = 0;
  private collapsedVersion = 0;
  query = "";
  searching = false;
  scope: Scope = "all";
  recent: readonly string[] = [];
  collapsed = new Set<string>();
  selected: string | null = null;
  focused: string | null = null;
  hovered: string | null = null;
  focus: "search" | "scope" | "list" | null = null;
  focusRequest: string | undefined;
  target: Target | null = null;
  error?: string;
  announcement = { sequence: 0, text: "" };
  viewport = { start: 0, size: 0 };
  constructor(private readonly ports: ModelPorts) {}
  subscribe(listener: () => void) {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }
  private changed() {
    if (!this.retired) for (const listener of this.listeners) listener();
  }
  private announce(text: string) {
    this.announcement = { sequence: this.announcement.sequence + 1, text };
  }
  private rebuild() {
    const anchor = this.cachedRows[this.viewport.start]?.key;
    const rows: Row[] = [];
    const addItems = (
      items: readonly EffectItem[],
      parent: string | null,
      section: string,
    ) => {
      for (const item of items)
        rows.push({
          key: itemKey(section, item.id),
          type: "item",
          item,
          parent,
        });
    };
    const recent = this.recentItems();
    if (this.scope === "recent" && !recent.length) this.scope = "all";
    if (
      this.scope.startsWith("group:") &&
      !this.groups.includes(this.scope.slice(6))
    )
      this.scope = "all";
    if (this.query.trim()) {
      addItems(
        this.matches.flatMap((id) => this.byId.get(id) ?? []),
        null,
        "search",
      );
    } else if (this.scope === "recent") addItems(recent, null, "recent-flat");
    else if (this.scope.startsWith("group:"))
      addItems(this.grouped.get(this.scope.slice(6)) ?? [], null, this.scope);
    else {
      const section = (
        name: string,
        entries: readonly EffectItem[],
        key: string,
      ) => {
        const expanded = !this.collapsed.has(key);
        rows.push({
          type: "group",
          key,
          name,
          count: entries.length,
          expanded,
        });
        if (expanded) addItems(entries, key, key);
      };
      if (recent.length) section("Recent", recent, "recent");
      for (const name of this.groups)
        section(name, this.grouped.get(name) ?? [], groupKey(name));
    }
    this.cachedRows = Object.freeze(rows.map((row) => Object.freeze(row)));
    const anchored = anchor ? rows.findIndex((row) => row.key === anchor) : -1;
    this.viewport.start =
      anchored >= 0
        ? anchored
        : Math.min(this.viewport.start, Math.max(0, rows.length - 1));
    if (this.focused && !rows.some((row) => row.key === this.focused))
      this.focused = rows[0]?.key ?? null;
  }
  replaceRegistry(registry: Registry) {
    if (this.retired) return;
    const seen = new Set<string>();
    const items = registry.items
      .map((item) => {
        if (!item.id || seen.has(item.id))
          throw new Error("Invalid/duplicate effect id");
        seen.add(item.id);
        return Object.freeze({ ...item });
      })
      .filter((item) => item.installed);
    this.items = Object.freeze(items);
    this.byId = new Map(items.map((item) => [item.id, item]));
    this.groups = unique([
      ...registry.groups,
      ...items.map((item) => item.group),
    ]);
    this.grouped = new Map(this.groups.map((group) => [group, []]));
    for (const item of items) this.grouped.get(item.group)!.push(item);
    if (this.selected && !this.byId.has(this.selected)) this.selected = null;
    this.rebuild();
    if (this.query.trim()) void this.setQuery(this.query);
    else this.changed();
  }
  settingsStamp() {
    return { recent: this.recentVersion, collapsed: this.collapsedVersion };
  }
  restoreSettings(
    recent: unknown,
    collapsed: unknown,
    stamp = this.settingsStamp(),
  ) {
    if (this.retired) return;
    if (stamp.recent === this.recentVersion)
      this.recent = readIds(recent).slice(0, 8);
    if (stamp.collapsed === this.collapsedVersion)
      this.collapsed = new Set(readIds(collapsed));
    this.rebuild();
    this.changed();
  }
  private save(
    key: "pm.fx.recent" | "pm.fx.collapsed",
    value: readonly string[],
  ) {
    const copy = [...value];
    this.settingsQueue = this.settingsQueue
      .then(async () => {
        if (!this.retired) await this.ports.persist(key, copy);
      })
      .catch((error: unknown) => {
        if (!this.retired) this.ports.report(error);
      });
  }
  async settingsSettled() {
    await this.settingsQueue;
  }
  recentItems() {
    return this.recent.flatMap((id) => this.byId.get(id) ?? []);
  }
  scopes(): { value: Scope; label: string }[] {
    return [
      { value: "all", label: "All" },
      ...(this.recentItems().length
        ? [{ value: "recent" as const, label: "Recent" }]
        : []),
      ...this.groups.map((name) => ({
        value: `group:${name}` as Scope,
        label: name,
      })),
    ];
  }
  rows() {
    return this.cachedRows;
  }
  installedCount() {
    return this.items.length;
  }
  resultCount() {
    return this.query.trim()
      ? this.matches.filter((id) => this.byId.has(id)).length
      : this.items.length;
  }
  async setQuery(query: string) {
    if (this.retired) return;
    this.error = undefined;
    this.query = query;
    const epoch = ++this.epoch;
    if (!query.trim()) {
      this.searching = false;
      this.rebuild();
      this.announce(`${this.items.length} effects`);
      this.changed();
      return;
    }
    this.searching = true;
    this.changed(); // Previous accepted rows remain visible until the worker answers.
    try {
      const matches = await this.ports.search({ items: this.items, query });
      if (epoch !== this.epoch || this.retired) return;
      this.matches = unique(matches).filter((id) => this.byId.has(id));
      this.searching = false;
      this.viewport.start = 0;
      this.rebuild();
      this.announce(`${this.matches.length} effects`);
      this.changed();
    } catch (error) {
      if (epoch === this.epoch && !this.retired) {
        this.searching = false;
        this.error = "Couldn’t search effects. Try again.";
        this.announce(this.error);
        this.ports.report(error);
        this.changed();
      }
    }
  }
  pickScope(scope: Scope) {
    if (!this.scopes().some((option) => option.value === scope)) return;
    ++this.epoch;
    this.error = undefined;
    this.query = "";
    this.searching = false;
    this.scope = scope;
    this.viewport.start = 0;
    this.rebuild();
    this.changed();
  }
  setTarget(target: Target | null) {
    if (this.retired) return;
    this.target = target
      ? Object.freeze({
          ...target,
          effects: Object.freeze([...target.effects]),
        })
      : null;
    this.changed();
  }
  setViewport(start: number, size: number) {
    if (![start, size].every((n) => Number.isSafeInteger(n) && n >= 0))
      throw new RangeError("Invalid viewport");
    this.viewport = {
      start: Math.min(start, Math.max(0, this.cachedRows.length - 1)),
      size,
    };
    this.changed();
  }
  window() {
    const { start, size } = this.viewport;
    const from = Math.max(0, start - size);
    const to = Math.min(this.cachedRows.length, start + size * 2);
    return {
      from,
      to,
      count: this.cachedRows.length,
      rows: this.cachedRows.slice(from, to),
    };
  }
  select(key: string) {
    const row = this.cachedRows.find((row) => row.key === key);
    if (row?.type !== "item") return;
    this.selected = row.item.id;
    this.focused = key;
    this.changed();
  }
  hover(key: string | null) {
    this.hovered = key;
    this.changed();
  }
  setFocus(focus: typeof this.focus) {
    this.focus = focus;
    this.changed();
  }
  private focusRow(key: string | null) {
    this.focused = key;
    this.focus = "list";
    const index = this.cachedRows.findIndex((row) => row.key === key);
    if (
      index >= 0 &&
      (index < this.viewport.start ||
        index >= this.viewport.start + this.viewport.size)
    )
      this.viewport.start = index;
    this.focusRequest = key ? ids.row(key) : ids.list;
    this.changed();
  }
  toggle(key: string, expanded?: boolean) {
    const row = this.cachedRows.find((row) => row.key === key);
    if (row?.type !== "group") return;
    const open = expanded ?? !row.expanded;
    if (
      !open &&
      this.cachedRows.some(
        (child) =>
          child.key === this.focused &&
          child.type === "item" &&
          child.parent === key,
      )
    )
      this.focusRow(key);
    if (open) this.collapsed.delete(key);
    else this.collapsed.add(key);
    ++this.collapsedVersion;
    this.rebuild();
    this.save("pm.fx.collapsed", [...this.collapsed]);
    this.changed();
  }
  async apply(id: string) {
    const item = this.byId.get(id);
    if (!item || item.failed || this.retired) return false;
    this.error = undefined;
    const target = this.target;
    if (!target) {
      this.announce("Select a layer first");
      this.changed();
      return false;
    }
    try {
      await this.ports.apply(item, target);
      if (this.retired) return false;
      this.recent = [id, ...this.recent.filter((other) => other !== id)].slice(
        0,
        8,
      );
      ++this.recentVersion;
      this.save("pm.fx.recent", this.recent);
      this.rebuild();
      this.announce(`Added ${item.label}`);
      this.changed();
      return true;
    } catch (error) {
      if (!this.retired) {
        this.error = `Couldn’t add ${item.label}. Try again.`;
        this.announce(this.error);
        this.ports.report(error);
        this.changed();
      }
      return false;
    }
  }
  drag(id: string) {
    const item = this.byId.get(id);
    return item && !item.failed
      ? { kind: "effect" as const, id: item.id, label: item.label }
      : null;
  }
  context(id: string) {
    const item = this.byId.get(id);
    if (!item) return [];
    return [
      {
        id: "add",
        label: "Add to Selected Layer",
        disabled: !this.target || !!item.failed,
      },
      ...(this.target?.effects.includes(id)
        ? [{ id: "reveal", label: "Show in Effect Controls", disabled: false }]
        : []),
    ];
  }
  async contextAction(id: string, action: string) {
    if (action === "add") return this.apply(id);
    const item = this.byId.get(id),
      target = this.target;
    if (
      action !== "reveal" ||
      !item ||
      !target?.effects.includes(id) ||
      this.retired
    )
      return false;
    try {
      await this.ports.reveal(item, target);
      return !this.retired;
    } catch (error) {
      if (!this.retired) this.ports.report(error);
      return false;
    }
  }
  async key(
    key: string,
    modifiers: {
      metaKey?: boolean;
      ctrlKey?: boolean;
      altKey?: boolean;
      composing?: boolean;
      repeat?: boolean;
    } = {},
  ) {
    if (
      this.retired ||
      modifiers.metaKey ||
      modifiers.ctrlKey ||
      modifiers.altKey ||
      modifiers.composing
    )
      return false;
    if (this.focus === "scope") return false; // Native pop-up owns its keyboard.
    if (this.focus === "search") {
      if (key === "Escape") {
        if (this.query) await this.setQuery("");
        else this.focusRow(this.focused ?? this.cachedRows[0]?.key ?? null);
      } else if (key === "ArrowDown")
        this.focusRow(this.cachedRows[0]?.key ?? null);
      else return false;
      return true;
    }
    if (this.focus !== "list") return false;
    const index = this.cachedRows.findIndex((row) => row.key === this.focused);
    const row = this.cachedRows[index];
    if (["ArrowUp", "ArrowDown", "Home", "End"].includes(key)) {
      const next =
        key === "Home"
          ? 0
          : key === "End"
            ? this.cachedRows.length - 1
            : index + (key === "ArrowDown" ? 1 : -1);
      this.focusRow(
        this.cachedRows[Math.max(0, Math.min(this.cachedRows.length - 1, next))]
          ?.key ?? null,
      );
    } else if (key === "ArrowRight" && row?.type === "group")
      this.toggle(row.key, true);
    else if (key === "ArrowLeft" && row?.type === "group")
      this.toggle(row.key, false);
    else if (key === "ArrowLeft" && row?.type === "item" && row.parent)
      this.focusRow(row.parent);
    else if (key === "Enter" && row?.type === "item") {
      if (!modifiers.repeat) await this.apply(row.item.id);
    } else if (key === "Escape") {
      this.focus = "search";
      this.focusRequest = ids.search;
      this.changed();
    } else if ([...key].length === 1) {
      this.focus = "search";
      this.focusRequest = ids.search;
      await this.setQuery(this.query + key);
    } else return false;
    return true;
  }
  inspect() {
    return {
      panel: ids.panel,
      title: "Effects",
      scope: this.scope,
      query: this.query,
      selected: this.selected,
      focused: this.focused,
      focus: this.focus,
      target: this.target,
      searching: this.searching,
      recent: [...this.recent],
      collapsed: [...this.collapsed],
      announcement: { ...this.announcement },
      window: this.window(),
      results: this.resultCount(),
      acceptsDrops: false,
      selectionMode: "single",
    };
  }
  dispose() {
    this.retired = true;
    ++this.epoch;
    this.listeners.clear();
  }
}
