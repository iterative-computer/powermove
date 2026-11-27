import { ThemeError } from "./css.mjs";
import { copyData } from "./data.mjs";
import { translateTheme, resolveScheme } from "./translator.mjs";

export const ABSENT = Object.freeze({ present: false });
export const present = (value) => ({ present: true, value: copyData(value) });
const equal = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const fieldValue = (fields, key) =>
  Object.hasOwn(fields, key) ? fields[key] : ABSENT;
const clone = copyData;

/** Pure reducer, owned by the kernel/realm adapter. It never writes a project,
 * store or view. Patches are applied by the kernel actor, including to inactive
 * saved workspaces. Missing fields are tombstones, not ambiguous JSON nulls.
 */
export class AppearanceLedger {
  constructor(state = { version: 1, scopes: [], archive: [] }) {
    if (
      state.version !== 1 ||
      !Array.isArray(state.scopes) ||
      !Array.isArray(state.archive)
    )
      throw new ThemeError(
        "invalid_appearance_state",
        "Unknown ledger format; archive without replacing it",
      );
    this.scopes = new Map(
      clone(state.scopes).map(([scope, fields]) => [scope, new Map(fields)]),
    );
    this.original = clone(state);
    this.archive = clone(state.archive);
  }
  record(owner, scope, before, after) {
    if (!owner || !scope)
      throw new ThemeError(
        "invalid_owner",
        "Appearance needs stable owner and scope ids",
      );
    let fields = this.scopes.get(scope);
    for (const key of new Set([
      ...Object.keys(before),
      ...Object.keys(after),
    ])) {
      const previous = fieldValue(before, key),
        value = fieldValue(after, key);
      if (equal(previous, value)) continue;
      if (!fields) {
        fields = new Map();
        this.scopes.set(scope, fields);
      }
      let entry = fields.get(key);
      if (!entry || !equal(entry.stack.at(-1)?.value, previous)) {
        if (entry)
          this.archive.push({
            scope,
            key,
            entry: clone(entry),
            reason: "new_baseline",
          });
        entry = { base: clone(previous), stack: [] };
        fields.set(key, entry);
      }
      const replaced = entry.stack.filter((frame) => frame.owner === owner);
      if (replaced.length)
        this.archive.push({
          scope,
          key,
          frames: clone(replaced),
          reason: "owner_reapply",
        });
      entry.stack = entry.stack.filter((frame) => frame.owner !== owner);
      entry.stack.push({ owner, value: clone(value) });
    }
  }
  release(owner, currentByScope) {
    const patches = [],
      pending = [];
    for (const [scope, fields] of this.scopes) {
      if (!Object.hasOwn(currentByScope, scope)) {
        if (
          [...fields.values()].some((entry) =>
            entry.stack.some((frame) => frame.owner === owner),
          )
        )
          pending.push(scope);
        continue;
      }
      const current = currentByScope[scope];
      for (const [key, entry] of fields) {
        if (!entry.stack.some((frame) => frame.owner === owner)) continue;
        const top = entry.stack.at(-1);
        this.archive.push({
          scope,
          key,
          entry: clone(entry),
          reason: "release",
          owner,
        });
        if (!equal(fieldValue(current, key), top.value)) {
          fields.delete(key);
          continue; // A later user edit is authoritative.
        }
        entry.stack = entry.stack.filter((frame) => frame.owner !== owner);
        if (top.owner === owner)
          patches.push({
            scope,
            key,
            expected: clone(top.value),
            value: clone(entry.stack.at(-1)?.value ?? entry.base),
          });
        if (!entry.stack.length) fields.delete(key);
      }
      if (!fields.size) this.scopes.delete(scope);
    }
    return { patches, pending };
  }
  reconcile(owners, currentByScope) {
    const stale = new Set(
      [...this.scopes.values()]
        .flatMap((fields) =>
          [...fields.values()].flatMap((entry) =>
            entry.stack.map((frame) => frame.owner),
          ),
        )
        .filter((owner) => !owners.has(owner)),
    );
    const staged = clone(currentByScope);
    return [...stale].map((owner) => {
      const result = this.release(owner, staged);
      for (const patch of result.patches)
        staged[patch.scope][patch.key] = clone(patch.value);
      return { owner, ...result };
    });
  }
  export() {
    return {
      ...clone(this.original),
      version: 1,
      scopes: clone(
        [...this.scopes].map(([scope, fields]) => [scope, [...fields]]),
      ),
      archive: clone(this.archive),
    };
  }
}

