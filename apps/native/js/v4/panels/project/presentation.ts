/** Copy and semantic values only; geometry and materials belong to the kit. */
import type { Asset, Availability, Command, Row } from "./contract";
export function clock(seconds: number) {
  const s = Math.max(0, Math.round(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}
/** Authored tagged formatter (AssetsPanel.svelte:116–130); no browser quirk. */
export function mediaSize(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB`;
}
const dimensions = (a: { width: number | null; height: number | null }) =>
  a.width !== null && a.height !== null ? `${a.width}×${a.height}` : null;
export function status(
  availability: Availability,
  a: Asset,
): {
  overlay: string;
  detail: string;
  tone: "danger" | "muted" | "warning";
} | null {
  if (typeof availability === "object" && "Changed" in availability)
    return {
      overlay: "",
      detail: `Changed · ${availability.Changed.source}`,
      tone: "warning",
    };
  // Models aren't probed as media, so they never leave Loading: say what
  // they are instead (the 3D layer itself reports any load failure).
  if (availability === "Loading" && a.kind === "model")
    return { overlay: "3D Model", detail: "", tone: "muted" };
  if (availability === "Loading")
    return { overlay: "Loading…", detail: "", tone: "muted" };
  if (availability === "Offline")
    return {
      overlay: "Media offline",
      detail: a.source_path
        ? `Missing · ${a.source_path}`
        : "Missing · locate the file to relink it",
      tone: "danger",
    };
  if (typeof availability === "object" && "Failed" in availability)
    return {
      overlay: "Media unavailable",
      detail: `Couldn’t load · ${availability.Failed}`,
      tone: "danger",
    };
  if (typeof availability === "object" && "Cloud" in availability) {
    const { provider, download } = availability.Cloud;
    return {
      overlay: download === "Downloading" ? "Downloading…" : "In cloud",
      detail:
        download === "Downloading"
          ? `Downloading from ${provider}…`
          : typeof download === "object"
            ? `Download failed · ${download.Failed}`
            : `Stored in ${provider}${a.source_path ? ` · ${a.source_path}` : ""}`,
      tone: "muted",
    };
  }
  return null;
}
/** A rendition's state in the row's quiet meta text. */
function rendition(state: unknown, label: string): string | null {
  if (state === "Queued") return `${label} Waiting`;
  if (typeof state !== "object" || state === null) return null;
  if ("Generating" in state) {
    const { percent } = (state as { Generating: { percent: number } }).Generating;
    return `Creating ${label} ${percent}%`;
  }
  if ("Ready" in state) return label;
  if ("Failed" in state) return `${label} Failed`;
  return null;
}
/** Proxy, optimized media and Always Play Original, as meta text. */
export function renditions(runtime: {
  proxy: unknown;
  optimized?: unknown;
  play_original?: boolean;
}): string {
  return [
    rendition(runtime.proxy, "Proxy"),
    rendition(runtime.optimized, "Optimized"),
    runtime.play_original ? "Plays Original" : null,
  ]
    .filter(Boolean)
    .join(" · ");
}
export function meta(row: Row): string {
  if ("Folder" in row.data) {
    const n = row.data.Folder.items;
    return `${n} ${n === 1 ? "item" : "items"}`;
  }
  if ("Composition" in row.data) {
    const c = row.data.Composition;
    return [
      dimensions(c),
      c.fps === null ? null : `${c.fps} fps`,
      c.duration === null ? null : clock(c.duration),
    ]
      .filter(Boolean)
      .join(" · ");
  }
  const [a, runtime] = row.data.Asset;
  const extra = renditions(runtime);
  const detail = status(runtime.availability, a)?.detail;
  if (detail) return [detail, extra].filter(Boolean).join(" · ");
  return (
    [
      a.format?.toLowerCase() === "svg" || /\.svg$/i.test(a.name)
        ? "SVG"
        : a.kind === "model"
          ? null
          : a.format?.toUpperCase(),
      runtime.hdr ?? null,
      a.kind === "audio" ? null : dimensions(a),
      a.duration === null ? null : clock(a.duration),
      a.kind === "model" && a.triangles !== null ? `${a.triangles} tris` : null,
      a.bytes === null ? null : mediaSize(a.bytes),
      extra,
    ]
      .filter(Boolean)
      .join(" · ") || (runtime.availability === "Ready" ? "Ready to use" : "")
  );
}
/** Spoken forms of the visible meta values (design review 2026-11-01). */
function spokenSeconds(seconds: number) {
  const s = Math.max(0, Math.round(seconds));
  return `${s} ${s === 1 ? "second" : "seconds"}`;
}
function spokenSize(bytes: number) {
  return mediaSize(bytes)
    .replace(" KB", " kilobytes")
    .replace(" MB", " megabytes");
}
export function accessibleName(row: Row): string {
  const selected = row.selected ? ["selected"] : [];
  if ("Folder" in row.data)
    return [row.data.Folder.name, "folder", meta(row), ...selected].join(", ");
  const a =
    "Composition" in row.data ? row.data.Composition : row.data.Asset[0];
  const parts = [
    a.name,
    "Composition" in row.data ? "composition" : row.data.Asset[0].kind,
  ];
  if (a.width !== null && a.height !== null)
    parts.push(`${a.width} by ${a.height}`);
  if (a.duration !== null) parts.push(spokenSeconds(a.duration));
  if ("Composition" in row.data) {
    if (row.data.Composition.open) parts.push("open in the timeline");
  } else {
    if (row.data.Asset[0].bytes !== null)
      parts.push(spokenSize(row.data.Asset[0].bytes));
    const availability = row.data.Asset[1].availability;
    if (availability === "Offline") parts.push("offline");
    if (availability === "Loading") parts.push("loading");
    if (typeof availability === "object" && "Changed" in availability)
      parts.push("changed");
    if (typeof availability === "object" && "Cloud" in availability)
      parts.push("in cloud");
    const extra = renditions(row.data.Asset[1]);
    if (extra) parts.push(extra.toLowerCase().replaceAll(" · ", ", "));
  }
  return [...parts, ...selected].join(", ");
}
export function commandLabel(command: Command, row?: Row, count = 0): string {
  if (typeof command === "string")
    return {
      DeleteAssets: count > 1 ? "Delete Selected Media…" : "Delete Media…",
      NewComposition: "New Composition…",
      ImportFiles: "Import Media…",
    }[command];
  if ("AddAsset" in command || "NestComposition" in command)
    return "Add to Timeline";
  if ("NewCompFromAsset" in command) return "New Comp from Selection";
  if ("Replace" in command)
    return row &&
      "Asset" in row.data &&
      row.data.Asset[1].availability === "Offline"
      ? "Locate File…"
      : "Replace File…";
  if ("Reveal" in command) return "Show in Finder";
  if ("Download" in command) return "Download from Cloud";
  if ("ImportFolder" in command)
    return row && "Folder" in row.data
      ? "Import Folder Into…"
      : "Import Folder…";
  if ("NewFolder" in command)
    return row && "Folder" in row.data ? "New Folder Inside" : "New Folder";
  if ("OpenFolder" in command) return "Open";
  if ("RenameFolder" in command) return "Rename";
  if ("DeleteFolder" in command) return "Delete Folder";
  if ("MoveOut" in command) return "Move Out of Folder";
  if ("OpenComposition" in command) return "Open in Timeline";
  if ("CompositionSettings" in command) return "Composition Settings…";
  if ("DuplicateComposition" in command) return "Duplicate";
  if ("Rendition" in command) {
    const r = command.Rendition;
    if (typeof r === "object") return "Always Play Original";
    return {
      CreateProxy: "Create Proxy Media",
      RebuildProxy: "Rebuild Proxy Media",
      DeleteProxy: "Delete Proxy Media",
      AttachProxy: "Attach Proxy Media…",
      CreateOptimized: "Create Optimized Media",
      DeleteOptimized: "Delete Optimized Media",
    }[r];
  }
  return "Delete Composition…";
}
export interface CardAction {
  label: string;
  /** SF Symbol for the over-image button (media-browser §2 Hover actions). */
  symbol: string;
  input: import("./contract").Input;
  disabled?: boolean;
}
export function cardActions(row: Row, compositions: number): CardAction[] {
  if ("Folder" in row.data)
    return [
      {
        label: "Delete Folder",
        symbol: "trash",
        input: { Invoke: { DeleteFolder: row.data.Folder.id } },
      },
    ];
  if ("Composition" in row.data) {
    const c = row.data.Composition;
    return [
      {
        label: "Add to Timeline",
        symbol: "plus",
        input: { Invoke: { NestComposition: c.id } },
        disabled: !c.can_nest,
      },
      {
        label: "Delete Composition…",
        symbol: "trash",
        input: { Invoke: { DeleteComposition: c.id } },
        disabled: compositions <= 1,
      },
    ];
  }
  const [a, r] = row.data.Asset;
  const result: CardAction[] = [];
  if (r.availability === "Ready") {
    if (a.source_path)
      result.push({
        label: "Show in Finder",
        symbol: "folder",
        input: { Invoke: { Reveal: a.id } },
      });
    result.push({
      label: "Add to Timeline",
      symbol: "plus",
      input: { Invoke: { AddAsset: a.id } },
    });
  } else if (
    r.availability === "Offline" ||
    (typeof r.availability === "object" &&
      ("Failed" in r.availability || "Changed" in r.availability))
  )
    result.push({
      label: "Locate File…",
      symbol: "link",
      input: { Invoke: { Replace: a.id } },
    });
  else if (typeof r.availability === "object" && "Cloud" in r.availability)
    result.push({
      label:
        typeof r.availability.Cloud.download === "object"
          ? "Retry Download"
          : "Download from Cloud",
      symbol: "icloud.and.arrow.down",
      input: { Invoke: { Download: a.id } },
      disabled: r.availability.Cloud.download === "Downloading",
    });
  result.push({
    label: "Delete Media",
    symbol: "trash",
    input: { DeleteAsset: a.id },
  });
  return result;
}

export type NativeMenuEntry = import("./contract").MenuEntry & {
  label: string;
  separatorBefore: boolean;
  shortcut?: "⌘N" | "⌘I";
};
/** Exact §4 group boundaries; no native host needs to guess copy or grouping. */
export function menuEntries(
  entries: import("./contract").MenuEntry[],
  row?: Row,
  count = 0,
): NativeMenuEntry[] {
  return entries.map((entry, index) => {
    const c = entry.command;
    const boundary =
      c === "ImportFiles" ||
      c === "DeleteAssets" ||
      (typeof c === "object" &&
        (("Rendition" in c &&
          (c.Rendition === "CreateProxy" ||
            c.Rendition === "CreateOptimized")) ||
          "Replace" in c ||
          "DeleteFolder" in c ||
          "CompositionSettings" in c ||
          "DeleteComposition" in c));
    return {
      ...entry,
      label: commandLabel(c, row, count),
      separatorBefore: index > 0 && boundary,
      ...(c === "NewComposition"
        ? { shortcut: "⌘N" as const }
        : c === "ImportFiles"
          ? { shortcut: "⌘I" as const }
          : {}),
    };
  });
}
