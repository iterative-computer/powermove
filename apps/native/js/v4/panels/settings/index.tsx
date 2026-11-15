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
  };
  /** The header's picker chose project `id`; Show in Finder. */
  window?: { project(id: string): void; reveal(): void };
}
const t = UI.token;
const text = { color: t("colors.text"), fontSize: t("typography.body") };
const help = { color: t("colors.muted"), fontSize: t("typography.minimum") };
const column = { display: "flex", flexDirection: "column" };
function Copy({
  children,
  secondary = false,
}: {
  children: ComponentChildren;
  secondary?: boolean;
  key?: string | number;
}) {
  return <text style={secondary ? help : text}>{children}</text>;
}
function Button({
  label,
  disabled,
  onClick,
  destructive = false,
}: {
  label: string;
  destructive?: boolean;
  disabled?: boolean;
  onClick: () => void;
  key?: string;
}) {
  return (
    <UI.PushButton
      label={label}
      role={destructive ? "destructive" : "normal"}
      disabled={disabled}
      onClick={onClick}
    >
      {label}
    </UI.PushButton>
  );
}
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
  if (row.control === "colour" && typeof row.value === "string")
    return <UI.ColorWell {...common} minimal value={row.value} />;
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
      <div style={trailing}>
        <Copy secondary>{row.value}</Copy>
        {button}
      </div>
    );
  }
  if (row.control === "status")
    return <Copy>{typeof row.value === "string" ? row.value : ""}</Copy>;
  // An unknown authored value stays in the service; never coerce it into a default.
  if (!row.binding) return null;
  if (row.control === "number" && typeof row.value === "number")
    return (
      <UI.NumberField
        {...common}
        value={row.value}
        min={row.min}
        max={row.max}
        unit={row.unit}
      />
    );
  if (row.control === "text" && typeof row.value === "string")
    return <UI.TextField {...common} value={row.value} />;
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
/** A row of a card (the Settings build note): its title over its
 * description, the control trailing on the title's line. A single line is
 * `form_row` tall; a description adds its lines and the inset under them. */
const rowStyle = {
  display: "flex",
  alignItems: "start",
  justifyContent: "space-between",
  gap: t("metrics.inset"),
  minHeight: t("metrics.form_row"),
  paddingLeft: t("metrics.inset"),
  paddingRight: t("metrics.inset"),
  paddingTop: t("metrics.gap"),
  paddingBottom: t("metrics.gap"),
};
const describedRow = { ...rowStyle, paddingBottom: t("metrics.inset") };
/** The title's line: as tall as a control, so the control beside it is
 * centred on the title, not on the row. */
