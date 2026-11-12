import { useLayoutEffect, useRef, useState } from "preact/hooks";
import {
  EmptyState,
  List,
  TextField,
  PushButton,
  ToolbarItem,
  token,
} from "../../src/components";
import type { Json, SurfaceEvent } from "../../src";
import type {
  Input,
  ItemId,
  Key,
  MenuEntry,
  Modifiers,
  Projection,
  Row,
} from "./contract";
import { itemKey, itemName, sameItem } from "./contract";
import {
  accessibleName,
  cardActions,
  clock,
  meta,
  status,
  menuEntries,
  type NativeMenuEntry,
} from "./presentation";
import { ProjectService } from "./service";

/** Trailing status ink. On a selected row (`selected_fill`) `muted` and
 * `danger` drop under 4.5:1, so the selection-aware tokens take over. */
export function trailingInk(
  state: { overlay: string; tone: "danger" | "muted" | "warning" } | null,
  selected: boolean,
): string {
  const tone = state?.overlay ? state.tone : "muted";
  if (!selected) return `colors.${tone}`;
  return tone === "danger"
    ? "colors.selected_danger"
    : "colors.selected_secondary";
}
export const projectPanel = Object.freeze({
  id: "assets",
  title: "Project",
  apiVersion: 4,
});
const text = {
  color: token("colors.text"),
  fontSize: token("typography.body"),
};
const muted = {
  color: token("colors.muted"),
  fontSize: token("typography.minimum"),
  fontVariantNumeric: "tabular-nums",
};
const target = {
  minWidth: token("metrics.hit"),
  minHeight: token("metrics.hit"),
};
export interface GridLayout {
  /** Resolved on a native layout worker from media-browser §2's column formula.
   * Never use a guessed column count or JS text measurement. */
  columns: number;
  rowHeight: number;
}
export interface NativeServices {
  /** Real registered local-text binding handles, supplied by the native host. */
  searchBinding: string;
  renameBinding: string;
  /** Enqueue-only native geometry/capture adapter. It emits the model's
   * Begin/MarqueeHits/MarqueeEdge/End inputs, including unmounted hit IDs. */
  marquee?(
    phase: "begin" | "move" | "end" | "cancel",
    event: SurfaceEvent,
    emit: (input: Input) => void,
  ): void;
  /** System menu; host owns placement, separators, shortcuts and focus return. */
  menu(
    entries: NativeMenuEntry[],
    choose: (entry: NativeMenuEntry) => void,
    context: { row?: Row; selectedCount: number; anchor: number },
  ): void;
}
export interface ProjectViewProps {
  view: Projection | null;
  transportFailed?: boolean;
  errorSequence?: number;
  unknownLoading?: ReadonlySet<string>;
  input(input: Input): void;
  menu(id: ItemId | null, anchor: number): void;
  native: NativeServices;
  layout: GridLayout;
  page(offset: number, limit: number): void;
  /** True only after the restoring publication has been empty for 0.5 s. */
  restoringEmptyAfterDelay?: boolean;
  restoring?: boolean;
  /** Poster keys are not resource leases; only a native resolver may supply these. */
  posters?: ReadonlyMap<string, string>;
}
export function modifiers(event: SurfaceEvent): Modifiers {
  return {
    meta: !!event.metaKey,
    control: !!event.ctrlKey,
    shift: !!event.shiftKey,
    alt: !!event.altKey,
  };
}
const keyNames: Record<string, Key> = {
  ArrowLeft: "Left",
  ArrowRight: "Right",
  ArrowUp: "Up",
  ArrowDown: "Down",
  Home: "Home",
  End: "End",
  Enter: "Enter",
  " ": "Space",
  Delete: "Delete",
  Backspace: "Backspace",
  Escape: "Escape",
};
const gridKeys = [...Object.keys(keyNames), "a", "A"];
function external(event: SurfaceEvent) {
  return !!event.drag?.types.includes("public.file-url");
}
function filePaths(event: SurfaceEvent): string[] {
  const paths = event.drag?.data["public.file-url"];
  return Array.isArray(paths) && paths.every((p) => typeof p === "string")
    ? (paths as string[])
    : [];
}
export function ProjectView(props: ProjectViewProps) {
  const { view, native, layout } = props;
  const input = (value: Input) => {
    if (!props.transportFailed) props.input(value);
  };
  if (
    !Number.isSafeInteger(layout.columns) ||
    layout.columns < 1 ||
    !Number.isFinite(layout.rowHeight) ||
    layout.rowHeight <= 0
  )
    throw new RangeError("Invalid native media grid layout");
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  // The host set the filter (the status line's missing files clear the
  // search): the field follows that query, and only a newer one. Answers to
  // the person's own typing, however late, never replace what they typed.
  const hostFilter = useRef(0);
  useLayoutEffect(() => {
    if (!view || view.host_filter <= hostFilter.current) return;
    hostFilter.current = view.host_filter;
    setQuery(view.query);
  }, [view?.host_filter]);
  const [hovered, setHovered] = useState<string | null>(null);
  const [rename, setRename] = useState("");
  const [crumbFocus, setCrumbFocus] = useState(0);
  const document = view?.page.stamp.document;
  useLayoutEffect(() => {
    setQuery("");
    setSearchOpen(false);
    setHovered(null);
    setRename("");
    setCrumbFocus(0);
  }, [document]);
  useLayoutEffect(() => {
    const folder = view?.page.rows.find(
      (r) => "Folder" in r.id && r.id.Folder === view.renaming,
    );
    if (folder) setRename(itemName(folder));
  }, [view?.renaming]);
  const filter = (value: string, missingOnly = !!view?.missing_only) => {
    setQuery(value);
    input({
      Filter: {
        query: value,
        kind: null,
        missing_only: missingOnly,
        proxy: null,
      },
    });
  };
  const invoke = (command: import("./contract").Command) =>
    input({ Invoke: command });
  const searchEscape = (event: SurfaceEvent) => {
    if (event.key !== "Escape" || event.composing) return;
    event.stopPropagation();
    if (query) filter("");
    else setSearchOpen(false);
  };
  const pointer = (
    row: Row,
    phase: "Down" | "Click" | "DoubleClick",
    event: SurfaceEvent,
  ) => {
    event.stopPropagation();
    if (event.button === 2) {
      if (phase === "Down" && !props.transportFailed)
        props.menu(row.id, event.currentTarget);
      return;
    }
    input({
      Pointer: {
        id: row.id,
        phase,
        button: event.button ?? 0,
        nested_control: false,
        modifiers: modifiers(event),
      },
    });
  };
  const dropHandlers = (folder: string | null) => ({
    "data-pm-drop": "files" as const,
    onDragOver: (event: SurfaceEvent) => {
      event.stopPropagation();
      input({ DragOver: { target: folder, files: external(event) } });
    },
    onDrop: (event: SurfaceEvent) => {
      event.stopPropagation();
      input({
        Drop: {
          target: folder,
          files: filePaths(event),
          modifiers: modifiers(event),
        },
      });
    },
  });
  const actionButtons = (
    actions: ReturnType<typeof cardActions>,
    ink: "colors.selected_text" | "colors.secondary",
  ) => (
    <div style={{ display: "flex", flexDirection: "row" }}>
      {actions.map((action) => (
        <ToolbarItem
          key={action.label}
          label={action.label}
          title={action.label}
          attr={{ symbol: action.symbol }}
          tabIndex={-1}
          disabled={props.transportFailed || action.disabled}
          onPointerDown={(event) => event.stopPropagation()}
          onDoubleClick={(event) => event.stopPropagation()}
          onDragStart={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.stopPropagation();
            if (!props.transportFailed && !action.disabled) input(action.input);
          }}
          style={{
            color: token(action.disabled ? "colors.muted" : ink),
          }}
        />
      ))}
    </div>
  );
  const renameField = (folderId: string, name: string) => (
    <TextField
      label={name}
      value={rename}
      id={`media-rename:${folderId}`}
      binding={native.renameBinding}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      focusRequest={`media-rename:${folderId}`}
      onInput={(e) => {
        if (typeof e.value === "string") setRename(e.value);
      }}
      preventDefault={{ keydown: ["Enter", "Escape"] }}
      onKeyDown={(e) => {
        if (e.composing) return;
        if (e.key === "Escape") {
          e.stopPropagation();
          input("CancelRename");
        }
        if (e.key === "Enter") {
          e.stopPropagation();
          input({ CommitRename: { folder: folderId, value: rename } });
        }
      }}
      onBlur={() =>
        input({ CommitRename: { folder: folderId, value: rename } })
      }
    />
  );
  /** One-column layout: a quiet source-list row (language §9 Lists). The name
   * leads; the trailing column holds the duration, or the media status in its
   * tone when the file needs attention. No thumbnail, badge or glyph. */
  const renderRow = (row: Row) => {
    const name = itemName(row);
    const asset = "Asset" in row.data ? row.data.Asset : null;
    const comp = "Composition" in row.data ? row.data.Composition : null;
    const folderId = "Folder" in row.data ? row.data.Folder.id : null;
    const loading = !!asset && !!props.unknownLoading?.has(asset[0].id);
    const state = asset
      ? status(loading ? "Loading" : asset[1].availability, asset[0])
      : null;
    const duration =
      comp?.duration ??
      (asset?.[0].kind === "image" ? null : asset?.[0].duration);
    // A missing file reads as one quiet word; its path is in the tooltip
    // and the relink flow, never in the row.
    const trailing = state?.overlay
      ? state.overlay === "Media offline"
        ? "Missing"
        : state.overlay
      : duration !== null && duration !== undefined
        ? clock(duration)
        : "";
    const hover = !view?.drag && (hovered === itemKey(row.id) || row.focused);
    const actions = hover ? cardActions(row, view!.composition_count) : [];
    const detail = [meta(row), state?.detail, row.folder_path?.join(" / ")]
      .filter(Boolean)
      .join(" · ");
    return (
      <div
        key={itemKey(row.id)}
        id={`media:${itemKey(row.id)}`}
        role="cell"
        label={accessibleName(row)}
        title={detail ? `${name}\n${detail}` : name}
        tabIndex={-1}
        aria-selected={row.selected}
        attr={{ item: row.id }}
        aria-disabled={!!props.transportFailed}
        state={{
          selected: row.selected,
          focused: row.focused,
          dragging: !!view?.drag,
          disabled: !!props.transportFailed,
          hovered: hovered === itemKey(row.id),
          loading,
          dropTarget:
            folderId !== null &&
            !!view?.drop_target &&
            view.drop_target.folder === folderId,
        }}
        style={{
          ...text,
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
          height: token("metrics.row"),
          gap: token("metrics.gap"),
          paddingLeft: token("metrics.gap"),
          paddingRight: token("metrics.gap"),
          borderRadius: token("metrics.control_radius"),
          background: row.selected
            ? token("colors.selected_fill")
            : view?.drop_target && view.drop_target.folder === folderId
              ? token("colors.accent_wash")
              : null,
        }}
        onPointerDown={(e) => pointer(row, "Down", e)}
        onClick={(e) => pointer(row, "Click", e)}
        onDoubleClick={(e) => pointer(row, "DoubleClick", e)}
        onPointerMove={() => setHovered(itemKey(row.id))}
        onPointerCancel={() => setHovered(null)}
        onDragStart={(e) => {
          if (e.target === e.currentTarget)
            input({ DragStart: { id: row.id, nested_control: false } });
        }}
        onDragEnd={() => input("DragEnd")}
        {...(folderId ? dropHandlers(folderId) : {})}
      >
        {folderId && view?.renaming === folderId ? (
          renameField(folderId, name)
        ) : (
          <text
            style={{
              ...text,
              flexGrow: 1,
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            {name}
          </text>
        )}
        {actions.length ? (
          actionButtons(actions, "colors.secondary")
        ) : trailing ? (
          <text
            style={{
              ...muted,
              flexShrink: 0,
              whiteSpace: "nowrap",
              color: token(trailingInk(state, row.selected)),
            }}
          >
            {trailing}
          </text>
        ) : null}
      </div>
    );
  };
  const renderCard = (row: Row) => {
    const name = itemName(row);
    const asset = "Asset" in row.data ? row.data.Asset : null;
    const comp = "Composition" in row.data ? row.data.Composition : null;
    const overlay = asset
      ? status(
          props.unknownLoading?.has(asset[0].id)
            ? "Loading"
            : asset[1].availability,
          asset[0],
        )
      : null;
    const duration =
      comp?.duration ??
      (asset?.[0].kind === "image" ? null : asset?.[0].duration);
    const posterKey = asset?.[1].poster ?? comp?.poster;
    const poster = posterKey ? props.posters?.get(posterKey) : undefined;
    const actions = cardActions(row, view!.composition_count);
    const folderId = "Folder" in row.data ? row.data.Folder.id : null;
    return (
      <div
        key={itemKey(row.id)}
        id={`media:${itemKey(row.id)}`}
        role="cell"
        label={accessibleName(row)}
        tabIndex={-1}
        aria-selected={row.selected}
        attr={{ item: row.id }}
        aria-disabled={!!props.transportFailed}
        state={{
          selected: row.selected,
          focused: row.focused,
          dragging: !!view?.drag,
          disabled: !!props.transportFailed,
          hovered: hovered === itemKey(row.id),
          loading: !!asset && !!props.unknownLoading?.has(asset[0].id),
          dropTarget:
            folderId !== null &&
            !!view?.drop_target &&
            view.drop_target.folder === folderId,
        }}
        style={{ ...target, ...text, display: "flex", flexDirection: "column" }}
        onPointerDown={(e) => pointer(row, "Down", e)}
        onClick={(e) => pointer(row, "Click", e)}
        onDoubleClick={(e) => pointer(row, "DoubleClick", e)}
        onPointerMove={() => setHovered(itemKey(row.id))}
        onPointerCancel={() => setHovered(null)}
        onDragStart={(e) => {
          if (e.target === e.currentTarget)
            input({ DragStart: { id: row.id, nested_control: false } });
        }}
        onDragEnd={() => input("DragEnd")}
        {...(folderId ? dropHandlers(folderId) : {})}
      >
        <div
          style={{
            background: token("colors.field"),
            borderRadius: token("metrics.control_radius"),
          }}
        >
          {poster ? <img resource={poster} alt={name} /> : null}
          {duration !== null && duration !== undefined ? (
            <text
              style={{
                fontSize: token("typography.minimum"),
                color: token("colors.selected_text"),
                fontVariantNumeric: "tabular-nums",
              }}
            >
              {clock(duration)}
            </text>
          ) : null}
          {comp?.open ? (
            <text
              label="Open in the timeline"
              style={{ color: token("colors.accent") }}
            />
          ) : null}
          {overlay?.overlay ? (
            <text
              style={{
                fontSize: token("typography.minimum"),
                color: token("colors.selected_text"),
              }}
              metadata={{ progress: asset?.[1].progress ?? null }}
            >
              {overlay.overlay}
            </text>
          ) : null}
          {!view?.drag && (hovered === itemKey(row.id) || row.focused)
            ? actionButtons(actions, "colors.selected_text")
            : null}
        </div>
        {folderId && view?.renaming === folderId ? (
          renameField(folderId, name)
        ) : (
          <text
            title={name}
            style={{
              ...text,
              marginTop: token("metrics.gap"),
              background: row.selected ? token("colors.selected_fill") : null,
              borderRadius: token("metrics.capsule_radius"),
              overflow: "hidden",
              whiteSpace: "nowrap",
            }}
          >
            {name}
          </text>
        )}
        <text
          style={{
            ...muted,
            color: token(
              overlay?.detail ? `colors.${overlay.tone}` : "colors.muted",
            ),
            whiteSpace: "nowrap",
            overflow: "hidden",
          }}
        >
          {[meta(row), row.folder_path?.join(" / ")]
            .filter(Boolean)
            .join(" · ")}
        </text>
      </div>
    );
  };
  const crumbItems = [{ id: null, name: "Media" }, ...(view?.crumbs ?? [])];
  const count = Math.ceil((view?.page.total ?? 0) / layout.columns);
  const start = Math.floor((view?.page.offset ?? 0) / layout.columns);
  const end = Math.min(
    count,
    Math.ceil(
      ((view?.page.offset ?? 0) + (view?.page.rows.length ?? 0)) /
        layout.columns,
    ),
  );
  const gridKey = (event: SurfaceEvent) => {
    if (event.composing || event.target !== event.currentTarget) return;
    if (event.key === "F2" && view?.focused && "Folder" in view.focused) {
      input({ BeginRename: view.focused.Folder });
      return;
    }
    const key =
      event.key?.toLowerCase() === "a" && (event.metaKey || event.ctrlKey)
        ? "A"
        : keyNames[event.key ?? ""];
    if (!key) return;
    event.stopPropagation();
    input({
      GridKey: {
        columns: layout.columns,
        target: view?.focused ? { Row: view.focused } : "List",
        key,
        modifiers: modifiers(event),
      },
    });
  };
  return (
    <div
      role="group"
      label="Project"
      style={{
        background: token("colors.panel"),
        borderRadius: token("metrics.panel_radius"),
        display: "flex",
        flexDirection: "column",
      }}
      onKeyDown={(event) => {
        if (event.composing || !event.metaKey || event.altKey || event.ctrlKey)
          return;
        if (event.key?.toLowerCase() === "f") {
          event.stopPropagation();
          setSearchOpen(true);
        }
        if (event.key?.toLowerCase() === "i" && view) {
          event.stopPropagation();
          invoke("ImportFiles");
        }
        if (event.key?.toLowerCase() === "n" && view) {
          event.stopPropagation();
          invoke("NewComposition");
        }
      }}
    >
      <div
        data-pm-slot="header"
        style={{
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
          height: token("metrics.header"),
          paddingLeft: token("metrics.inset"),
          paddingRight: token("metrics.inset"),
        }}
      >
        <text
          style={{
            color: token("colors.text"),
            fontSize: token("typography.title"),
            fontWeight: token("typography.semibold_weight"),
          }}
        >
          Project
        </text>
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
          }}
        >
          <ToolbarItem
            label="Search Media"
            attr={{ symbol: "magnifyingglass" }}
            title="Search Media"
            disabled={!view}
            active={searchOpen}
            onClick={() => setSearchOpen(!searchOpen)}
            style={{ symbolSize: token("metrics.symbol_panel") }}
          />
          <ToolbarItem
            label="Add"
            title="Add"
            attr={{ symbol: "plus" }}
            disabled={!view || props.transportFailed}
            onClick={(e) => {
              if (!props.transportFailed) props.menu(null, e.currentTarget);
            }}
            style={{ symbolSize: token("metrics.symbol_panel") }}
          />
        </div>
      </div>
      {searchOpen && view ? (
        <div
          style={{
            height: token("metrics.row"),
            paddingLeft: token("metrics.inset"),
            paddingRight: token("metrics.inset"),
          }}
        >
          <TextField
            label="Search media"
            title="Search media"
            value={query}
            metadata={{ hostValue: hostFilter.current }}
            id="media-search"
            binding={native.searchBinding}
            focusRequest="media-search"
            onInput={(e) => {
              if (typeof e.value === "string" && !e.composing) filter(e.value);
            }}
            onKeyDown={searchEscape}
            preventDefault={{ keydown: ["Escape"] }}
          />
        </div>
      ) : null}
      {view?.missing_only ? (
        <div
          style={{
            height: token("metrics.row"),
            paddingLeft: token("metrics.inset"),
            paddingRight: token("metrics.inset"),
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <text
            role="status"
            style={{
              color: token("colors.danger"),
              fontSize: token("typography.minimum"),
            }}
          >
            Missing files only
          </text>
          <PushButton
            label="Show All"
            title="Show all media"
            onClick={() => filter(query, false)}
          />
        </div>
      ) : null}
      {view?.folder ? (
        <div
          role="toolbar"
          label="Folder path"
          focusRequest={`media-crumb:${Math.min(crumbFocus, crumbItems.length - 1)}`}
          style={{
            height: token("metrics.row"),
            paddingLeft: token("metrics.inset"),
            paddingRight: token("metrics.inset"),
            display: "flex",
            flexDirection: "row",
          }}
        >
          {crumbItems.map((crumb, index) => [
            index > 0 ? (
              <text
                key={`separator:${index}`}
                style={{
                  color: token("colors.muted"),
                  fontSize: token("typography.body"),
                }}
              >
                {" / "}
              </text>
            ) : null,
            <ToolbarItem
              attr={index === 0 ? { symbol: "chevron.left" } : undefined}
              key={crumb.id ?? "root"}
              id={`media-crumb:${index}`}
              label={crumb.name}
              title={crumb.name}
              tabIndex={
                index === Math.min(crumbFocus, crumbItems.length - 1) ? 0 : -1
              }
              value={
                index === crumbItems.length - 1 ? "current location" : undefined
              }
              onClick={() => input({ OpenFolder: crumb.id })}
              {...dropHandlers(crumb.id)}
              preventDefault={{ keydown: ["ArrowLeft", "ArrowRight"] }}
              onKeyDown={(e) => {
                if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                  e.stopPropagation();
                  setCrumbFocus(
                    (index +
                      (e.key === "ArrowLeft" ? -1 : 1) +
                      crumbItems.length) %
                      crumbItems.length,
                  );
                }
              }}
              style={{
                color: token(
                  index === crumbItems.length - 1
                    ? "colors.text"
                    : "colors.secondary",
                ),
                fontSize: token("typography.body"),
              }}
            >
              {crumb.name}
            </ToolbarItem>,
          ])}
        </div>
      ) : null}
      {props.transportFailed && (
        <div
          id="media:transport-error"
          role="status"
          label="Error"
          metadata={{ sequence: props.errorSequence ?? 0 }}
          style={{ ...text, color: token("colors.warning") }}
        >
          <text
            attr={{ symbol: "exclamationmark.triangle.fill" }}
            style={{ color: token("colors.warning") }}
          />
          <text>Project media isn’t updating right now.</text>
        </div>
      )}
      <List
        label="Project media"
        tabIndex={0}
        attr={{ rowCount: view?.page.total ?? 0, multiselectable: true }}
        style={{
          ...target,
          // List rows inset their own selection by `gap`; grid cards sit at `inset`.
          paddingLeft: token(
            layout.columns === 1 ? "metrics.gap" : "metrics.inset",
          ),
          paddingRight: token(
            layout.columns === 1 ? "metrics.gap" : "metrics.inset",
          ),
          paddingTop: token("metrics.gap"),
        }}
        preventDefault={{ keydown: [...gridKeys, "F2"] }}
        onKeyDown={gridKey}
        onPointerDown={(e) => {
          if (e.button === 2 && !props.transportFailed)
            props.menu(null, e.currentTarget);
          else native.marquee?.("begin", e, input);
        }}
        onPointerMove={(e) => {
          if (e.target === e.currentTarget) setHovered(null);
          native.marquee?.("move", e, input);
        }}
        onPointerUp={(e) => native.marquee?.("end", e, input)}
        onPointerCancel={(e) => native.marquee?.("cancel", e, input)}
        onBlur={() => input({ EndMarquee: { pointer: null } })}
        onDragEnter={(e) => {
          if (external(e)) input("FilesEnter");
        }}
        onDragLeave={(e) => {
          if (external(e)) input("FilesLeave");
          else input("DropLeave");
        }}
        {...dropHandlers(view?.folder ?? null)}
      >
        {!view ? (
          <EmptyState
            label="No Project"
            explanation="Open a project to see its media."
            attr={{ symbol: "photo.on.rectangle" }}
            action={null}
          />
        ) : view.page.total === 0 ? (
          props.restoring &&
          !props.restoringEmptyAfterDelay ? null : props.restoringEmptyAfterDelay ? (
            <text style={muted}>Loading media…</text>
          ) : query ? (
            <text
              style={{
                fontSize: token("typography.reading"),
                color: token("colors.secondary"),
                textAlign: "center",
              }}
            >{`No media matches “${query}”`}</text>
          ) : view.folder ? (
            <EmptyState
              label="Empty Folder"
              explanation="Drag media or files here."
              attr={{ symbol: "folder" }}
              action={null}
            />
          ) : (
            <EmptyState
              label="No Media"
              explanation="Drag files here or import from your computer."
              attr={{ symbol: "photo.on.rectangle.angled" }}
              action={
                <ToolbarItem
                  label="Import…"
                  onClick={() => invoke("ImportFiles")}
                />
              }
            />
          )
        ) : (
          <virtual-list
            count={count}
            range={[start, end]}
            itemSize={layout.rowHeight}
            onRangeChange={(e) => {
              if (!e.range) return;
              const [from, to] = e.range;
              if (
                from < 0 ||
                to < from ||
                to > count ||
                (to - from) * layout.columns > 4096
              )
                return;
              props.page(
                from * layout.columns,
                Math.min(
                  view.page.total - from * layout.columns,
                  (to - from) * layout.columns,
                ),
              );
            }}
          >
            {Array.from({ length: end - start }, (_, index) => (
              <div
                key={start + index}
                style={{
                  display: "flex",
                  // One column stacks full-width list rows; more columns lay cards out.
                  flexDirection: layout.columns === 1 ? "column" : "row",
                  gap: token("metrics.inset"),
                }}
              >
                {view.page.rows
                  .slice(index * layout.columns, (index + 1) * layout.columns)
                  .map(layout.columns === 1 ? renderRow : renderCard)}
              </div>
            ))}
          </virtual-list>
        )}
        {view?.external_drag ? (
          <div
            style={{
              background: token("colors.accent_wash"),
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <EmptyState
              label="Add to Project Media"
              explanation="Drops here don’t touch the timeline."
              action={null}
              attr={{ symbol: "plus" }}
              style={{
                background: token("colors.floating"),
                borderRadius: token("metrics.panel_radius"),
                padding: token("metrics.inset"),
              }}
            />
          </div>
        ) : null}
      </List>
      {view && view.selected_count >= 2 ? (
        <div
          role="group"
          label={`${view.selected_count} selected`}
          style={{
            background: token("colors.floating"),
            borderRadius: token("metrics.control_radius"),
            height: token("metrics.row"),
            margin: token("metrics.inset"),
            paddingLeft: token("metrics.inset"),
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <text style={text}>{`${view.selected_count} selected`}</text>
          <ToolbarItem
            label="Delete selected media"
            title="Delete Selected Media…"
            attr={{ symbol: "trash" }}
            disabled={props.transportFailed}
            onClick={() => invoke("DeleteAssets")}
            style={{ color: token("colors.danger") }}
          />
        </div>
      ) : null}
    </div>
  );
}
export function ProjectPanel({
  service,
  ...props
}: Omit<ProjectViewProps, "view" | "input" | "menu" | "page"> & {
  service: ProjectService;
}) {
  const [, update] = useState(0);
  useLayoutEffect(() => service.listen(() => update((n) => n + 1)), [service]);
  const view = service.view;
  return (
    <ProjectView
      {...props}
      view={view}
      transportFailed={service.error !== null}
      errorSequence={service.errorSequence}
      unknownLoading={
        new Set(
          view?.page.rows
            .filter((row) => service.readiness.loading(row, service.now()))
            .flatMap((row) => ("Asset" in row.id ? [row.id.Asset] : [])) ?? [],
        )
      }
      input={(input) => {
        if (view) service.run(service.input(input, view.page.stamp));
      }}
      page={(offset, limit) =>
        view && service.run(service.page(offset, limit, view.page.stamp))
      }
      menu={(id, anchor) =>
        service.run(
          service.menu(id, view?.page.stamp).then((reply) => {
            if (!reply?.menu) return;
            const row = reply.view.page.rows.find((row) =>
              sameItem(row.id, id),
            );
            props.native.menu(
              menuEntries(reply.menu.entries, row, reply.view.selected_count),
              (entry) => {
                if (entry.enabled)
                  service.run(
                    service.input({ Invoke: entry.command }, reply.menu!.owner),
                  );
              },
              { row, selectedCount: reply.view.selected_count, anchor },
            );
          }),
        )
      }
    />
  );
}
export { ProjectService } from "./service";
export type { Projection, Input, ItemId, MenuEntry, Json };
