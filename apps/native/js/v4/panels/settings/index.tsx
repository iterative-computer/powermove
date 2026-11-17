import { useLayoutEffect, useState } from "preact/hooks";
import type { ComponentChildren } from "preact";
import * as UI from "../../src/components";
import type { Json, SurfaceEvent } from "../../src/protocol";
import {
  extensionTypes,
  sectionCopy,
  type Row,
  type Section,
} from "./catalogue";
import { SettingsModel, type SpeechModel } from "./model";
export { SettingsModel } from "./model";
export type {
  SettingsServices,
  ProfileSettings,
  SettingsWindow,
  Publication,
  Action,
  Extension,
  SpeechModel,
} from "./model";

export interface SettingsPanelProps {
  key?: string;
  model: SettingsModel;
  /** Host-owned local native text/select leases; not kernel channel names. */
  bindings: {
    search: string;
    extensionSearch: string;
    extensionType: string;
    project?: string;
    commandSearch?: string;
  };
  /** The header's picker chose project `id`; Show in Finder. */
  window?: { project(id: string): void; reveal(): void };
}

function Copy({
  children,
  secondary = false,
}: {
  children: ComponentChildren;
  secondary?: boolean;
  key?: string | number;
}) {
  return (
    <text metadata={{ settingsLayout: secondary ? "secondary" : "label" }}>
      {children}
    </text>
  );
}
function Button({
  label,
  accessibilityLabel,
  disabled,
  onClick,
  destructive = false,
}: {
  label: string;
  accessibilityLabel?: string;
  destructive?: boolean;
  disabled?: boolean;
  onClick: () => void;
  key?: string;
}) {
  return (
    <UI.PushButton
      label={accessibilityLabel ?? label}
      role={destructive ? "destructive" : "normal"}
      disabled={disabled}
      onClick={onClick}
    >
      {label}
    </UI.PushButton>
  );
}
/** A colour as a row says it: Black and White by name, else its hex. */
function colourName(hex: string) {
  const v = hex.replace("#", "").slice(0, 6).toLowerCase();
  if (v === "000000") return "Black";
  if (v === "ffffff") return "White";
  return `#${v.toUpperCase()}`;
}
/** The row Settings › Project's header card holds. */
const HEADER_ROW = "project.name";
/** Rows the design gives a stepper beside the field. */
const STEPPED = new Set(["project.fps"]);
/** Short exclusive choices the design shows as a segmented control. */
const SEGMENTED = new Set([
  "project.colorDepth",
  "appearance",
  "open-projects-in",
]);
function RowControl({
  row,
  model,
}: {
  row: Row;
  model: SettingsModel;
  key?: string;
}) {
  // A choice stays open while the previous one saves (the model keeps the
  // last); an action waits for the one in progress.
  const disabled =
    row.disabled ||
    row.busy ||
    (row.control === "action" && model.pending.has(row.id));
  const change = (event: SurfaceEvent) => {
    if (event.value !== undefined) void model.write(row.id, event.value);
  };
  const common = {
    label: row.label,
    disabled: disabled || !row.binding,
    binding: row.binding ?? "",
    attr: { "aria-description": row.detail ?? "" },
    onChange: change,
  };
  if (row.busy) return <UI.ProgressRow label="Checking…" />;
  if (row.control === "switch" && typeof row.value === "boolean")
    return <UI.Switch {...common} value={row.value} />;
  if (row.control === "secure")
    return row.binding ? (
      <UI.SecureField
        label={row.label}
        binding={row.binding}
        disabled={disabled}
      />
    ) : null;
  // AppKit's minimal colour well, its colour named beside it.
  if (row.control === "colour" && typeof row.value === "string")
    return (
      <div metadata={{ settingsLayout: "line" }}>
        <Copy secondary>{colourName(row.value)}</Copy>
        <UI.ColorWell
          {...common}
          minimal
          value={row.value}
          metadata={{ native: true }}
        />
      </div>
    );
  if (row.control === "action") {
    const button = (
      <Button
        // A new label (Sign In… → Sign Out) is a new control: the native
        // button keeps the title it was created with.
        key={row.actionLabel}
        label={row.actionLabel!}
        disabled={disabled || !model.services}
        onClick={() => {
          void model.perform({ type: "row", id: row.id, action: row.action! });
        }}
      />
    );
    // A size beside its button (Settings › Storage: 9.3 GB  Delete…).
    if (typeof row.value !== "string") return button;
    return (
      <div metadata={{ settingsLayout: "line" }}>
        <Copy secondary>{row.value}</Copy>
        {button}
      </div>
    );
  }
  if (row.control === "status")
    return <Copy>{typeof row.value === "string" ? row.value : ""}</Copy>;
  // An unknown authored value stays in the service; never coerce it into a default.
  if (!row.binding) return null;
  // The design's controls: AppKit's bordered fields (a stepper on Frame
  // Rate), and a segmented control for a short exclusive choice.
  if (row.control === "number" && typeof row.value === "number")
    return (
      <UI.NumberField
        {...common}
        value={row.value}
        min={row.min}
        max={row.max}
        unit={row.unit}
        precision={row.precision}
        metadata={{ form: true, stepper: STEPPED.has(row.id) }}
      />
    );
  if (row.control === "text" && typeof row.value === "string")
    return (
      <UI.TextField {...common} value={row.value} metadata={{ form: true }} />
    );
  if (
    row.control === "choice" &&
    row.value !== undefined &&
    SEGMENTED.has(row.id) &&
    row.options?.length
  ) {
    const options = row.options;
    return (
      <UI.SegmentedControl
        label={row.label}
        disabled={common.disabled}
        attr={common.attr}
        value={String(row.value)}
        options={options.map((o) => ({ id: String(o.v), label: o.label }))}
        onChange={(id) => {
          const option = options.find((o) => String(o.v) === id);
          if (option) void model.write(row.id, option.v);
        }}
      />
    );
  }
  if (row.control === "choice" && row.value !== undefined)
    return (
      <UI.PopUpButton
        {...common}
        value={row.value}
        options={row.options ?? []}
      />
    );
  return null;
}
function RowCopy({
  title,
  detail,
  children,
}: {
  title: string;
  detail?: string;
  children?: ComponentChildren;
}) {
  return (
    <div metadata={{ settingsLayout: "copy" }}>
      <text>{title}</text>
      {detail && <text metadata={{ settingsLayout: "footer" }}>{detail}</text>}
      {children}
    </div>
  );
}
function Group({
  title,
  muted,
  children,
}: {
  title: string;
  muted?: boolean;
  children: ComponentChildren;
  key?: string;
}) {
  return (
    <div role="group" label={title} metadata={{ settingsLayout: "group" }}>
      <text
        role="heading"
        metadata={{ settingsLayout: muted ? "mutedHeading" : "heading" }}
      >
        {title}
      </text>
      <UI.FormGroup label={title}>{children}</UI.FormGroup>
    </div>
  );
}
function SettingRow({
  row,
  model,
}: {
  row: Row;
  model: SettingsModel;
  key?: string;
}) {
  return (
    <div
      id={`settings.row.${row.id}`}
      role="group"
      label={row.label}
      metadata={{ settingsLayout: "row" }}
    >
      <RowCopy
        title={row.label}
        detail={row.id === "media.location" ? undefined : row.detail}
      >
        {row.error && (
          <div
            role="status"
            label={row.error}
            metadata={{
              settingsLayout: "error",
              symbol: "exclamationmark.triangle.fill",
            }}
          >
            {row.error}
          </div>
        )}
      </RowCopy>
      <div metadata={{ settingsLayout: "line" }}>
        {row.id === "media.location" && row.detail && (
          <text metadata={{ settingsLayout: "path" }}>{row.detail}</text>
        )}
        <RowControl row={row} model={model} />
      </div>
    </div>
  );
}
function SpeechRow({
  item,
  model,
}: {
  item: SpeechModel;
  model: SettingsModel;
  key?: string;
}) {
  const act = (action: "download" | "cancel" | "remove" | "retry" | "use") => {
    void model.perform({ type: "speech", id: item.id, action });
  };
  const disabled = !model.services || model.pending.has(item.id);
  return (
    <div role="group" label={item.name} metadata={{ settingsLayout: "row" }}>
      <RowCopy
        title={
          item.recommended && item.state !== "installed"
            ? `${item.name} · Recommended`
            : item.name
        }
        detail={item.size}
      />
      <div metadata={{ settingsLayout: "line" }}>
        {item.state === "available" && (
          <Button
            label="Download"
            disabled={disabled}
            onClick={() => act("download")}
          />
        )}
        {item.state === "downloading" && (
          <>
            <UI.ProgressRow label={item.name} value={item.progress} />
            <Button
              label="Cancel"
              disabled={disabled}
              onClick={() => act("cancel")}
            />
          </>
        )}
        {item.state === "installed" && (
          <>
            {item.inUse ? (
              <Copy>In Use</Copy>
            ) : (
              <Button
                label="Use"
                disabled={disabled}
                onClick={() => act("use")}
              />
            )}
            <Button
              label="Delete…"
              disabled={disabled}
              onClick={() => act("remove")}
            />
          </>
        )}
        {item.state === "error" && (
          <>
            <div
              role="status"
              label={item.error ?? ""}
              metadata={{
                settingsLayout: "error",
                symbol: "exclamationmark.triangle.fill",
              }}
            >
              {item.error}
            </div>
            <Button
              label="Retry"
              disabled={disabled}
              onClick={() => act("retry")}
            />
          </>
        )}
      </div>
    </div>
  );
}
function Extensions({ model, bindings }: SettingsPanelProps) {
  const detail = model.detail;
  // Its page as every Settings page: named groups, each a card of rows.
  if (detail)
    return (
      // Its groups spaced as the page spaces its own.
      <div
        role="group"
        label={detail.name}
        metadata={{ settingsLayout: "section" }}
      >
        <div metadata={{ settingsLayout: "line" }}>
          <UI.PushButton
            label="Extensions"
            metadata={{ symbol: "chevron.left" }}
            onClick={() => model.closeDetail()}
          >
            Extensions
          </UI.PushButton>
        </div>
        <Group title={detail.name}>
          <div role="group" label="About" metadata={{ settingsLayout: "row" }}>
            <RowCopy title="About" detail={detail.about} />
          </div>
          {detail.includes.length > 0 && (
            <div
              role="group"
              label="Includes"
              metadata={{ settingsLayout: "row" }}
            >
              <RowCopy title="Includes" detail={detail.includes.join("\n")} />
            </div>
          )}
          {detail.shortcuts.length > 0 && (
            <div
              role="group"
              label="Keyboard Shortcuts"
              metadata={{ settingsLayout: "row" }}
            >
              <RowCopy
                title="Keyboard Shortcuts"
                detail={detail.shortcuts.join("\n")}
              />
            </div>
          )}
          <div role="group" label="Files" metadata={{ settingsLayout: "row" }}>
            <RowCopy title="Files" detail={detail.files.join("\n")} />
            <div metadata={{ settingsLayout: "line" }}>
              <Button
                label="Show in Finder"
                disabled={!model.services}
                onClick={() => {
                  void model.perform({
                    type: "extension.finder",
                    id: detail.id,
                  });
                }}
              />
            </div>
          </div>
        </Group>
        {detail.variables.length > 0 && (
          <Group title="Setup">
            {detail.variables.map((row) => (
              <SettingRow key={row.id} row={row} model={model} />
            ))}
          </Group>
        )}
        {detail.canArchive && (
          <Group title="Remove">
            <div
              role="group"
              label="Delete"
              metadata={{ settingsLayout: "row" }}
            >
              <RowCopy
                title="Delete Extension"
                detail="Its files move to the archive in your Powermove profile, and its settings are kept."
              />
              <div metadata={{ settingsLayout: "line" }}>
                <Button
                  label="Delete…"
                  destructive
                  disabled={!model.services || model.pending.has(detail.id)}
                  onClick={() => {
                    void model.perform({
                      type: "extension.archive",
                      id: detail.id,
                    });
                  }}
                />
              </div>
            </div>
          </Group>
        )}
      </div>
    );
  return (
    <>
      {/* The filter stands on the page's text column, over the list's card. */}
      <div metadata={{ settingsLayout: "line" }}>
        <div metadata={{ settingsLayout: "line" }}>
          <UI.TextField
            label="Filter extensions"
            binding={bindings.extensionSearch}
            value={model.extensionQuery}
            metadata={{ placeholder: "Filter", search: true }}
            onInput={(e) => {
              if (typeof e.value === "string") model.filterExtensions(e.value);
            }}
          />
        </div>
        <UI.PopUpButton
          label="Extension Type"
          binding={bindings.extensionType}
          value={model.extensionType}
          options={[...extensionTypes]}
          onChange={(e) => {
            if (
              extensionTypes.includes(
                e.value as (typeof extensionTypes)[number],
              )
            )
              model.filterExtensions(
                model.extensionQuery,
                e.value as (typeof extensionTypes)[number],
              );
          }}
        />
      </div>
      <Group title="Installed">
        <div metadata={{ settingsLayout: "column" }}>
          {model.publication.extensionsState === "loading" ? (
            <UI.ProgressRow label="Loading extensions…" />
          ) : model.publication.extensionsState === "error" ? (
            <UI.EmptyState
              label="Extensions couldn’t be loaded."
              explanation=""
              action={
                <Button
                  label="Try Again"
                  disabled={!model.services}
                  onClick={() => {
                    void model.perform({ type: "extensions.retry" });
                  }}
                />
              }
            />
          ) : model.publication.extensionsState === "unavailable" ? (
            <UI.EmptyState
              label="Extensions are unavailable in this session."
              explanation=""
              action={null}
            />
          ) : model.extensions.length === 0 &&
            (model.extensionQuery.trim() ||
              model.extensionType !== "All Types") ? (
            <UI.EmptyState
              label="No Results"
              explanation="No extensions match this filter."
              action={null}
            />
          ) : model.extensions.length === 0 ? (
            <UI.EmptyState
              label="No Extensions"
              explanation="Extensions you install appear here."
              action={null}
            />
          ) : (
            // API GAP: structural virtual-list is the existing wire primitive; the kit
            // has no VirtualList/source-list component or AXRowCount projection yet.
            <virtual-list
              id="settings.extensions"
              role="list"
              label="Extensions"
              count={model.extensions.length}
              range={model.range}
              metadata={{
                rowCount: model.extensions.length,
                // Its height (PM-386): every row up to forty, in the page
                // that scrolls them; past that a forty-row list scrolls
                // itself and shows only what is in view.
                rowFloor: Math.min(model.extensions.length, 40),
                rowHeightToken: "metrics.form_row",
                anchorId: model.anchorId ?? "",
              }}
              onRangeChange={(event) => {
                if (event.range)
                  model.viewport(
                    event.range[0],
                    event.range[1] - event.range[0],
                  );
              }}
            >
              {model.visibleExtensions.map((extension) => (
                <div
                  key={extension.id}
                  id={`settings.extension.${extension.id}`}
                  role="listitem"
                  label={extension.name}
                  metadata={{ settingsLayout: "row" }}
                >
                  <RowCopy title={extension.name} />
                  <div metadata={{ settingsLayout: "line" }}>
                    <Copy
                      secondary
                    >{`${extension.type} · ${extension.source}`}</Copy>
                    {extension.binding && (
                      <UI.Switch
                        label={extension.name}
                        binding={extension.binding}
                        value={extension.enabled}
                        disabled={
                          !model.services || model.pending.has(extension.id)
                        }
                        onChange={(e) => {
                          if (typeof e.value === "boolean")
                            void model.perform({
                              type: "extension.enabled",
                              id: extension.id,
                              enabled: e.value,
                            });
                        }}
                      />
                    )}
                    <Button
                      label="Details"
                      accessibilityLabel={`Details for ${extension.name}`}
                      onClick={() => model.viewExtension(extension.id)}
                    />
                  </div>
                </div>
              ))}
            </virtual-list>
          )}
        </div>
      </Group>
      <Group title="Store">
        <div
          role="group"
          label="Powermove Store"
          metadata={{ settingsLayout: "row" }}
        >
          <RowCopy
            title="Powermove Store"
            detail="Publish and manage your extensions on the Store."
          />
          <div metadata={{ settingsLayout: "line" }}>
            <Button
              label="Open Store"
              disabled={!model.services}
              onClick={() => {
                void model.perform({ type: "store.open" });
              }}
            />
          </div>
        </div>
      </Group>
    </>
  );
}
const memoryGroups = [
  ["goal", "Goal"],
  ["constraint", "Rules"],
  ["open", "Open Items"],
  ["decision", "Decisions"],
  ["rejected", "Rejected Ideas"],
  ["feedback", "Feedback"],
] as const;
function MemoryNotes({ model }: { model: SettingsModel }) {
  const memory = model.publication.memory;
  if (!memory) return null;
  const query = model.query.trim().toLocaleLowerCase();
  const matches = memory.entries.filter(
    (note) =>
      !query ||
      `${note.text} ${note.why ?? ""} ${note.kind}`
        .toLocaleLowerCase()
        .includes(query),
  );
  if (!memory.entries.length)
    return (
      <Group title="Notes">
        <div
          role="group"
          label="No Notes Yet"
          metadata={{ settingsLayout: "row" }}
        >
          <RowCopy
            title="No Notes Yet"
            detail="When an agent records a goal, rule or decision for this project, it will show up here."
          />
        </div>
      </Group>
    );
  return (
    <>
      {memoryGroups.map(([kind, title]) => {
        const notes = matches
          .filter((note) => note.kind === kind)
          .sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
        if (!notes.length) return null;
        return (
          <Group key={kind} title={title} muted={!memory.enabled}>
            {notes.map((note, index) => {
              const date =
                typeof note.updatedAt === "number" &&
                Number.isFinite(note.updatedAt)
                  ? new Date(note.updatedAt).toLocaleDateString(undefined, {
                      month: "short",
                      day: "numeric",
                      year: "numeric",
                    })
                  : "";
              const meta = `${note.author === "user" ? "You" : "Agent"}${date ? ` · ${date}` : ""}${note.resolved ? " · Resolved" : ""}`;
              return (
                <div
                  key={`${kind}.${index}`}
                  role="group"
                  label={note.text}
                  metadata={{ settingsLayout: "note" }}
                >
                  <text>{note.text}</text>
                  {note.why && <text>{note.why}</text>}
                  <text
                    metadata={{
                      settingsLayout: "secondary",
                      ...(note.resolved ? { symbol: "checkmark.circle" } : {}),
                    }}
                  >
                    {meta}
                  </text>
                </div>
              );
            })}
          </Group>
        );
      })}
    </>
  );
}
/** A search lists at most this many matching rows of a section, so that
 * one letter matching hundreds of commands stays one page the host takes;
 * the rest are counted under them. */
