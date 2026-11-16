import { useLayoutEffect, useRef, useState } from "preact/hooks";
import {
  EmptyState,
  List,
  TextField,
  PushButton,
  SegmentedControl,
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
  /** A grid card's thumbnail, in points (16:9 of its column). */
  thumbWidth?: number;
  thumbHeight?: number;
}
/** The panel's two layouts: source-list rows, or a grid of thumbnails. */
export type ViewMode = "list" | "grid";
/** The page the panel asks for first, and the least it asks for after. */
export const FIRST_PAGE = 96;
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
  /** The header's list/grid choice; absent, the header offers none. */
  viewMode?: ViewMode;
  onViewMode?(mode: ViewMode): void;
  /** The list's width in points, as the host lays it out. */
  onWidth?(width: number): void;
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
  // The field shows the query that filters the list: the model's, except
  // for what was just typed and is not answered yet (an answer to an earlier
  // keystroke never replaces a later one). A copy of its own could disagree
  // with the filter, a placeholder over a filtered list once the panel was
  // drawn again. A filter the host set (the status line's missing files
  // clear the search) replaces what is typed, once per revision.
  const [typed, setTyped] = useState<string | null>(null);
  const query = typed ?? view?.query ?? "";
  useLayoutEffect(() => {
    if (typed !== null && view?.query === typed) setTyped(null);
  }, [view?.query, typed]);
  const hostFilter = useRef(0);
  useLayoutEffect(() => {
    if (!view || view.host_filter <= hostFilter.current) return;
    hostFilter.current = view.host_filter;
    setTyped(null);
  }, [view?.host_filter]);
  const [hovered, setHovered] = useState<string | null>(null);
  /** The page last asked for by the list's range (offset, limit, document). */
  const askedPage = useRef("");
  const [rename, setRename] = useState("");
  const [crumbFocus, setCrumbFocus] = useState(0);
  const document = view?.page.stamp.document;
  useLayoutEffect(() => {
    setTyped(null);
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
    setTyped(value);
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
  // Escape's clear is the panel's own change to the text: it replaces what
  // the field holds even while it is typed in (an echo of the typing leaves
  // the field's committed value behind, so a plain "" might change nothing).
  const [clears, setClears] = useState(0);
  const searchEscape = (event: SurfaceEvent) => {
    if (event.key !== "Escape" || event.composing) return;
    event.stopPropagation();
    if (query) {
      filter("");
      setClears(clears + 1);
    } else setSearchOpen(false);
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
        // Typing a name is the field's: no key moves or previews the grid.
        e.stopPropagation();
        if (e.composing) return;
        if (e.key === "Escape") {
          input("CancelRename");
        }
        if (e.key === "Enter") {
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
  /** What a row's thumbnail shows (v1.2.0's AssetsPanel tile): the poster,
   * or the kind's glyph on the quiet well; audio on its green ground. */
  const facts = (row: Row) => {
    const asset = "Asset" in row.data ? row.data.Asset : null;
    const comp = "Composition" in row.data ? row.data.Composition : null;
    const folder = "Folder" in row.data ? row.data.Folder : null;
    const loading = !!asset && !!props.unknownLoading?.has(asset[0].id);
    const state = asset
      ? status(loading ? "Loading" : asset[1].availability, asset[0])
      : null;
    const duration =
      comp?.duration ??
      (asset?.[0].kind === "image" ? null : asset?.[0].duration);
    const posterKey = asset?.[1].poster ?? comp?.poster;
    const poster = posterKey ? props.posters?.get(posterKey) : undefined;
    // Offline, unavailable and in-cloud media are the ones needing the
    // person; Loading is only a quiet wait.
    const attention =
      !!state?.overlay &&
      state.overlay !== "Loading…" &&
      state.overlay !== "3D Model";
    const cloud =
      !!asset &&
      typeof asset[1].availability === "object" &&
      "Cloud" in asset[1].availability;
    return {
      asset,
      comp,
      folder,
      loading,
      state,
      duration,
      poster,
      attention,
      cloud,
    };
  };
  const glyph = (f: ReturnType<typeof facts>) =>
    f.folder
      ? "folder"
      : f.comp
        ? "square.stack.3d.down.right"
        : f.asset?.[0].kind === "audio"
          ? "music.note"
          : f.asset?.[0].kind === "video"
            ? "film"
            : f.asset?.[0].kind === "model"
              ? "cube"
              : "photo";
  /** A composition's frame on its tile (v1.2.0 `.comp-frame`): inset in
   * the tile, holding its thumbnail. A comp is the tile's 16:9 as a rule;
   * the frame takes the tile's shape inside its inset. */
  const compFrame = (poster: string | undefined) => (
    <div
      style={{
        position: "absolute",
        inset: "fill",
        padding: token("metrics.media_comp_inset"),
        display: "flex",
      }}
    >
      <div
        style={{
          position: "relative",
          flexGrow: 1,
          overflow: "hidden",
          borderRadius: token("metrics.inner_radius"),
          borderWidth: token("metrics.border"),
          borderColor: token("colors.hairline"),
          background: token("colors.media_badge"),
        }}
      >
        {poster ? (
          <img
            resource={poster}
            alt=""
            style={{ position: "absolute", inset: "fill" }}
          />
        ) : null}
      </div>
    </div>
  );
  /** The 16:9 tile. `small`: a list row's leading thumbnail. */
  const tile = (row: Row, small: boolean) => {
    const f = facts(row);
    const hover =
      !small && !view?.drag && (hovered === itemKey(row.id) || row.focused);
    const actions = hover ? cardActions(row, view!.composition_count) : [];
    const ring = row.selected
      ? "colors.accent"
      : row.focused && !small
        ? "colors.focus"
        : null;
    const dropTarget =
      !!f.folder &&
      !!view?.drop_target &&
      view.drop_target.folder === f.folder.id;
    return (
      <div
        style={{
          position: "relative",
          overflow: "hidden",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          borderRadius: token(
            small ? "metrics.inner_radius" : "metrics.control_radius",
          ),
          background: token(
            dropTarget
              ? "colors.accent_wash"
              : f.asset?.[0].kind === "audio" && !f.attention
                ? "colors.labels.green"
                : "colors.fill",
          ),
          ...(small
            ? {
                width: token("metrics.media_row_thumb_width"),
                height: token("metrics.media_row_thumb_height"),
                flexShrink: 0,
              }
            : { flexGrow: 1 }),
        }}
      >
        {/* A missing file's tile reads red under its dimmed picture. */}
        {f.attention && !f.cloud ? (
          <div
            style={{
              position: "absolute",
              inset: "fill",
              background: token("colors.danger"),
              opacity: token("metrics.media_tint_opacity"),
            }}
          />
        ) : f.cloud ? (
          <div
            style={{
              position: "absolute",
              inset: "fill",
              background: token("colors.accent"),
              opacity: token("metrics.media_tint_opacity"),
            }}
          />
        ) : null}
        {f.comp && !small ? compFrame(f.poster) : null}
        {f.poster && !f.folder && !(f.comp && !small) ? (
          <img
            resource={f.poster}
            alt={itemName(row)}
            style={{
              position: "absolute",
              inset: "fill",
              opacity: f.attention
                ? token("metrics.media_offline_opacity")
                : f.loading
                  ? token("metrics.media_loading_opacity")
                  : null,
            }}
          />
        ) : (f.attention && !f.cloud) || (f.comp && f.poster) ? null : (
          <text
            attr={{ symbol: glyph(f) }}
            style={{
              color: token(dropTarget ? "colors.accent" : "colors.muted"),
              symbolSize: token(
                small
                  ? "metrics.symbol_inline"
                  : f.folder
                    ? "metrics.media_folder_glyph"
                    : "metrics.media_glyph",
              ),
            }}
          />
        )}
        {/* Selected: a 2 pt accent ring inside the tile; keyboard focus a
            quieter one (AssetsPanel `.asset-preview::after`). */}
        {ring || dropTarget ? (
          <div
            style={{
              position: "absolute",
              inset: "fill",
              borderRadius: token(
                small ? "metrics.inner_radius" : "metrics.control_radius",
              ),
              borderWidth: token(
                small ? "metrics.border" : "metrics.media_ring",
              ),
              borderColor: token(ring ?? "colors.accent"),
            }}
          />
        ) : null}
        {small ? null : (
          <>
            {f.duration !== null &&
            f.duration !== undefined &&
            !f.state?.overlay &&
            !hover ? (
              <text
                style={{
                  position: "absolute",
                  left: token("metrics.media_inset"),
                  top: token("metrics.media_inset"),
                  paddingLeft: token("metrics.media_inset"),
                  paddingRight: token("metrics.media_inset"),
                  height: token("metrics.media_badge_height"),
                  background: token("colors.media_badge"),
                  borderRadius: token("metrics.inner_radius"),
                  fontSize: token("typography.minimum"),
                  color: token("colors.selected_text"),
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {clock(f.duration)}
              </text>
            ) : null}
            {f.state?.overlay ? (
              <div
                role="img"
                label={f.state.overlay}
                metadata={{ progress: f.asset?.[1].progress ?? null }}
                style={{
                  position: "absolute",
                  left: token("metrics.media_inset"),
                  bottom: token("metrics.media_inset"),
                  height: token("metrics.media_badge_height"),
                  paddingLeft: token("metrics.media_inset"),
                  paddingRight: token("metrics.media_inset"),
                  gap: token("metrics.media_inset"),
                  display: "flex",
                  flexDirection: "row",
                  alignItems: "center",
                  borderRadius: token("metrics.inner_radius"),
                  background: token(
                    f.cloud
                      ? "colors.accent"
                      : f.attention
                        ? "colors.danger"
                        : "colors.media_badge",
                  ),
                }}
              >
                {f.attention ? (
                  <text
                    attr={{
                      symbol: f.cloud
                        ? "icloud.and.arrow.down"
                        : "exclamationmark.triangle.fill",
                    }}
                    style={{
                      color: token("colors.selected_text"),
                      symbolSize: token("metrics.symbol_inline"),
                    }}
                  />
                ) : null}
                <text
                  style={{
                    fontSize: token("typography.minimum"),
                    fontWeight: token("typography.medium_weight"),
                    color: token("colors.selected_text"),
                    whiteSpace: "nowrap",
                  }}
                >
                  {f.state.overlay}
                </text>
              </div>
            ) : null}
            {f.folder?.link ? (
              <text
                attr={{
                  symbol:
                    f.folder.sync === "NotFound"
                      ? "link.badge.plus"
                      : f.folder.sync === "Syncing"
                        ? "arrow.triangle.2.circlepath"
                        : "link",
                }}
                style={{
                  position: "absolute",
                  right: token("metrics.media_inset"),
                  bottom: token("metrics.media_inset"),
                  symbolSize: token("metrics.symbol_inline"),
                  color: token(
                    f.folder.sync === "NotFound"
                      ? "colors.danger"
                      : "colors.secondary",
                  ),
                }}
              />
            ) : null}
            {f.comp?.open ? (
              <div
                role="img"
                label="Open in the timeline"
                style={{
                  position: "absolute",
                  right: token("metrics.media_inset"),
                  bottom: token("metrics.media_inset"),
                  width: token("metrics.status_dot"),
                  height: token("metrics.status_dot"),
                  borderRadius: token("metrics.status_dot"),
                  background: token("colors.accent"),
                }}
              />
            ) : null}
            {actions.length ? (
              <div
                style={{
                  position: "absolute",
                  right: token("metrics.media_inset"),
                  top: token("metrics.media_inset"),
                }}
              >
                {actionButtons(actions, "colors.selected_text")}
              </div>
            ) : null}
          </>
        )}
      </div>
    );
  };
  /** The quiet line under a name: what the item is, or what it needs. */
  const detailLine = (row: Row) => {
    const f = facts(row);
    if (f.folder) {
      const n = f.folder.items;
      const count = n ? `${n} ${n === 1 ? "item" : "items"}` : "Empty";
      if (f.folder.sync === "NotFound") return "Folder not found";
      if (f.folder.sync === "Syncing") return "Syncing…";
      return f.folder.link ? `Linked · ${n} items` : count;
    }
    if (f.state?.overlay === "Loading…") return "Loading media…";
    return [meta(row), row.folder_path?.join(" / ")]
      .filter(Boolean)
      .join(" · ");
  };
  const rowHandlers = (row: Row) => {
    const folderId = "Folder" in row.data ? row.data.Folder.id : null;
    return {
      onPointerDown: (e: SurfaceEvent) => pointer(row, "Down", e),
      onClick: (e: SurfaceEvent) => pointer(row, "Click", e),
      onDoubleClick: (e: SurfaceEvent) => pointer(row, "DoubleClick", e),
      onPointerMove: () => setHovered(itemKey(row.id)),
      onPointerCancel: () => setHovered(null),
      onDragStart: (e: SurfaceEvent) => {
        if (e.target === e.currentTarget)
          input({ DragStart: { id: row.id, nested_control: false } });
      },
      onDragEnd: () => input("DragEnd"),
      ...(folderId ? dropHandlers(folderId) : {}),
    };
  };
  const itemProps = (row: Row) => {
    const f = facts(row);
    const name = itemName(row);
    const detail = detailLine(row);
    return {
      id: `media:${itemKey(row.id)}`,
      role: "cell",
      label: accessibleName(row),
      title: f.folder?.link
        ? `Linked to ${f.folder.link.path}. Changes in Finder appear here. Renaming it here doesn't rename the folder on disk.`
        : detail
          ? `${name}\n${detail}`
          : name,
      tabIndex: -1,
      "aria-selected": row.selected,
      attr: { item: row.id },
      "aria-disabled": !!props.transportFailed,
      state: {
        selected: row.selected,
        focused: row.focused,
        dragging: !!view?.drag,
        disabled: !!props.transportFailed,
        hovered: hovered === itemKey(row.id),
        loading: f.loading,
        dropTarget:
          !!f.folder &&
          !!view?.drop_target &&
          view.drop_target.folder === f.folder.id,
      },
    } as const;
  };
  const nameText = (row: Row) => {
    const f = facts(row);
    return f.folder && view?.renaming === f.folder.id ? (
      renameField(f.folder.id, itemName(row))
    ) : (
      <text
        style={{
          ...text,
          flexShrink: 1,
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
        }}
      >
        {itemName(row)}
      </text>
    );
  };
  /** One-column layout: a source-list row in v1.2.0's language. A small
   * tile leads (its picture, or the kind's glyph), the name follows, and the
   * trailing column holds the duration, or the status in red when the file
   * needs attention. Selection is the row's fill, as in a Finder list. */
  const renderRow = (row: Row) => {
    const f = facts(row);
    const hover = !view?.drag && (hovered === itemKey(row.id) || row.focused);
    const actions = hover ? cardActions(row, view!.composition_count) : [];
    const trailing = f.state?.overlay
      ? f.state.overlay === "Media offline"
        ? "Missing"
        : f.state.overlay
      : f.folder
        ? f.folder.items
          ? String(f.folder.items)
          : ""
        : f.duration !== null && f.duration !== undefined
          ? clock(f.duration)
          : "";
    return (
      <div
        key={itemKey(row.id)}
        {...itemProps(row)}
        style={{
          ...text,
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          height: token("metrics.row"),
          gap: token("metrics.gap"),
          paddingLeft: token("metrics.gap"),
          paddingRight: token("metrics.gap"),
          borderRadius: token("metrics.control_radius"),
          background: row.selected ? token("colors.selected_fill") : null,
        }}
        {...rowHandlers(row)}
      >
        {tile(row, true)}
        <div
          style={{
            flexGrow: 1,
            flexShrink: 1,
            overflow: "hidden",
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            gap: token("metrics.gap"),
          }}
        >
          {nameText(row)}
          {f.folder?.link ? (
            <text
              attr={{
                symbol:
                  f.folder.sync === "NotFound"
                    ? "link.badge.plus"
                    : f.folder.sync === "Syncing"
                      ? "arrow.triangle.2.circlepath"
                      : "link",
              }}
              style={{
                symbolSize: token("metrics.symbol_inline"),
                color: token(
                  f.folder.sync === "NotFound"
                    ? row.selected
                      ? "colors.selected_danger"
                      : "colors.danger"
                    : row.selected
                      ? "colors.selected_secondary"
                      : "colors.secondary",
                ),
              }}
            />
          ) : null}
        </div>
        {actions.length ? (
          actionButtons(actions, "colors.secondary")
        ) : trailing ? (
          <text
            style={{
              ...muted,
              flexShrink: 0,
              whiteSpace: "nowrap",
              color: token(
                trailingInk(f.attention ? f.state : null, row.selected),
              ),
            }}
          >
            {trailing}
          </text>
        ) : null}
      </div>
    );
  };
  /** A grid card (v1.2.0's `.asset-card`): the tile, then the name and a
   * quiet detail line; red when the file is missing. */
  const renderCard = (row: Row) => {
    const f = facts(row);
    return (
      <div
        key={itemKey(row.id)}
        {...itemProps(row)}
        style={{
          ...target,
          ...text,
          display: "flex",
          flexDirection: "column",
          gap: token("metrics.media_inset"),
          // Every card of a row is as wide (media-browser §2).
          flexGrow: 1,
          flexShrink: 1,
          flexBasis: token("metrics.thumbnail_min"),
          overflow: "hidden",
          paddingBottom: token("metrics.inset"),
        }}
        {...rowHandlers(row)}
      >
        {tile(row, false)}
        <div style={{ display: "flex", flexDirection: "column" }}>
          {nameText(row)}
          <text
            style={{
              ...muted,
              color: token(
                (f.attention && !f.cloud) || f.folder?.sync === "NotFound"
                  ? "colors.danger"
                  : "colors.muted",
              ),
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            {detailLine(row)}
          </text>
        </div>
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
    // The list or one of its items: a click focuses the item it lands on,
    // and its keys are the grid's (fields inside keep theirs, below).
    if (event.composing) return;
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
          {props.viewMode && props.onViewMode ? (
            <SegmentedControl
              id="media:view"
              label="View"
              value={props.viewMode}
              options={[
                { id: "list", label: "List", symbol: "list.bullet" },
                { id: "grid", label: "Grid", symbol: "square.grid.2x2" },
              ]}
              onChange={(mode) => props.onViewMode?.(mode as ViewMode)}
            />
          ) : null}
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
            metadata={{
              hostValue:
                Math.max(hostFilter.current, view.host_filter) * 65536 + clears,
              placeholder: "Search",
              search: true,
            }}
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
              label="This folder is empty"
              explanation="Drag media or files here."
              attr={{ symbol: "folder" }}
              action={null}
            />
          ) : (
            <EmptyState
              label="Add media"
              explanation="Drag files here or import from your computer."
              attr={{ symbol: "folder" }}
              action={null}
            />
          )
        ) : (
          <virtual-list
            count={count}
            range={[start, end]}
            itemSize={layout.rowHeight}
            metadata={{ fixedItems: layout.columns > 1 }}
            onRangeChange={(e) => {
              if (typeof e.width === "number") props.onWidth?.(e.width);
              if (!e.range) return;
              const [from, to] = e.range;
              if (
                from < 0 ||
                to < from ||
                to > count ||
                (to - from) * layout.columns > 4096
              )
                return;
              // The rows shown, with as many again on each side: scrolling
              // inside the page loaded asks for nothing (each page republishes
              // and redraws the panel), and the host scrolls the rows it has.
              // A page that reaches the last row asks for a first page's worth
              // more: media imported later joins the end, which the next
              // publication then brings without a scroll (the native model
              // clamps to what exists).
              const loadedRoom = askedPage.current.endsWith(":room");
              if (from >= start && to <= end && (to < count || loadedRoom))
                return;
              const span = Math.max(1, to - from);
              const first = Math.max(0, from - span);
              const last = Math.min(count, to + span);
              const room = last >= count ? FIRST_PAGE : 0;
              const offset = first * layout.columns;
              const limit = Math.min(
                4096,
                (last - first) * layout.columns + room,
              );
              const key = `${offset}:${limit}:${view.page.stamp.document}${room ? ":room" : ""}`;
              if (key === askedPage.current) return;
              askedPage.current = key;
              props.page(offset, limit);
            }}
          >
            {Array.from({ length: end - start }, (_, index) => {
              const rows = view.page.rows.slice(
                index * layout.columns,
                (index + 1) * layout.columns,
              );
              return (
                <div
                  key={start + index}
                  style={{
                    display: "flex",
                    // One column stacks full-width list rows; more columns lay cards out.
                    flexDirection: layout.columns === 1 ? "column" : "row",
                    flexGrow: layout.columns === 1 ? null : 1,
                    gap: token("metrics.inset"),
                  }}
                >
                  {layout.columns === 1
                    ? rows.map(renderRow)
                    : [
                        ...rows.map(renderCard),
                        // A short last row keeps its cards a column wide.
                        ...Array.from(
                          { length: layout.columns - rows.length },
                          (_, i) => (
                            <div
                              key={`space:${i}`}
                              style={{
                                flexGrow: 1,
                                flexShrink: 1,
                                flexBasis: token("metrics.thumbnail_min"),
                              }}
                            />
                          ),
                        ),
                      ]}
                </div>
              );
            })}
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
  layoutFor,
  viewMode: initialMode,
  onViewMode,
  loadViewMode,
  ...props
}: Omit<ProjectViewProps, "view" | "input" | "menu" | "page" | "onWidth"> & {
  service: ProjectService;
  /** The host's layout for a view at a width (`project.layout`); with it the
   * header offers the list/grid choice and the grid follows the width. */
  layoutFor?(mode: ViewMode, width: number | null): GridLayout;
  /** The open project's saved choice (asked again when another opens). */
  loadViewMode?(): Promise<ViewMode>;
}) {
  const [, update] = useState(0);
  useLayoutEffect(() => service.listen(() => update((n) => n + 1)), [service]);
  const [mode, setMode] = useState<ViewMode>(initialMode ?? "list");
  // Another project in this panel: its own list/grid choice.
  const documentId = service.view?.page.stamp.document;
  // `undefined` is never a document: the first one shown asks too.
  const shownDocument = useRef<number | null | undefined>(undefined);
  useLayoutEffect(() => {
    if (documentId === shownDocument.current) return;
    shownDocument.current = documentId;
    let live = true;
    void loadViewMode?.().then((next) => {
      if (live) setMode(next);
    });
    return () => {
      live = false;
    };
  }, [documentId]);
  const [width, setWidth] = useState<number | null>(null);
  const layout = layoutFor ? layoutFor(mode, width) : props.layout;
  const view = service.view;
  // A new column count regroups the rows: ask for the page from the start
  // of the row holding the first item shown, so rows begin at a column.
  const columns = useRef(layout.columns);
  useLayoutEffect(() => {
    if (columns.current === layout.columns) return;
    columns.current = layout.columns;
    if (!view) return;
    const offset =
      Math.floor(view.page.offset / layout.columns) * layout.columns;
    service.run(
      service.page(
        offset,
        // Never clamped to the items there are: media imported later
        // joins this page with the next publication.
        Math.min(
          4096,
          Math.max(FIRST_PAGE, view.page.rows.length + layout.columns),
        ),
        view.page.stamp,
      ),
    );
  }, [layout.columns]);
  return (
    <ProjectView
      {...props}
      layout={layout}
      viewMode={layoutFor ? mode : undefined}
      onViewMode={(next) => {
        setMode(next);
        onViewMode?.(next);
      }}
      onWidth={(next) => {
        if (next !== width) setWidth(next);
      }}
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
