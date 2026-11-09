/** serde contract of pm-panel-media at 9ea399ed9. No writable project mirror. */
import type { Json } from "../../src/protocol";
export type ItemId =
  | { Asset: string }
  | { Folder: string }
  | { Composition: string };
export interface Stamp {
  project: string;
  document: number;
  revision: number;
}
export interface Modifiers {
  meta: boolean;
  control: boolean;
  shift: boolean;
  alt: boolean;
}
export type Availability =
  | "Unknown"
  | "Loading"
  | "Ready"
  | "Offline"
  | { Changed: { source: string } }
  | { Failed: string }
  | {
      Cloud: {
        provider: string;
        download: "Stored" | "Downloading" | { Failed: string };
      };
    };
export interface Asset {
  id: string;
  name: string;
  kind: string;
  folder: string | null;
  width: number | null;
  height: number | null;
  duration: number | null;
  bytes: number | null;
  format: string | null;
  source_path: string | null;
  triangles: number | null;
  vertices: number | null;
  references: number;
}
export interface Folder {
  id: string;
  name: string;
  parent: string | null;
  items: number;
}
export interface Composition {
  id: string;
  name: string;
  width: number | null;
  height: number | null;
  duration: number | null;
  fps: number | null;
  open: boolean;
  can_nest: boolean;
  uses: number;
  background?: string | null;
  poster?: string | null;
}
export interface Row {
  id: ItemId;
  data:
    | {
        Asset: [
          Asset,
          {
            availability: Availability;
            proxy: Json;
            /** Optimized media; absent before renditions were tracked. */
            optimized?: Json;
            play_original?: boolean;
            poster: string | null;
            progress?: { completed: number; total: number } | null;
          },
        ];
      }
    | { Folder: Folder }
    | { Composition: Composition };
  selected: boolean;
  focused: boolean;
  tab_stop: boolean;
  folder_path?: string[];
}
export interface Page {
  stamp: Stamp;
  total: number;
  offset: number;
  rows: Row[];
  next: number | null;
}
export type Command =
  | "DeleteAssets"
  | "NewComposition"
  | "ImportFiles"
  | { AddAsset: string }
  | { NewCompFromAsset: string }
  | { Replace: string }
  | { Reveal: string }
  | { Download: string }
  | { ImportFolder: string | null }
  | { NewFolder: string | null }
  | { OpenFolder: string }
  | { RenameFolder: string }
  | { DeleteFolder: string }
  | { MoveOut: ItemId }
  | { OpenComposition: string }
  | { NestComposition: string }
  | { CompositionSettings: string }
  | { Rendition: RenditionAction }
  | { DuplicateComposition: string }
  | { DeleteComposition: string };
export type Key =
  | "A"
  | "Left"
  | "Right"
  | "Up"
  | "Down"
  | "Home"
  | "End"
  | "Enter"
  | "Space"
  | "Delete"
  | "Backspace"
  | "Escape";
export type Input =
  | { GridColumns: number }
  | {
      GridKey: {
        target: "List" | { Row: ItemId };
        key: Key;
        modifiers: Modifiers;
        columns: number;
      };
    }
  | "Clear"
  | "CancelRename"
  | "DragEnd"
  | "FilesEnter"
  | "FilesLeave"
  | "DropLeave"
  | {
      Pointer: {
        id: ItemId;
        phase: "Down" | "Click" | "DoubleClick";
        button: number;
        nested_control: boolean;
        modifiers: Modifiers;
      };
    }
  | {
      Key: { target: "List" | { Row: ItemId }; key: Key; modifiers: Modifiers };
    }
  | { Invoke: Command }
  | { OpenFolder: string | null }
  | { DeleteAsset: string }
  | { BeginRename: string }
  | { CommitRename: { folder: string; value: string } }
  | {
      Filter: {
        query: string;
        kind: null;
        missing_only: boolean;
        proxy: null;
      };
    }
  | { ConfirmDelete: { token: number; approved: boolean } }
  | { DragStart: { id: ItemId; nested_control: boolean } }
  | { DragOver: { target: string | null; files: boolean } }
  | { Drop: { target: string | null; files: string[]; modifiers: Modifiers } }
  | { BeginMarquee: { pointer: number; modifiers: Modifiers } }
  | { MarqueeHits: { pointer: number; activated: boolean; ids: string[] } }
  | { EndMarquee: { pointer: number | null } }
  | {
      MarqueeEdge: {
        pointer: number;
        activated: boolean;
        direction: "Before" | "After";
      };
    };
export type RenditionAction =
  | "CreateProxy"
  | "RebuildProxy"
  | "DeleteProxy"
  | "AttachProxy"
  | "CreateOptimized"
  | "DeleteOptimized"
  | { PlayOriginal: boolean };
export interface MenuEntry {
  command: Command;
  enabled: boolean;
  /** A toggle that is on (Always Play Original). */
  checked?: boolean;
}
export interface Projection {
  page: Page;
  folder: string | null;
  crumbs: Folder[];
  selected_count: number;
  focused: ItemId | null;
  renaming: string | null;
  drag: Json;
  drop_target: null | { folder: string | null };
  external_drag: boolean;
  composition_count: number;
  /** Only missing files are listed (opened from the status line). */
  missing_only: boolean;
  /** The search the model applies; the field follows it. */
  query: string;
  /** Moves when the host set the filter; only then does the field follow `query`. */
  host_filter: number;
}
export type Action =
  | { Page: { offset: number; limit: number } }
  | { Input: Input }
  | { Menu: ItemId | null };
export interface Reply {
  view: Projection;
  menu?: { owner: Stamp; entries: MenuEntry[]; effects: Json[] };
  outcome?: Json;
}
export const itemKey = (id: ItemId) => JSON.stringify(id);
export const itemName = (row: Row) =>
  "Asset" in row.data
    ? row.data.Asset[0].name
    : "Folder" in row.data
      ? row.data.Folder.name
      : row.data.Composition.name;
export const sameItem = (a: ItemId | null, b: ItemId | null) =>
  itemKey(a as ItemId) === itemKey(b as ItemId);