const titleLine = {
  display: "flex",
  alignItems: "center",
  minHeight: t("metrics.control"),
};
const trailing = {
  ...titleLine,
  flexShrink: 0,
  gap: t("metrics.gap"),
};
/** A row's words: they take what the control leaves and wrap in it. */
const copyColumn = {
  ...column,
  flexGrow: 1,
  flexShrink: 1,
  // It may be narrower than its longest line, which then wraps.
  minWidth: 0,
};
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
    <div style={copyColumn}>
      <div style={titleLine}>
        <text style={{ ...text, fontSize: t("typography.title") }}>
          {title}
        </text>
      </div>
      {detail && (
        <text style={{ ...help, fontSize: t("typography.minimum") }}>
          {detail}
        </text>
      )}
      {children}
    </div>
  );
}
/** The rows of one card, a hairline between each two (inset on both sides). */
function Rows({ children }: { children: ComponentChildren }) {
  const rows = (Array.isArray(children) ? children.flat() : [children]).filter(
    (row) => row !== null && row !== undefined && row !== false,
  );
  return (
    <>
      {rows.flatMap((row, i) =>
        i === 0
          ? [row]
          : [
              <div
                key={`separator.${i}`}
                style={{
                  height: t("metrics.border"),
                  marginLeft: t("metrics.inset"),
                  marginRight: t("metrics.inset"),
                  background: t("colors.separator"),
                }}
              />,
              row,
            ],
      )}
    </>
  );
}
/** A group of rows: its name over its card (13 pt medium, outside it). */
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
    <div role="group" label={title} style={column}>
      <text
        role="heading"
        style={{
          color: t(muted ? "colors.muted" : "colors.text"),
          fontSize: t("typography.title"),
          fontWeight: t("typography.medium_weight"),
          paddingLeft: t("metrics.inset"),
          marginBottom: t("metrics.dock_gap"),
        }}
      >
        {title}
      </text>
      <UI.FormGroup label={title}>
        <Rows>{children}</Rows>
      </UI.FormGroup>
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
      style={row.detail || row.error ? describedRow : rowStyle}
    >
      <RowCopy title={row.label} detail={row.detail}>
        {row.error && (
          <div
            role="status"
            label={row.error}
            metadata={{ symbol: "exclamationmark.triangle.fill" }}
            style={{
              color: t("colors.danger"),
              fontSize: t("typography.minimum"),
            }}
          >
            {row.error}
          </div>
        )}
      </RowCopy>
      <div style={trailing}>
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
    <div role="group" label={item.name} style={describedRow}>
      <RowCopy
        title={
          item.recommended && item.state !== "installed"
            ? `${item.name} · Recommended`
            : item.name
        }
        detail={item.size}
      />
      <div style={trailing}>
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
              metadata={{ symbol: "exclamationmark.triangle.fill" }}
              style={{ color: t("colors.danger") }}
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
  if (detail)
    return (
      <div role="group" label={detail.name}>
        <UI.ToolbarItem
          label="Extensions"
          metadata={{ symbol: "chevron.left" }}
          onClick={() => model.closeDetail()}
        >
          Extensions
        </UI.ToolbarItem>
        <Copy>{detail.name}</Copy>
        <div role="group" label="About">
          <text
            style={{ color: t("colors.text"), fontSize: t("typography.title") }}
          >
            About
          </text>
          <Copy>{detail.about}</Copy>
        </div>
        <div role="group" label="Includes">
          <text
            style={{ color: t("colors.text"), fontSize: t("typography.title") }}
          >
            Includes
          </text>
          {detail.includes.map((entry, i) => (
            <Copy key={i}>{entry}</Copy>
          ))}
        </div>
        <div role="group" label="Keyboard Shortcuts">
          <Copy>Keyboard Shortcuts</Copy>
          {detail.shortcuts.map((entry, i) => (
            <Copy key={i}>{entry}</Copy>
          ))}
        </div>
        <div role="group" label="Files">
          <Copy>Files</Copy>
          {detail.files.map((entry, i) => (
            <Copy key={i}>{entry}</Copy>
          ))}
          <Button
            label="Show in Finder"
            disabled={!model.services}
            onClick={() => {
              void model.perform({ type: "extension.finder", id: detail.id });
            }}
          />
        </div>
        {detail.variables.map((row) => (
          <SettingRow key={row.id} row={row} model={model} />
        ))}
        {detail.canArchive && (
          <Button
            label="Delete…"
            destructive
            disabled={!model.services || model.pending.has(detail.id)}
            onClick={() => {
              void model.perform({ type: "extension.archive", id: detail.id });
            }}
          />
        )}
      </div>
    );
  return (
    <>
      {/* The filter stands on the page's text column, over the list's card. */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: t("metrics.gap"),
          height: t("metrics.form_row"),
          paddingLeft: t("metrics.inset"),
          paddingRight: t("metrics.inset"),
        }}
      >
        <div style={{ display: "flex", flexGrow: 1, flexShrink: 1 }}>
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
        <div
          style={{
            ...column,
            paddingTop: t("metrics.inset"),
            paddingBottom: t("metrics.inset"),
          }}
        >
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
                rowHeightToken: "metrics.row",
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
                <UI.ListItem
                  key={extension.id}
                  id={`settings.extension.${extension.id}`}
                  label={extension.name}
                  selected={false}
                  onSelect={() => model.viewExtension(extension.id)}
                >
                  <Copy>{extension.name}</Copy>
                  <Copy
                    secondary
                  >{`${extension.type} · ${extension.source}`}</Copy>
                  {/* Activation service is unavailable; do not imply an acknowledged enable switch. */}
                </UI.ListItem>
              ))}
            </virtual-list>
          )}
        </div>
      </Group>
      <Group title="Store">
        <div role="group" label="Powermove Store" style={describedRow}>
          <RowCopy
            title="Powermove Store"
            detail="Publish and manage your extensions on the Store."
          />
          <div style={trailing}>
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
        <div role="group" label="No Notes Yet" style={describedRow}>
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
                  style={{
                    ...column,
                    minHeight: t("metrics.form_row"),
                    padding: t("metrics.inset"),
                  }}
                >
                  <text
                    style={{
                      fontSize: t("typography.body"),
                      color: t(memory.enabled ? "colors.text" : "colors.muted"),
                    }}
                  >
                    {note.text}
                  </text>
                  {note.why && (
                    <text
                      style={{
                        fontSize: t("typography.reading"),
                        color: t(
                          memory.enabled ? "colors.secondary" : "colors.muted",
                        ),
                      }}
                    >
                      {note.why}
                    </text>
                  )}
                  <text
                    metadata={
                      note.resolved ? { symbol: "checkmark.circle" } : {}
                    }
                    style={help}
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
function More({ count, title }: { count: number; title: string }) {
  if (count <= 0) return null;
  return (
    <text role="status" style={{ ...help, paddingLeft: t("metrics.inset") }}>
      {`${count} more in ${title}. Keep typing to narrow the search.`}
    </text>
  );
}
/** Settings › Shortcuts: every command with its keys, grouped by its menu;
 * Change… takes the next key pressed (Escape cancels), and keys two
 * commands share are listed with the order they are tried in. */