const SEARCH_ROWS = 40;
/** Shortcut rows per commit: a row is about 5 KB of ops, a commit at most
 * 1 MiB. */
const SHORTCUT_CHUNK = 60;
function More({ count, title }: { count: number; title: string }) {
  if (count <= 0) return null;
  return (
    <text role="status" metadata={{ settingsLayout: "secondary" }}>
      {`${count} more in ${title}. Keep typing to narrow the search.`}
    </text>
  );
}
/** Settings › Shortcuts' first card as the design has it: Your Changes
 * counts the commands changed, and Import and Export are one row. Other
 * groups pass through. */
function keysCard(
  rows: Row[],
  model: SettingsModel,
): (Row | { share: true })[] {
  const shared = rows.filter((r) => SHARE.includes(r.id));
  const changed =
    model.publication.shortcuts?.rows.filter((r) => r.changed).length ?? 0;
  const out: (Row | { share: true })[] = rows
    .filter((r) => !SHARE.includes(r.id))
    .map((r) =>
      r.id === "shortcuts.resetAll" && changed > 0
        ? {
            ...r,
            detail:
              `${changed} ${changed === 1 ? "command" : "commands"} changed. ${r.detail ?? ""}`.trim(),
          }
        : r,
    );
  if (shared.length) out.push({ share: true });
  return out;
}
const SHARE = ["shortcuts.import", "shortcuts.export"];
/** Import… and Export… in one row (the design's Share Shortcuts). */
function ShareRow({ model }: { model: SettingsModel; key?: string }) {
  const run = (action: string) => {
    void model.perform({ type: "row", id: action, action });
  };
  return (
    <div
      id="settings.row.shortcuts.share"
      role="group"
      label="Share Shortcuts"
      metadata={{ settingsLayout: "row" }}
    >
      <RowCopy
        title="Share Shortcuts"
        detail="Bring in keys from another Mac or your agent, or save them to a file."
      />
      <div metadata={{ settingsLayout: "line" }}>
        <Button
          label="Import…"
          disabled={!model.services}
          onClick={() => run("shortcuts.import")}
        />
        <Button
          label="Export…"
          disabled={!model.services}
          onClick={() => run("shortcuts.export")}
        />
      </div>
    </div>
  );
}
/** A shortcut's keys as keycaps (the build note's custom piece): each
 * modifier its own cap, then the key. */