/** Import the two shipped v1 ownership records. Originals, unknown fields and
 * rejected entries remain in the private archive. Diagnostics block certifying
 * a complete migration; an unsupported format never becomes an empty success.
 */
export function importLegacyAppearance(workspaceState, themeState) {
  const state = {
    version: 1,
    scopes: [],
    archive: [
      {
        reason: "legacy_import",
        workspaceState: clone(workspaceState),
        themeState: clone(themeState),
      },
    ],
  };
  const diagnostics = [];
  function entry(input) {
    if (
      !input ||
      !Array.isArray(input.stack) ||
      !input.stack.length ||
      !input.stack.every((frame) => typeof frame?.owner === "string")
    )
      return null;
    return {
      base: Object.hasOwn(input, "base") ? present(input.base) : ABSENT,
      stack: input.stack.map((frame) => ({
        owner: frame.owner,
        value: Object.hasOwn(frame, "value") ? present(frame.value) : ABSENT,
      })),
    };
  }
  if (workspaceState !== undefined && workspaceState !== null) {
    if (
      workspaceState.version !== 1 ||
      !Array.isArray(workspaceState.workspaces)
    )
      diagnostics.push({ code: "legacy_workspace_format" });
    else
      for (const item of workspaceState.workspaces) {
        if (
          !Array.isArray(item) ||
          typeof item[0] !== "string" ||
          !Array.isArray(item[1])
        ) {
          diagnostics.push({ code: "legacy_workspace_entry" });
          continue;
        }
        const fields = [];
        for (const field of item[1]) {
          let valid = false;
          try {
            valid =
              typeof field?.[0] === "string" && validPath(JSON.parse(field[0]));
          } catch {
            /* Archive malformed paths. */
          }
          const parsed = entry(field?.[1]);
          if (valid && parsed) fields.push([field[0], parsed]);
          else
            diagnostics.push({
              code: "legacy_workspace_field",
              workspace: item[0],
            });
        }
        if (fields.length) state.scopes.push([`workspace:${item[0]}`, fields]);
      }
  }
  if (themeState !== undefined && themeState !== null) {
    if (themeState.version !== 1 || !Array.isArray(themeState.fields))
      diagnostics.push({ code: "legacy_theme_format" });
    else {
      const fields = [];
      for (const field of themeState.fields) {
        const parsed = entry(field?.[1]);
        const valid = (value) =>
          typeof value === "string" &&
          (field[0] === "scheme"
            ? ["light", "dark", "system"].includes(value)
            : value.length > 0);
        if (
          ["activeId", "scheme"].includes(field?.[0]) &&
          parsed &&
          parsed.base.present &&
          valid(parsed.base.value) &&
          parsed.stack.every(
            (frame) => frame.value.present && valid(frame.value.value),
          )
        )
          fields.push([field[0], parsed]);
        else diagnostics.push({ code: "legacy_theme_field" });
      }
      if (fields.length) state.scopes.push(["theme", fields]);
    }
  }
  return { state, diagnostics };
}

// This seam wraps the existing synchronous workspace facade on its V8 thread.
// Disposal uses ledger.release and kernel-applied patches, rather than a second
// writer. Capture in finally preserves the tag's partial writes when fn throws.
export function captureWorkspaceMutation(ledger, owner, api, fn, options) {
  const before = workspaceFields(api.current() ?? {});
  let result;
  try {
    result = api.mutate(fn, options);
    return result;
  } finally {
    const after = result ?? api.current();
    if (after)
      ledger.record(
        owner,
        `workspace:${after.id}`,
        before,
        workspaceFields(after),
      );
  }
}

const safe = (key) => !["__proto__", "prototype", "constructor"].includes(key);
const dockFields = ["size", "flex", "hidden"];
const panelFields = ["size", "min", "flex", "collapsed", "title"];
export function workspaceFields(workspace) {
  const fields = {};
  function visit(value, path) {
    if (value && typeof value === "object" && !Array.isArray(value)) {
      fields[JSON.stringify(path)] = { present: true, container: true };
      for (const [key, item] of Object.entries(value))
        if (safe(key)) visit(item, [...path, key]);
    } else
      fields[JSON.stringify(path)] =
        value === undefined ? ABSENT : present(value);
  }
  visit(workspace.density, ["density"]);
  visit(workspace.theme, ["theme"]);
  visit(workspace.chrome, ["chrome"]);
  for (const dock of workspace.layout?.docks ?? []) {
    const path = ["layout", "docks", { dock: dock.id }];
    for (const key of dockFields) visit(dock[key], [...path, key]);
    for (const panel of dock.panels ?? [])
      for (const key of panelFields)
        visit(panel[key], [...path, "panels", { panel: panel.id }, key]);
  }
  return fields;
}