function Shortcuts({ model }: { model: SettingsModel }) {
  const page = model.publication.shortcuts;
  if (!page) return null;
  const query = model.query.trim();
  const matching = page.rows.filter(
    (row) => !query || model.matches.rows.has(`shortcut.${row.id}`),
  );
  const rows = query ? matching.slice(0, SEARCH_ROWS) : matching;
  const menus = [...new Set(rows.map((row) => row.menu))];
  const shared = new Map(
    page.conflicts.flatMap((c) => c.labels.map((label) => [label, c])),
  );
  const act = (op: "record" | "reset" | "clear", id: string) => {
    void model.perform({ type: "shortcut", op, id });
  };
  const disabled = (id: string) => !model.services || model.pending.has(id);
  return (
    <>
      {menus.map((menu) => (
        <Group key={menu} title={menu}>
          {rows
            .filter((row) => row.menu === menu)
            .map((row) => {
              const clash = shared.get(row.label);
              const keys = row.keys.length ? row.keys.join("  ") : "No key";
              const detail = row.recording
                ? "Press the new key. Escape keeps the old one."
                : row.fixed
                  ? `${keys} · The menu bar's key.`
                  : clash
                    ? `${keys} · ${clash.keys} also: ${clash.labels
                        .filter((label) => label !== row.label)
                        .join(", ")}`
                    : keys;
              return (
                <div
                  key={row.id}
                  id={`settings.shortcut.${row.id}`}
                  role="group"
                  label={row.label}
                  style={describedRow}
                >
                  <RowCopy title={row.label} detail={detail} />
                  {!row.fixed && (
                    <div style={trailing}>
                      {row.changed && (
                        <Button
                          key="reset"
                          label="Reset"
                          disabled={disabled(row.id)}
                          onClick={() => act("reset", row.id)}
                        />
                      )}
                      {row.keys.length > 0 && !row.recording && (
                        <Button
                          key="clear"
                          label="Remove"
                          disabled={disabled(row.id)}
                          onClick={() => act("clear", row.id)}
                        />
                      )}
                      <Button
                        key={row.recording ? "listening" : "change"}
                        label={row.recording ? "Cancel" : "Change…"}
                        disabled={disabled(row.id)}
                        onClick={() => act("record", row.id)}
                      />
                    </div>
                  )}
                </div>
              );
            })}
        </Group>
      ))}
      {!query && page.conflicts.length > 0 && (
        <Group title="Shared Keys">
          {page.conflicts.map((clash) => (
            <div
              key={clash.chord}
              id={`settings.shared.${clash.chord}`}
              role="group"
              label={clash.keys}
              style={describedRow}
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
  const name = {
    ...text,
    fontSize: t("typography.heading"),
    fontWeight: t("typography.semibold_weight"),
  };
  return (
    <UI.FormGroup label="Project">
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: t("metrics.inset"),
          padding: t("metrics.inset"),
        }}
      >
        <div
          style={{
            position: "relative",
            flexShrink: 0,
            width: t("metrics.settings_poster_width"),
            height: t("metrics.settings_poster_height"),
            borderRadius: t("metrics.inner_radius"),
            overflow: "hidden",
            background: t("colors.field"),
          }}
        >
          {header.poster ? (
            <img
              resource={header.poster}
              alt={project.name}
              style={{ position: "absolute", inset: "fill" }}
            />
          ) : null}
        </div>
        <div style={copyColumn}>
          {projects.length > 1 && bindings.project ? (
            <div style={{ display: "flex" }}>
              <UI.PopUpButton
                label="Project"
                data-pm-variant="inline"
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
            <text style={name}>{project.name}</text>
          )}
          {header.path ? (
            <text
              style={{
                ...help,
                fontSize: t("typography.minimum"),
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "middle",
              }}
            >
              {header.path}
            </text>
          ) : null}
        </div>
        {header.path ? (
          <div style={trailing}>
            <Button label="Show in Finder" onClick={() => window?.reveal()} />
          </div>
        ) : null}
      </div>
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
  const rows = model.query.trim() ? matching.slice(0, SEARCH_ROWS) : matching;
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
      style={{
        ...column,
        paddingTop: t("metrics.dock_gap"),
        paddingLeft: t("metrics.sheet_inset"),
        paddingRight: t("metrics.sheet_inset"),
        paddingBottom: t("metrics.sheet_inset"),
        gap: t("metrics.sheet_inset"),
      }}
    >
      {/* The toolbar names the section; a search lists several, each
          under its name. */}
      {model.query.trim() ? (
        <text
          role="heading"
          style={{
            fontSize: t("typography.title"),
            fontWeight: t("typography.semibold_weight"),
            color: t("colors.text"),
            paddingLeft: t("metrics.inset"),
          }}
        >
          {copy.title}
        </text>
      ) : null}
      {section === "project" && !model.query.trim() ? (
        <ProjectHeader model={model} bindings={bindings} window={window} />
      ) : null}
      {groups.map((group) => (
        <Group key={group} title={group}>
          {rows
            .filter((row) => row.group === group)
            .map((row) => (
              <SettingRow key={row.id} row={row} model={model} />
            ))}
        </Group>
      ))}
      <More count={matching.length - rows.length} title={copy.title} />
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
      {section === "shortcuts" && <Shortcuts model={model} />}
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
        windowTitle: sectionCopy[model.active].title,
        hostWindow: "settings",
        resize: "height",
        widthToken: "metrics.settings_width",
        heightToken: "metrics.settings_height",
      }}
      // The page of the Settings window: AppKit's sidebar, search field and
      // toolbar (which names the section) frame it (PM-375).
      style={{ display: "flex" }}
      onKeyDown={(event) => {
        event.stopPropagation();
        // Native cancellation must already have happened in the shell planner.
        // The frozen v4 contract forbids retroactive JS preventDefault().
        model.key(event);
      }}
    >
      <div style={{ display: "flex", flexGrow: 1, flexShrink: 1 }}>
        <scroll
          id="settings.detail"
          role="group"
          label={sectionCopy[model.active].title}
          // A column: the page is as wide as the window leaves it.
          style={{ ...column, flexGrow: 1, flexShrink: 1, minWidth: 0 }}
        >
          {model.query.trim() && model.matches.count === 0 ? (
            // settings.md §5: centred reading text, not an empty-state title.
            <div
              role="status"
              label="No settings found."
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <text
                style={{
                  fontSize: t("typography.reading"),
                  color: t("colors.secondary"),
                }}
              >
                No settings found.
              </text>
            </div>
          ) : (
            // A readable form column centred in the sheet, as the original
            // app's Settings: however wide the window, labels and their
            // controls stay close enough to read as pairs.
            // Its natural height (never shrunk to the viewport): the scroll
            // scrolls it, and a list inside keeps the rows it shows.
            <div
              style={{
                display: "flex",
                justifyContent: "center",
                flexShrink: 0,
              }}
            >
              <div
                style={{
                  ...column,
                  flexGrow: 1,
                  flexShrink: 1,
                  minWidth: 0,
                  maxWidth: t("metrics.settings_column"),
                }}
              >
                {/* One section per page, as v1 and System Settings; a search
                    lists every section with a match. */}
                {(model.query.trim()
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
                ))}
              </div>
            </div>
          )}
        </scroll>
      </div>
    </div>
  );
}
/** Mount as a v4 extension in a separate NSWindow; never dock this root. */
export const settingsExtension = Object.freeze({
  id: "settings",
  apiVersion: 4,
  component: SettingsPanel,
});
