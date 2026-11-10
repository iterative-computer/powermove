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
  bindings: { search: string; extensionSearch: string; extensionType: string };
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
  const disabled = row.disabled || row.busy || model.pending.has(row.id);
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
  if (row.control === "action")
    return (
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
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: t("metrics.inset"),
        minHeight: t("metrics.form_row"),
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.inset"),
        paddingTop: t("metrics.gap"),
        paddingBottom: t("metrics.gap"),
      }}
    >
      {/* Title over its description; the control trails, centred on them. */}
      <div style={{ ...column, flexGrow: 1, gap: t("metrics.border") }}>
        <text style={{ ...text, fontSize: t("typography.title") }}>
          {row.label}
        </text>
        {row.detail && (
          <text style={{ ...help, fontSize: t("typography.label") }}>
            {row.detail}
          </text>
        )}
        {row.error && (
          <div
            role="status"
            label={row.error}
            metadata={{ symbol: "exclamationmark.triangle.fill" }}
            style={{
              color: t("colors.danger"),
              fontSize: t("typography.label"),
            }}
          >
            {row.error}
          </div>
        )}
      </div>
      <div style={{ display: "flex", flexShrink: 0, alignItems: "center" }}>
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
  const act = (
    action: "download" | "cancel" | "remove" | "retry" | "use",
  ) => {
    void model.perform({ type: "speech", id: item.id, action });
  };
  const disabled = !model.services || model.pending.has(item.id);
  return (
    <div role="group" label={item.name}>
      <Copy>
        {item.recommended && item.state !== "installed"
          ? `${item.name} · Recommended`
          : item.name}
      </Copy>
      <Copy secondary>{item.size}</Copy>
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
      <UI.TextField
        label="Filter extensions"
        binding={bindings.extensionSearch}
        value={model.extensionQuery}
        onInput={(e) => {
          if (typeof e.value === "string") model.filterExtensions(e.value);
        }}
      />
      <UI.PopUpButton
        label="Extension Type"
        binding={bindings.extensionType}
        value={model.extensionType}
        options={[...extensionTypes]}
        onChange={(e) => {
          if (
            extensionTypes.includes(e.value as (typeof extensionTypes)[number])
          )
            model.filterExtensions(
              model.extensionQuery,
              e.value as (typeof extensionTypes)[number],
            );
        }}
      />
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
        (model.extensionQuery.trim() || model.extensionType !== "All Types") ? (
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
            rowHeightToken: "metrics.row",
            anchorId: model.anchorId ?? "",
          }}
          onRangeChange={(event) => {
            if (event.range)
              model.viewport(event.range[0], event.range[1] - event.range[0]);
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
              <Copy secondary>{`${extension.type} · ${extension.source}`}</Copy>
              {/* Activation service is unavailable; do not imply an acknowledged enable switch. */}
            </UI.ListItem>
          ))}
        </virtual-list>
      )}
      <Copy>Publish and manage your extensions on the Store.</Copy>
      <Button
        label="Open Store"
        disabled={!model.services}
        onClick={() => {
          void model.perform({ type: "store.open" });
        }}
      />
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
      <div role="group" label="No Notes Yet" style={column}>
        <Copy>No Notes Yet</Copy>
        <Copy secondary>
          When an agent records a goal, rule or decision for this project, it
          will show up here.
        </Copy>
      </div>
    );
  return (
    <>
      {memoryGroups.map(([kind, title]) => {
        const notes = matches
          .filter((note) => note.kind === kind)
          .sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
        if (!notes.length) return null;
        return (
          <UI.FormGroup key={kind} title={title} muted={!memory.enabled}>
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
          </UI.FormGroup>
        );
      })}
    </>
  );
}
function SectionContent({
  section,
  model,
  bindings,
}: SettingsPanelProps & { section: Section }) {
  const copy = sectionCopy[section];
  const rows = model.rows.filter(
    (row) =>
      row.section === section &&
      (!model.query.trim() || model.matches.rows.has(row.id)),
  );
  const groups = [...new Set(rows.map((row) => row.group))];
  return (
    <div
      id={`settings.section.${section}`}
      role="group"
      label={copy.title}
      style={{
        ...column,
        padding: t("metrics.sheet_inset"),
        gap: t("metrics.section_gap"),
      }}
    >
      <div
        style={{
          ...column,
          gap: t("metrics.border"),
          paddingLeft: t("metrics.inset"),
        }}
      >
        <text
          role="heading"
          style={{
            fontSize: t("typography.display"),
            fontWeight: t("typography.semibold_weight"),
            color: t("colors.text"),
          }}
        >
          {copy.title}
        </text>
        <text
          style={{
            fontSize: t("typography.reading"),
            color: t("colors.muted"),
          }}
        >
          {section === "project"
            ? `Settings for “${model.publication.project!.name}”.`
            : copy.description}
        </text>
      </div>
      {groups.map((group) => (
        <UI.FormGroup key={group} title={group}>
          {rows
            .filter((row) => row.group === group)
            .map((row) => (
              <SettingRow key={row.id} row={row} model={model} />
            ))}
        </UI.FormGroup>
      ))}
      {section === "memory" && <MemoryNotes model={model} />}
      {section === "transcription" &&
        model.publication.speech
          .filter(
            (item) =>
              !model.query.trim() ||
              model.matches.rows.has(`speech.${item.id}`),
          )
          .map((item) => <SpeechRow key={item.id} item={item} model={model} />)}
      {section === "extensions" && (
        <Extensions model={model} bindings={bindings} />
      )}
    </div>
  );
}
export function SettingsPanel({ model, bindings }: SettingsPanelProps) {
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
      style={{ display: "flex", background: t("colors.panel") }}
      onKeyDown={(event) => {
        event.stopPropagation();
        // Native cancellation must already have happened in the shell planner.
        // The frozen v4 contract forbids retroactive JS preventDefault().
        model.key(event);
      }}
    >
      <div
        role="group"
        label="Settings sections"
        style={{
          ...column,
          width: t("metrics.settings_sidebar"),
          padding: t("metrics.gap"),
          gap: t("metrics.gap"),
        }}
      >
        <UI.TextField
          id="settings.search"
          label="Search"
          binding={bindings.search}
          value={model.query}
          metadata={{ searching: model.searching, placeholder: "Search" }}
          onInput={(event) => {
            if (typeof event.value === "string") void model.search(event.value);
          }}
        />
        <UI.List
          label="Settings sections"
          id="settings.sections"
          style={{ minWidth: t("metrics.hit"), minHeight: t("metrics.hit") }}
          tabIndex={0}
          preventDefault={{ keydown: ["ArrowUp", "ArrowDown", "Home", "End"] }}
          onKeyDown={(event) => model.navigate(event.key ?? "")}
        >
          {(["Project", "App"] as const)
            .filter((group) => group === "App" || !!model.publication.project)
            .map((group) => (
              <div key={group} role="group" label={group}>
                {(group !== "Project" || !!model.publication.project) && (
                  <text
                    style={{
                      ...help,
                      paddingLeft: t("metrics.inset"),
                      paddingTop: t("metrics.gap"),
                    }}
                  >
                    {group}
                  </text>
                )}
                {model.availableSections
                  .filter(
                    (section) =>
                      ["project", "memory"].includes(section) ===
                      (group === "Project"),
                  )
                  .map((section) => (
                    <UI.ListItem
                      key={section}
                      id={`settings.nav.${section}`}
                      label={sectionCopy[section].title}
                      selected={model.active === section}
                      tabIndex={-1}
                      metadata={{ symbol: sectionCopy[section].symbol }}
                      style={{
                        // The neutral on state, as tabs and segments.
                        ...(model.active === section
                          ? { background: t("colors.fill_strong") }
                          : {}),
                        ...(model.query.trim() &&
                        !model.matches.sections.has(section)
                          ? { color: t("colors.muted") }
                          : {}),
                      }}
                      onSelect={() => model.select(section)}
                    />
                  ))}
              </div>
            ))}
        </UI.List>
      </div>
      <scroll
        id="settings.detail"
        role="group"
        label={sectionCopy[model.active].title}
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
          // One section per page, as v1 and System Settings; a search lists
          // every section with a match.
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