function ShortcutKeys({
  row,
  onRecord,
}: {
  row: import("./model").ShortcutRow;
  onRecord: () => void;
}) {
  const keys = row.keys.join("  /  ");
  if (row.fixed)
    return (
      <text
        label={
          row.keys.length
            ? `${row.label}: macOS's key, not assignable`
            : `${row.label}: not assignable`
        }
        metadata={{ settingsLayout: "secondary" }}
      >
        {keys || "—"}
      </text>
    );
  return (
    <Button
      label={keys || "None"}
      accessibilityLabel={`Change the shortcut for ${row.label}`}
      onClick={onRecord}
    />
  );
}

function Recording({ onCancel }: { onCancel: () => void }) {
  return (
    <Button
      label="Type a shortcut…"
      accessibilityLabel="Type a shortcut. Escape cancels."
      onClick={onCancel}
    />
  );
}
/** Settings › Shortcuts: every command with its keys as keycaps, grouped by
 * its menu, filtered by Search Commands. Pressing a command's keys takes
 * the next key; a key another command has asks first ("press it again to
 * move it here"); Delete takes the key away, Escape keeps the old one. */
function Shortcuts({
  model,
  bindings,
}: {
  model: SettingsModel;
  bindings: SettingsPanelProps["bindings"];
}) {
  const [filter, setFilter] = useState("");
  const page = model.publication.shortcuts;
  const query = model.query.trim();
  const own = filter.trim().toLocaleLowerCase();
  const matching = (page?.rows ?? []).filter(
    (row) =>
      (!query || model.matches.rows.has(`shortcut.${row.id}`)) &&
      (!own ||
        `${row.label} ${row.menu} ${row.keys.join(" ")}`
          .toLocaleLowerCase()
          .includes(own)),
  );
  const all = query || own ? matching.slice(0, SEARCH_ROWS) : matching;
  // Every command at once is more than one commit may carry (the host's
  // argument quota): the rows arrive a chunk a turn, from the first again
  // whenever the search changes.
  const key = `${query}\u0000${own}`;
  const [shown, setShown] = useState({ key, limit: SHORTCUT_CHUNK });
  const limit = shown.key === key ? shown.limit : SHORTCUT_CHUNK;
  useLayoutEffect(() => {
    // A new search starts from the first chunk, and is remembered even when
    // it fits in one (else its end found the old search's count and sent
    // every row at once).
    if (shown.key !== key) {
      setShown({ key, limit: SHORTCUT_CHUNK });
      return;
    }
    if (limit >= all.length) return;
    const timer = setTimeout(
      () => setShown({ key, limit: limit + SHORTCUT_CHUNK }),
      0,
    );
    return () => clearTimeout(timer);
  }, [key, limit, all.length, shown.key]);
  if (!page) return null;
  const rows = all.slice(0, limit);
  const menus = [...new Set(rows.map((row) => row.menu))];
  const act = (op: "record" | "reset" | "clear", id: string) => {
    void model.perform({ type: "shortcut", op, id });
  };
  const disabled = (id: string) => !model.services || model.pending.has(id);
  const commandRow = (row: import("./model").ShortcutRow) => [
    <div
      key={row.id}
      id={`settings.shortcut.${row.id}`}
      role="group"
      label={row.label}
      metadata={{ settingsLayout: "row" }}
    >
      {/* The command's name takes a click too: it starts listening, as a
       * click on its keys does (the keys stay the keyboard's target). */}
      {row.fixed ? (
        <RowCopy title={row.label} />
      ) : (
        <UI.PushButton
          tabIndex={-1}
          label={row.label}
          metadata={{ settingsLayout: "nameButton" }}
          onClick={() => act("record", row.id)}
        >
          {row.label}
        </UI.PushButton>
      )}
      <div metadata={{ settingsLayout: "line" }}>
        {row.changed && !row.fixed && !row.recording ? (
          <Button
            key="reset"
            label="Reset"
            disabled={disabled(row.id)}
            onClick={() => act("reset", row.id)}
          />
        ) : null}
        {row.recording ? (
          <Recording onCancel={() => act("record", row.id)} />
        ) : (
          <ShortcutKeys row={row} onRecord={() => act("record", row.id)} />
        )}
      </div>
    </div>,
    row.recording && row.asked ? (
      <div
        key={`${row.id}.asked`}
        role="status"
        label="Shortcut in use"
        metadata={{
          settingsLayout: "error",
          symbol: "exclamationmark.triangle.fill",
        }}
      >
        {row.asked.menu
          ? `${row.asked.keys} belongs to ${row.asked.other} in the menu bar. Type another, or Esc to cancel.`
          : `${row.asked.keys} is used by ${row.asked.other}. Press it again to move it here, or Esc to cancel.`}
      </div>
    ) : null,
  ];
  return (
    <>
      {/* The first group's header carries Search Commands; it keeps one key
       * so the field stays put (and focused) while typing changes which menu
       * comes first, and it stays when nothing matches so the search can be
       * cleared. */}
      {(menus.length || !own ? menus : [null]).map((menu, i) => (
        <div
          key={i === 0 ? "first" : menu!}
          role="group"
          label={menu ?? "Search Commands"}
          metadata={{ settingsLayout: "group" }}
        >
          <div metadata={{ settingsLayout: "headingLine" }}>
            <text role="heading">{menu ?? "No Matching Commands"}</text>
            {i === 0 && bindings.commandSearch ? (
              <div>
                <UI.TextField
                  label="Search Commands"
                  binding={bindings.commandSearch}
                  value={filter}
                  metadata={{ placeholder: "Search Commands", search: true }}
                  onInput={(event) => {
                    if (typeof event.value === "string") setFilter(event.value);
                  }}
                />
              </div>
            ) : null}
          </div>
          {menu === null ? null : (
            <UI.FormGroup label={menu}>
              {rows.filter((row) => row.menu === menu).flatMap(commandRow)}
            </UI.FormGroup>
          )}
        </div>
      ))}
      {!query && !own && page.conflicts.length > 0 && (
        <Group title="Shared Keys">
          {page.conflicts.map((clash) => (
            <div
              key={clash.chord}
              id={`settings.shared.${clash.chord}`}
              role="group"
              label={clash.keys}
              metadata={{ settingsLayout: "row" }}
            >
              <RowCopy
                title={clash.keys}
                detail={`Tried in this order: ${clash.labels.join(", then ")}.`}
              />
            </div>
          ))}
        </Group>
      )}
      <More count={matching.length - rows.length} title="Shortcuts" />
    </>
  );
}
/** Settings › Project's first card: the poster, the project's name (a
 * picker of the projects open when there are several), its file and Show
 * in Finder. */