function validPath(path) {
  if (
    !Array.isArray(path) ||
    !path.length ||
    typeof path.at(-1) !== "string" ||
    path.some((part) => typeof part === "string" && !safe(part))
  )
    return false;
  if (path[0] === "density") return path.length === 1;
  if (["theme", "chrome"].includes(path[0]))
    return path.every((part) => typeof part === "string");
  return (
    path[0] === "layout" &&
    path[1] === "docks" &&
    typeof path[2]?.dock === "string" &&
    ((path.length === 4 && dockFields.includes(path[3])) ||
      (path.length === 6 &&
        path[3] === "panels" &&
        typeof path[4]?.panel === "string" &&
        panelFields.includes(path[5])))
  );
}

// Returns a copy for a kernel transaction. Never resurrect a deleted dock/panel.
export function applyWorkspacePatches(workspace, patches) {
  const result = clone(workspace);
  const rank = (patch) =>
    patch.value.container ? 0 : patch.expected?.container ? 2 : 1;
  const ordered = patches
    .map((patch) => ({ ...patch, path: JSON.parse(patch.key) }))
    .sort(
      (a, b) =>
        rank(a) - rank(b) ||
        Number(a.value.present) - Number(b.value.present) ||
        (rank(a) === 2
          ? b.path.length - a.path.length
          : a.path.length - b.path.length),
    );
  for (const { key: encodedPath, path, value, expected } of ordered) {
    if (!validPath(path))
      throw new ThemeError(
        "invalid_appearance_path",
        "Unsafe workspace appearance path",
      );
    if (
      expected &&
      !equal(fieldValue(workspaceFields(result), encodedPath), expected)
    )
      continue;
    let target = result;
    for (const part of path.slice(0, -1)) {
      if (typeof part === "string") {
        if (!target || typeof target !== "object") {
          target = null;
          break;
        }
        if (!target[part] || typeof target[part] !== "object") {
          if (!value.present || path[0] === "layout") {
            target = null;
            break;
          }
          target[part] = {};
        }
        target = target[part];
      } else
        target = Array.isArray(target)
          ? target.find((item) => item.id === (part.dock ?? part.panel))
          : null;
      if (!target) break;
    }
    if (!target) continue;
    const key = path.at(-1);
    if (
      expected?.container &&
      target[key] &&
      typeof target[key] === "object" &&
      Object.keys(target[key]).length
    )
      continue;
    if (value.container) {
      if (!target[key] || typeof target[key] !== "object") target[key] = {};
    } else if (value.present) target[key] = clone(value.value);
    else delete target[key]; // Removes an override field, never authored sources.
  }
  return result;
}

/** Synchronous registry facade for a single V8 realm. Persist state/receipts
 * through the kernel adapter after each call; publish translated snapshots to
 * native consumers asynchronously. This object owns no disk or UI state.
 */
