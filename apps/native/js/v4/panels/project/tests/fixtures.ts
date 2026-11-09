import type { Asset, Availability, Projection, Row } from "../contract";
export const owner = { project: "P", document: 1, revision: 1 };
export function asset(id = "a", availability: Availability = "Ready"): Row {
  const a: Asset = {
    id,
    name: `${id}.mov`,
    kind: "video",
    folder: null,
    width: 1920,
    height: 1080,
    duration: 12,
    bytes: 48 * 1024 * 1024,
    format: "mov",
    source_path: `/synthetic/${id}.mov`,
    triangles: null,
    vertices: null,
    references: 0,
  };
  return {
    id: { Asset: id },
    data: { Asset: [a, { availability, proxy: "Unknown", poster: null }] },
    selected: false,
    focused: false,
    tab_stop: true,
  };
}
export function folder(): Row {
  return {
    id: { Folder: "f" },
    data: { Folder: { id: "f", name: "Shots", parent: null, items: 4 } },
    selected: false,
    focused: false,
    tab_stop: false,
  };
}
export function comp(): Row {
  return {
    id: { Composition: "C" },
    data: {
      Composition: {
        id: "C",
        name: "Launch Film",
        width: 1920,
        height: 1080,
        duration: 20,
        fps: 30,
        open: true,
        can_nest: false,
        uses: 0,
      },
    },
    selected: false,
    focused: false,
    tab_stop: false,
  };
}
export function projection(rows: Row[] = []): Projection {
  return {
    page: { stamp: owner, total: rows.length, offset: 0, rows, next: null },
    folder: null,
    crumbs: [],
    selected_count: rows.filter((r) => r.selected && "Asset" in r.id).length,
    focused: rows.find((r) => r.focused)?.id ?? null,
    renaming: null,
    drag: null,
    drop_target: null,
    external_drag: false,
    composition_count: 1,
    missing_only: false,
  };
}
export const project = {
  id: "P",
  compId: "C",
  compName: "Main",
  w: 1920,
  h: 1080,
  dur: 20,
  fps: 30,
  assets: {
    a: {
      id: "a",
      name: "Alpha.mov",
      kind: "video",
      dur: 12,
      w: 1920,
      h: 1080,
      size: 48 * 1024 * 1024,
      sourcePath: "/synthetic/Alpha.mov",
    },
    b: { id: "b", name: "Beta.wav", kind: "audio", dur: 12 },
    d: { id: "d", name: "Delta.mov", kind: "video", dur: 12 },
    nested: {
      id: "nested",
      name: "Nested.mov",
      kind: "video",
      folder: "inner",
    },
  },
  assetFolders: {
    f: { id: "f", name: "Shots", parent: null },
    inner: { id: "inner", name: "Inside", parent: "f" },
  },
  layers: [{ id: "layer", type: "video", from: 0, dur: 12, d: { asset: "a" } }],
  comps: {},
};
