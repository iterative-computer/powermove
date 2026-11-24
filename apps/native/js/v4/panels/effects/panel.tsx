import { useLayoutEffect, useReducer } from "preact/hooks";
import {
  EmptyState,
  PopUpButton,
  PushButton,
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
const groupTextStyle = {
  color: token("colors.text"),
  fontSize: token("typography.body"),
  fontWeight: token("typography.semibold_weight"),
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};
const countTextStyle = {
  color: token("colors.muted"),
  fontSize: token("typography.minimum"),
  fontVariantNumeric: "tabular-nums",
  flexShrink: 0,
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
  const scopes = model.scopes();
  const active =
    scopes.find((scope) => scope.value === model.scope) ?? scopes[0];
  const scopeOptions = scopes.flatMap((scope, index) => {
    const firstGroup =
      scope.value.startsWith("group:") &&
      index > 0 &&
      !scopes[index - 1].value.startsWith("group:");
    return [
      ...(firstGroup ? [{ separator: true as const }] : []),
      {
        v: scope.value,
        label: scope.label,
        checked: scope.value === model.scope,
      },
    ];
  });
  const status = model.error ?? model.selectionHint;
  const focus = (row: Row) =>
    model.focus === "list" && model.focused === row.key;
  // Speak each registry kind, including transitions, in the row label.
  const itemLabel = (row: Extract<Row, { type: "item" }>) =>
    `${row.item.label}, ${row.item.kind}, ${row.item.group}`;
  return (
    <div
      id={ids.panel}
      role="group"
      label="Effects"
      focusRequest={model.focusRequest}
      style={{
        display: "flex",
        flexDirection: "column",
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
          metadata={{ placeholder: "Search effects and presets", search: true }}
          attr={{ symbol: "magnifyingglass" }}
          binding={bindings.search}
          value={model.query}
          disabled={model.installedCount() === 0}
          tabIndex={0}
          style={{ flexGrow: 1 }}
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
          data-pm-variant="quiet"
          label={`Category, ${active.label}`}
          title="Show Category"
          binding={bindings.scope}
          value={model.scope}
          disabled={model.installedCount() === 0}
          options={scopeOptions}
          tabIndex={0}
          style={{
            flexShrink: 0,
            color: token("colors.secondary"),
            background: token("colors.panel"),
            maxWidth: token("metrics.value_column"),
          }}
          onFocus={() => model.setFocus("scope")}
          onChange={(event) => {
            const value = event.value;
            const option = scopes.find((scope) => scope.value === value);
            if (option) model.pickScope(option.value);
          }}
        />
      </div>
      {model.deletedPresetName ? (
        <div
          role="status"
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            padding: token("metrics.inset"),
            background: token("colors.field"),
            borderRadius: token("metrics.control_radius"),
            gap: token("metrics.gap"),
          }}
        >
          <text>Deleted “{model.deletedPresetName}”</text>
          <PushButton
            label="Undo delete preset"
            style={{ borderRadius: token("radius.xs") }}
            onClick={() => void model.presetAction?.("preset:", "undo")}
          >
            Undo
          </PushButton>
        </div>
      ) : null}
      {status ? (
        <text
          role="status"
          label={status}
          metadata={{ symbol: "exclamationmark.triangle.fill" }}
          style={{
            color: token("colors.warning"),
            fontSize: token("typography.body"),
          }}
        >
          {status}
        </text>
      ) : null}
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          flexGrow: 1,
          minWidth: 0,
          minHeight: 0,
        }}
      >
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
                borderRadius: token("metrics.inner_radius"),
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
                      display: "flex",
                      flexDirection: "row",
                      alignItems: "center",
                      justifyContent: "space-between",
                      ...(increaseContrast &&
                      window.from + window.rows.indexOf(row) > 0
                        ? {
                            borderTopWidth: token("metrics.border"),
                            borderTopColor: token("colors.separator"),
                          }
                        : {}),
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        flexDirection: "row",
                        alignItems: "center",
                        gap: token("metrics.gap"),
                        flexGrow: 1,
                        minWidth: 0,
                      }}
                    >
                      <text
                        attr={{
                          symbol: row.expanded
                            ? "chevron.down"
                            : "chevron.right",
                        }}
                        style={{
                          color: token("colors.muted"),
                          symbolSize: token("metrics.symbol_inline"),
                          flexShrink: 0,
                        }}
                      />
                      <text style={groupTextStyle}>{row.name}</text>
                    </div>
                    <text style={countTextStyle}>{String(row.count)}</text>
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
                      : "Drag onto a layer, or double-click to apply to the selected layer."
                  }
                  tabIndex={-1}
                  onSelect={() => model.select(row.key)}
                  onDoubleClick={() => {
                    void model.apply(item.id);
                  }}
                  // Lifts onto a layer's row or the viewer (the host starts the
                  // drag; `metadata.item` names the effect). A definition that
                  // failed to load doesn't lift.
                  onDragStart={item.failed ? undefined : () => {}}
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
                    ...(row.parent !== null
                      ? { paddingLeft: token("metrics.tree_indent") }
                      : {}),
                  }}
                  metadata={{
                    item: item.id,
                    dragLabel: item.label,
                    kind: item.kind,
                    parent: row.parent,
                    focused,
                    draggable: !item.failed,
                    definitionFailed: !!item.failed,
                  }}
                >
                  <div
                    style={{
                      display: "flex",
                      flexDirection: "row",
                      alignItems: "center",
                      gap: token("metrics.gap"),
                      flexGrow: 1,
                      minWidth: 0,
                    }}
                  >
                    {item.kind === "preset" ? (
                      <ToolbarItem
                        label="Animation preset"
                        disabled
                        attr={{ symbol: "square.stack" }}
                      />
                    ) : null}
                    {item.kind === "preset" &&
                    model.renamingPreset === item.id ? (
                      <TextField
                        label="Preset name"
                        binding={model.presetRenameBinding}
                        value={item.label}
                        onChange={(e) =>
                          void model.presetAction?.(
                            item.id,
                            "rename",
                            String(e.value ?? ""),
                          )
                        }
                      />
                    ) : (
                      <text
                        title={item.label}
                        style={{
                          ...textStyle,
                          ...(item.failed
                            ? { color: token("colors.muted") }
                            : {}),
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
                          style={{
                            fontWeight: token("typography.semibold_weight"),
                          }}
                        >
                          {parts.match}
                        </text>
                        {parts.after}
                      </text>
                    )}
                  </div>
                  <div
                    style={{
                      display: "flex",
                      flexDirection: "row",
                      alignItems: "center",
                      gap: token("metrics.gap"),
                      flexShrink: 0,
                    }}
                  >
                    {item.kind === "preset" && (hovered || focused) ? (
                      <ToolbarItem
                        label={`More actions for ${item.label}`}
                        attr={{ symbol: "ellipsis" }}
                        onClick={() => void model.presetMenu?.(item.id)}
                      />
                    ) : null}
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
                        attr={{ symbol: "plus" }}
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
    </div>
  );
}