function ProjectHeader({ model, bindings, window }: SettingsPanelProps) {
  const project = model.publication.project;
  const header = model.publication.header;
  if (!project || !header) return null;
  const projects = header.projects ?? [];
  const nameRow = model.rows.find((row) => row.id === HEADER_ROW);
  return (
    <UI.FormGroup label="Project">
      <div metadata={{ settingsLayout: "header" }}>
        {header.poster ? (
          <img resource={header.poster} alt={project.name} />
        ) : (
          <div metadata={{ settingsLayout: "poster", symbol: "film" }} />
        )}
        <div metadata={{ settingsLayout: "column" }}>
          {projects.length > 1 && bindings.project ? (
            <div metadata={{ settingsLayout: "line" }}>
              <UI.PopUpButton
                label="Project"
                // The design's pull-down: the project's name, the menu of
                // open projects dropping below it.
                data-pm-variant="pulldown"
                binding={bindings.project}
                value={project.id}
                options={projects.map((p) => ({ v: p.id, label: p.name }))}
                onChange={(event) => {
                  if (typeof event.value === "string")
                    window?.project(event.value);
                }}
              />
            </div>
          ) : (
            <text>{project.name}</text>
          )}
          {header.path ? (
            <text metadata={{ settingsLayout: "path" }}>{header.path}</text>
          ) : null}
        </div>
        {header.path ? (
          <div metadata={{ settingsLayout: "line" }}>
            <Button label="Show in Finder" onClick={() => window?.reveal()} />
          </div>
        ) : null}
      </div>
      {nameRow ? (
        <SettingRow row={{ ...nameRow, detail: undefined }} model={model} />
      ) : null}
    </UI.FormGroup>
  );
}
function SectionContent({
  section,
  model,
  bindings,
  window,
}: SettingsPanelProps & { section: Section }) {
  const copy = sectionCopy[section];
  const matching = model.rows.filter(
    (row) =>
      row.section === section &&
      (!model.query.trim() || model.matches.rows.has(row.id)),
  );
  const searching = !!model.query.trim();
  // The project's name stands in its header card (not while searching,
  // where it is a match like any other row).
  const listed = matching.filter(
    (row) =>
      searching ||
      section !== "project" ||
      !model.publication.header ||
      row.id !== HEADER_ROW,
  );
  const rows = searching ? listed.slice(0, SEARCH_ROWS) : listed;
  const groups = [...new Set(rows.map((row) => row.group))];
  const speech = model.publication.speech.filter(
    (item) =>
      !model.query.trim() || model.matches.rows.has(`speech.${item.id}`),
  );
  return (
    <div
      id={`settings.section.${section}`}
      role="group"
      label={copy.title}
      metadata={{ settingsLayout: "section" }}
    >
      {/* The toolbar names the section; a search lists several, each
          under its name. */}
      {model.query.trim() ? <text role="heading">{copy.title}</text> : null}
      {section === "project" && !model.query.trim() ? (
        <ProjectHeader model={model} bindings={bindings} window={window} />
      ) : null}
      {groups.map((group) => (
        <Group key={group} title={group}>
          {keysCard(
            rows.filter((row) => row.group === group),
            model,
          ).map((row) =>
            "share" in row ? (
              <ShareRow key="share" model={model} />
            ) : (
              <SettingRow key={row.id} row={row} model={model} />
            ),
          )}
        </Group>
      ))}
      <More count={listed.length - rows.length} title={copy.title} />
      {section === "memory" && <MemoryNotes model={model} />}
      {section === "transcription" && speech.length > 0 && (
        <Group title="Models">
          {speech.map((item) => (
            <SpeechRow key={item.id} item={item} model={model} />
          ))}
        </Group>
      )}
      {section === "extensions" && (
        <Extensions model={model} bindings={bindings} />
      )}
      {section === "shortcuts" && (
        <Shortcuts model={model} bindings={bindings} />
      )}
    </div>
  );
}
export function SettingsPanel({ model, bindings, window }: SettingsPanelProps) {
  const [, update] = useState(0);
  useLayoutEffect(() => model.subscribe(() => update((n) => n + 1)), [model]);
  if (!model.open) return null;
  return (
    <div
      id="settings"
      role="group"
      label="Settings"
      focusScope
      metadata={{
        hostWindow: "settings",
        windowTitle: sectionCopy[model.active].title,
      }}
      onKeyDown={(event) => {
        event.stopPropagation();
        // Native cancellation must already have happened in the shell planner.
        // The frozen v4 contract forbids retroactive JS preventDefault().
        model.key(event);
      }}
    >
      <scroll
        id="settings.detail"
        role="group"
        label={sectionCopy[model.active].title}
      >
        {model.query.trim() && model.matches.count === 0 ? (
          <UI.EmptyState
            label="No Results"
            explanation="Try a different search."
          />
        ) : (
          (model.query.trim()
            ? model.visibleSections
            : model.visibleSections.filter(
                (section) => section === model.active,
              )
          ).map((section) => (
            <SectionContent
              key={section}
              section={section}
              model={model}
              bindings={bindings}
              window={window}
            />
          ))
        )}
      </scroll>
    </div>
  );
}
/** Mount as a v4 extension in a separate NSWindow; never dock this root. */
export const settingsExtension = Object.freeze({
  id: "settings",
  apiVersion: 4,
  component: SettingsPanel,
});