export class ThemeController {
  constructor({ state, systemScheme = "light", onChange = () => {} } = {}) {
    if (state && state.version !== 1)
      throw new ThemeError(
        "invalid_appearance_state",
        "Unknown controller format; archive without replacing it",
      );
    this.original = clone(state ?? {});
    this.ledger = new AppearanceLedger(state?.ledger);
    this.selection = clone(
      state?.selection ?? { activeId: "default", scheme: "system" },
    );
    this.pendingSelection = state?.pendingSelection ?? null;
    this.systemScheme = systemScheme;
    this.onChange = onChange;
    this.registrations = new Map();
    this.archive = clone(state?.archive ?? []);
  }
  definition(id = this.selection.activeId) {
    return this.registrations.get(id)?.at(-1)?.definition;
  }
  fields() {
    return {
      activeId: present(this.selection.activeId),
      scheme: present(this.selection.scheme),
    };
  }
  notify() {
    this.onChange({
      id: this.selection.activeId,
      scheme: resolveScheme(
        this.definition(),
        this.selection.scheme,
        this.systemScheme,
      ),
    });
  }
  change(owner, field, value) {
    if (!["activeId", "scheme"].includes(field))
      throw new ThemeError(
        "invalid_appearance_path",
        "Unknown theme state field",
      );
    if (field === "scheme" && !["light", "dark", "system"].includes(value))
      throw new ThemeError("invalid_scheme", "Invalid scheme");
    if (field === "activeId" && (typeof value !== "string" || !value))
      throw new ThemeError("invalid_theme", "Invalid theme id");
    const before = this.fields();
    this.selection[field] = value;
    if (owner) this.ledger.record(owner, "theme", before, this.fields());
    if (field === "activeId") this.pendingSelection = null;
    this.notify();
  }
  setSystemScheme(scheme) {
    if (!["light", "dark"].includes(scheme))
      throw new ThemeError("invalid_scheme", "Invalid system scheme");
    this.systemScheme = scheme;
    if (
      this.selection.scheme === "system" &&
      !["light", "dark"].includes(this.definition()?.scheme)
    )
      this.notify();
  }
  register(owner, input, trust = "trusted") {
    const qualified =
      trust === "store" &&
      typeof input?.id === "string" &&
      input.id &&
      input.id !== owner &&
      !input.id.startsWith(`${owner}.`)
        ? { ...input, id: `${owner}.${input.id}` }
        : input;
    if (
      trust === "store" &&
      this.registrations.get(qualified?.id)?.at(-1)?.owner &&
      this.registrations.get(qualified.id).at(-1).owner !== owner
    )
      throw new ThemeError(
        "permission_denied",
        "Store theme id belongs to another owner",
      );
    // Compile before registration: invalid CSS never replaces a working theme.
    translateTheme(qualified, { trust });
    const definition =
      trust === "store" ? { ...clone(qualified), rootAttributes: {} } : input;
    const entry = {
      owner,
      definition,
      trust,
      authoredDefinition: clone(input),
    };
    const stack = this.registrations.get(definition.id) ?? [];
    stack.push(entry);
    this.registrations.set(definition.id, stack);
    if (
      this.pendingSelection === definition.id &&
      this.selection.activeId === "default"
    ) {
      this.selection.activeId = this.pendingSelection;
      this.pendingSelection = null;
    }
    if (this.selection.activeId === definition.id) this.notify();
    let disposed = false;
    return {
      dispose: () => {
        if (disposed) return;
        disposed = true;
        this.archive.push(clone(entry));
        const current = this.registrations.get(definition.id) ?? [];
        const wasTop = current.at(-1) === entry;
        const remaining = current.filter((candidate) => candidate !== entry);
        if (remaining.length) this.registrations.set(definition.id, remaining);
        else this.registrations.delete(definition.id);
        if (wasTop && this.selection.activeId === definition.id) this.notify();
      },
    };
  }
  api(owner, trust = "trusted") {
    let disposed = false;
    const handles = [];
    return {
      register: (definition) => {
        if (disposed) return { dispose() {} };
        const handle = this.register(owner, definition, trust);
        handles.push(handle);
        return handle;
      },
      activate: (id) => {
        if (!disposed) {
          const qualified =
            trust === "store" &&
            typeof id === "string" &&
            id &&
            id !== owner &&
            !id.startsWith(`${owner}.`)
              ? `${owner}.${id}`
              : id;
          this.change(owner, "activeId", qualified);
        }
      },
      active: () => this.selection.activeId,
      list: () =>
        [...this.registrations.values()].map((stack) =>
          trust === "store"
            ? clone(stack.at(-1).definition)
            : stack.at(-1).definition,
        ),
      scheme: () =>
        trust === "store"
          ? resolveScheme(
              this.definition(),
              this.selection.scheme,
              this.systemScheme,
            )
          : this.selection.scheme,
      setScheme: (mode) => {
        if (trust === "store")
          throw new ThemeError(
            "permission_denied",
            "theme.setScheme is denied in the Store sandbox",
          );
        if (!disposed) this.change(owner, "scheme", mode);
      },
      dispose: () => {
        if (disposed) return;
        disposed = true;
        for (const handle of handles) handle.dispose();
        this.release(owner);
      },
    };
  }
  release(owner) {
    const { patches } = this.ledger.release(owner, { theme: this.fields() });
    for (const patch of patches) {
      let value = patch.value.value;
      if (
        patch.key === "activeId" &&
        value !== "default" &&
        !this.definition(value)
      ) {
        this.pendingSelection = value;
        value = "default";
      }
      this.selection[patch.key] = value;
    }
    if (patches.length) this.notify();
  }
  plan(options = {}) {
    return translateTheme(
      this.definition() ?? {
        id: this.selection.activeId,
        name: "Default",
        scheme: "auto",
      },
      {
        ...options,
        trust:
          this.registrations.get(this.selection.activeId)?.at(-1)?.trust ??
          "trusted",
        preference: this.selection.scheme,
        systemScheme: this.systemScheme,
      },
    );
  }
  export() {
    return {
      ...clone(this.original),
      version: 1,
      selection: clone(this.selection),
      pendingSelection: this.pendingSelection,
      ledger: this.ledger.export(),
      archive: clone(this.archive),
    };
  }
}
