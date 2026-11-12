import { useLayoutEffect, useReducer } from "preact/hooks";
import {
  EmptyState,
  PopUpButton,
  TextField,
  ToolbarItem,
  Tree,
  TreeItem,
  token,
} from "../../src/components";
import { ids, labelParts, type EffectsModel, type Row } from "./model";

export interface EffectsPanelProps {
  model: EffectsModel;
  /** Host-owned local UI bindings. See README API gaps; never bind document channels. */
  bindings: { search: string; scope: string };
  increaseContrast?: boolean;
}
const textStyle = {
  color: token("colors.text"),
  fontSize: token("typography.body"),
};
// Group labels are quiet captions above their rows (no-decorative-chrome rule).
const groupTextStyle = {
  color: token("colors.muted"),
  fontSize: token("typography.minimum"),
};
const hitStyle = {
  minWidth: token("metrics.hit"),
  minHeight: token("metrics.hit"),
};
export function EffectsPanel({
  model,
  bindings,
  increaseContrast = false,
}: EffectsPanelProps) {
  const [, redraw] = useReducer((n: number) => n + 1, 0);
  useLayoutEffect(() => model.subscribe(() => redraw(undefined)), [model]);
  const window = model.window();
  const active = model.scopes().find((scope) => scope.value === model.scope)!;
  const focus = (row: Row) =>
    model.focus === "list" && model.focused === row.key;
  // Design review 2026-11-01: presets speak their kind like effects do.
  const itemLabel = (row: Extract<Row, { type: "item" }>) =>
    `${row.item.label}, ${row.item.kind === "effect" ? "effect" : "preset"}, ${row.item.group}`;
  return (
    <div
      id={ids.panel}
      role="group"
      label="Effects"
      focusRequest={model.focusRequest}
      style={{
        background: token("colors.panel"),
        borderRadius: token("metrics.panel_radius"),
        ...(increaseContrast
          ? {
              borderWidth: token("metrics.border"),
              borderColor: token("colors.separator"),
            }
          : {}),
      }}
    >
      <text
        role="heading"
        data-pm-slot="header"
        style={{
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          height: token("metrics.header"),
          paddingLeft: token("metrics.inset"),
          fontSize: token("typography.title"),
          fontWeight: token("typography.semibold_weight"),
          color: token("colors.text"),
        }}
      >
        Effects
      </text>
      <div
        style={{
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          height: token("metrics.row"),
          paddingLeft: token("metrics.inset"),
          paddingRight: token("metrics.inset"),
          gap: token("metrics.gap"),
        }}
      >
        <TextField
          id={ids.search}
          label="Search effects"
          title="Search effects"
          metadata={{ placeholder: "Search", search: true }}
          binding={bindings.search}
          value={model.query}
          disabled={model.installedCount() === 0}
          tabIndex={0}
          onFocus={() => model.setFocus("search")}
          onInput={(event) => {
            if (typeof event.value === "string")
              void model.setQuery(event.value);
          }}
          preventDefault={{ keydown: ["Escape", "ArrowDown"] }}
          onKeyDown={(event) => {
            void model.key(event.key ?? "", event);
          }}
        />
        <PopUpButton
          id={ids.scope}
          label={`Category, ${active.label}`}
          title="Show Category"
          binding={bindings.scope}
          value={model.scope}
          disabled={model.installedCount() === 0}
          options={model
            .scopes()
            .map((scope) => ({ v: scope.value, label: scope.label }))}
          tabIndex={0}
          style={{
            color: token("colors.secondary"),
            background: token("colors.panel"),
            maxWidth: token("metrics.value_column"),
          }}
          onFocus={() => model.setFocus("scope")}
          onChange={(event) => {
            const value = event.value;
            const option = model
              .scopes()
              .find((scope) => scope.value === value);
            if (option) model.pickScope(option.value);
          }}
        />
      </div>
      {model.error ? (
        <text
          role="status"
          label={model.error}
          metadata={{ symbol: "exclamationmark.triangle.fill" }}
          style={{
            color: token("colors.warning"),
            fontSize: token("typography.body"),
          }}
        >
          {model.error}
        </text>
      ) : null}
      {model.installedCount() === 0 ? (
        <EmptyState
          label="No Effects"
          explanation="Installed effects and presets appear here."
          attr={{ symbol: "wand.and.rays" }}
          action={null}
        />
      ) : window.count === 0 && !model.searching ? (
        <text
          style={{
            color: token("colors.secondary"),
            fontSize: token("typography.reading"),
            textAlign: "center",
          }}
        >
          {model.query.trim()
            ? `No effects match “${model.query}”`
            : "Nothing here yet"}
        </text>
      ) : (
        <Tree
          id={ids.list}
          label="Effects"
          tabIndex={0}
          style={hitStyle}
          metadata={{
            rowCount: window.count,
            range: [window.from, window.to],
            acceptsDrops: false,
            selectionMode: "single",
          }}
          onFocus={() => model.setFocus("list")}
          onPointerMove={(event) => {
            if (event.target === event.currentTarget) model.hover(null);
          }}
          preventDefault={{
            keydown: [
              "ArrowUp",
              "ArrowDown",
              "ArrowLeft",
              "ArrowRight",
              "Home",
              "End",
              "Enter",
              "Escape",
            ],
          }}
          onKeyDown={(event) => {
            void model.key(event.key ?? "", event);
          }}
        >
          {window.rows.map((row) => {
            const selected =
              row.type === "item" && model.selected === row.item.id;
            const hovered = model.hovered === row.key;
            const focused = focus(row);
            const style = {
              ...textStyle,
              borderRadius: token("metrics.control_radius"),
              ...(selected
                ? { background: token("colors.selected_fill") }
                : hovered
                  ? { background: token("colors.hover") }
                  : {}),
              ...(increaseContrast && selected
                ? {
                    borderWidth: token("metrics.border"),
                    borderColor: token("colors.separator"),
                  }
                : {}),
            };
            if (row.type === "group")
              return (
                <TreeItem
                  key={row.key}
                  id={ids.row(row.key)}
                  label={`${row.name}, ${row.count} effects, ${row.expanded ? "expanded" : "collapsed"}`}
                  tabIndex={-1}
                  expanded={row.expanded}
                  onSelect={() => model.toggle(row.key)}
                  onPointerMove={() => model.hover(row.key)}
                  onFocus={() => {
                    model.focused = row.key;
                    model.setFocus("list");
                  }}
                  style={{
                    ...style,
                    ...(increaseContrast &&
                    window.from + window.rows.indexOf(row) > 0
                      ? {
                          borderTopWidth: token("metrics.border"),
                          borderTopColor: token("colors.separator"),
                        }
                      : {}),
                  }}
                >
                  <text style={groupTextStyle}>{row.name}</text>
                </TreeItem>
              );
            const { item } = row;
            const parts = labelParts(item.label, model.query);
            const showAdd = selected || hovered || focused;
            return (
              <TreeItem
                key={row.key}
                id={ids.row(row.key)}
                label={itemLabel(row)}
                selected={selected}
                title={
                  item.failed
                    ? "This effect couldn’t load."
                    : "Drag onto a layer, or click Add."
                }
                tabIndex={-1}
                onSelect={() => model.select(row.key)}
                onDoubleClick={() => {
                  void model.apply(item.id);
                }}
                onPointerMove={() => model.hover(row.key)}
                onFocus={() => {
                  model.focused = row.key;
                  model.setFocus("list");
                }}
                style={{
                  ...style,
                  display: "flex",
                  flexDirection: "row",
                  alignItems: "center",
                  justifyContent: "space-between",
                }}
                metadata={{
                  item: item.id,
                  kind: item.kind,
                  parent: row.parent,
                  focused,
                  draggable: !item.failed,
                  definitionFailed: !!item.failed,
                }}
              >
                {/* Symbols, native drag capsules and focus rings need kit APIs; no glyph substitute. */}
                <text
                  title={item.label}
                  style={{
                    ...textStyle,
                    ...(item.failed ? { color: token("colors.muted") } : {}),
                    // The name's parts (before, match, after) run on one line.
                    display: "flex",
                    flexDirection: "row",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {parts.before}
                  <text
                    style={{ fontWeight: token("typography.semibold_weight") }}
                  >
                    {parts.match}
                  </text>
                  {parts.after}
                </text>
                <div
                  style={{
                    display: "flex",
                    flexDirection: "row",
                    alignItems: "center",
                    gap: token("metrics.gap"),
                  }}
                >
                  {item.failed && (
                    <text
                      attr={{ symbol: "exclamationmark.triangle.fill" }}
                      style={{
                        color: token("colors.warning"),
                        fontSize: token("typography.body"),
                      }}
                    />
                  )}
                  {model.query.trim() && (
                    <text
                      style={{
                        fontSize: token("typography.minimum"),
                        color: token("colors.muted"),
                      }}
                    >
                      {item.group}
                    </text>
                  )}
                  {showAdd && !item.failed && (
                    <ToolbarItem
                      id={ids.add(row.key)}
                      label={`Add ${item.label}`}
                      title={
                        model.target
                          ? `Add ${item.label}`
                          : "Select a layer first"
                      }
                      disabled={!model.target}
                      tabIndex={-1}
                      style={{
                        width: token("metrics.hit"),
                        height: token("metrics.hit"),
                        color: token(
                          model.target ? "colors.secondary" : "colors.muted",
                        ),
                      }}
                      onDoubleClick={(event) => event.stopPropagation()}
                      onClick={(event) => {
                        event.stopPropagation();
                        if ((event.clickCount ?? 1) <= 1)
                          void model.apply(item.id);
                      }}
                    />
                  )}
                </div>
              </TreeItem>
            );
          })}
        </Tree>
      )}
    </div>
  );
}
