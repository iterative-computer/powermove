import { afterEach, it, expect, vi } from "vitest";
afterEach(() => vi.unstubAllGlobals());
it("serializes official API data, resolves instances, dedupes image bytes and exports only unsupported nodes", async () => {
  const image = {
    type: "IMAGE",
    imageHash: "synthetic-image",
    scaleMode: "FILL",
    visible: true,
    opacity: 1,
    blendMode: "NORMAL",
  };
  const make = (id: string, patch: any = {}) => ({
    id,
    type: "RECTANGLE",
    name: id,
    width: 100,
    height: 60,
    absoluteTransform: [
      [1, 0, 0],
      [0, 1, 0],
    ],
    fills: [{ type: "SOLID", color: { r: 1, g: 0, b: 0 } }],
    strokes: [],
    effects: [],
    visible: true,
    opacity: 1,
    blendMode: "NORMAL",
    absoluteRenderBounds: { x: 0, y: 0, width: 100, height: 60 },
    exportAsync: vi.fn(async () =>
      Uint8Array.from([137, 80, 78, 71, 13, 10, 26, 10]),
    ),
    ...patch,
  });
  const editable = make("editable"),
    first = make("image-a", { fills: [image] }),
    second = make("image-b", { fills: [image] });
  const instance = make("instance", {
    type: "INSTANCE",
    children: [editable, first, second],
  });
  const page = {
    id: "page",
    parent: null,
    children: [instance],
    selection: [instance],
  };
  instance.parent = page;
  for (const n of instance.children) n.parent = instance;
  const bytes = vi.fn(async () => Uint8Array.from([1, 2, 3]));
  vi.stubGlobal("__html__", "");
  vi.stubGlobal("figma", {
    showUI: vi.fn(),
    get fileKey() {
      throw new Error("Private fileKey must not be read");
    },
    root: { getPluginData: () => "synthetic-file", setPluginData: vi.fn() },
    on: vi.fn(),
    notify: vi.fn(),
    currentPage: page,
    ui: { postMessage: vi.fn() },
    clientStorage: { getAsync: async () => null },
    getImageByHash: () => ({ getBytesAsync: bytes }),
    base64Encode: (b: Uint8Array) => Buffer.from(b).toString("base64"),
  });
  const pluginPath = new URL(
    "../../../../../packages/figma-plugin/code.ts",
    import.meta.url,
  ).pathname;
  const { serializeSelection } = await import(pluginPath);
  const doc = await serializeSelection();
  expect(doc.nodes[0]!.children).toHaveLength(3);
  expect(doc.images).toEqual({ "synthetic-image": "AQID" });
  expect(bytes).toHaveBeenCalledTimes(1);
  expect(editable.exportAsync).not.toHaveBeenCalled();
  expect(instance.exportAsync).not.toHaveBeenCalled();
  expect(first.exportAsync).not.toHaveBeenCalled();
  expect(second.exportAsync).not.toHaveBeenCalled();
});

it("creates and reuses a document identity without private fileKey access", async () => {
  const values = new Map<string, string>();
  const root = {
    getPluginData: (key: string) => values.get(key) ?? "",
    setPluginData: (key: string, value: string) => values.set(key, value),
  };
  const file = new URL(
    "../../../../../packages/figma-plugin/identity.ts",
    import.meta.url,
  ).pathname;
  const { figmaFileId } = await import(file);
  const first = figmaFileId(root);
  expect(first).toMatch(/^pm-/);
  expect(values.get("powermove-file-id")).toBe(first);
  expect(figmaFileId(root)).toBe(first);
  expect(
    figmaFileId({ getPluginData: () => "", setPluginData: () => {} }),
  ).not.toBe(first);
});

it("notifies the designer when too many nodes are selected", async () => {
  vi.resetModules();
  const notify = vi.fn(),
    postMessage = vi.fn();
  const nodes = Array.from({ length: 5001 }, (_, i) => ({
    id: String(i),
    parent: null,
    type: "RECTANGLE",
    name: "Synthetic",
    absoluteTransform: [
      [1, 0, 0],
      [0, 1, 0],
    ],
    width: 10,
    height: 10,
    fills: [],
    strokes: [],
    effects: [],
  }));
  const host: any = {
    showUI: () => {},
    on: () => {},
    root: {
      getPluginData: () => "synthetic-document",
      setPluginData: () => {},
    },
    currentPage: { selection: nodes },
    clientStorage: { getAsync: async () => null },
    ui: { postMessage },
    notify,
  };
  vi.stubGlobal("__html__", "");
  vi.stubGlobal("figma", host);
  const file = new URL(
    "../../../../../packages/figma-plugin/code.ts",
    import.meta.url,
  ).pathname;
  await import(file);
  await host.ui.onmessage({ type: "send" });
  expect(notify).toHaveBeenCalledWith("Select fewer layers.", { error: true });
  expect(postMessage).toHaveBeenCalledWith({
    type: "error",
    text: "Select fewer layers.",
  });
});

it("loads pairing from client storage after the UI is ready and follows selection changes", async () => {
  vi.resetModules();
  const postMessage = vi.fn(),
    stored = new Map([["powermove-token", "a".repeat(64)]]);
  let selectionChanged: () => void = () => {};
  const host: any = {
    showUI: () => {},
    on: (event: string, callback: () => void) => {
      if (event === "selectionchange") selectionChanged = callback;
    },
    currentPage: { selection: [{ id: "1" }] },
    clientStorage: {
      getAsync: async (key: string) => stored.get(key),
      setAsync: async (key: string, value: string) => {
        stored.set(key, value);
      },
      deleteAsync: async (key: string) => {
        stored.delete(key);
      },
    },
    ui: { postMessage },
  };
  vi.stubGlobal("__html__", "");
  vi.stubGlobal("figma", host);
  const file = new URL(
    "../../../../../packages/figma-plugin/code.ts",
    import.meta.url,
  ).pathname;
  await import(file);
  await host.ui.onmessage({ type: "ready" });
  expect(postMessage).toHaveBeenCalledWith({
    type: "token",
    token: "a".repeat(64),
  });
  host.currentPage.selection = [{ id: "1" }, { id: "2" }];
  selectionChanged();
  expect(postMessage).toHaveBeenLastCalledWith({ type: "count", count: 2 });
  await host.ui.onmessage({ type: "token", token: "b".repeat(64) });
  expect(stored.get("powermove-token")).toBe("b".repeat(64));
  await host.ui.onmessage({ type: "token", token: null });
  expect(stored.size).toBe(0);
});

it("uses and removes a computed stroke outline for mixed joins without touching the source", async () => {
  vi.resetModules();
  const remove = vi.fn(),
    stroke = { type: "SOLID", color: { r: 0, g: 0, b: 1 } };
  const source: any = {
    id: "1",
    type: "VECTOR",
    name: "Mixed joins",
    width: 100,
    height: 60,
    absoluteTransform: [
      [1, 0, 0],
      [0, 1, 0],
    ],
    fills: [{ type: "SOLID", color: { r: 1, g: 0, b: 0 } }],
    strokes: [stroke],
    strokeWeight: 4,
    strokeJoin: Symbol("mixed"),
    vectorPaths: [{ data: "M0 0L100 0L50 60Z", windingRule: "NONZERO" }],
    effects: [],
    exportAsync: vi.fn(),
    outlineStroke: vi.fn(() => ({
      absoluteTransform: [
        [1, 0, -2],
        [0, 1, -2],
      ],
      fillGeometry: [{ data: "M0 0H104V64H0Z", windingRule: "NONZERO" }],
      remove,
    })),
  };
  const page = { selection: [source], children: [source] };
  source.parent = page;
  vi.stubGlobal("__html__", "");
  vi.stubGlobal("figma", {
    showUI: () => {},
    on: () => {},
    root: { getPluginData: () => "synthetic", setPluginData: () => {} },
    currentPage: page,
    ui: { postMessage: () => {} },
    clientStorage: { getAsync: async () => null },
    base64Encode: (bytes: Uint8Array) => Buffer.from(bytes).toString("base64"),
  });
  const file = new URL(
    "../../../../../packages/figma-plugin/code.ts",
    import.meta.url,
  ).pathname;
  const { serializeSelection } = await import(file);
  const doc = await serializeSelection();
  expect(remove).toHaveBeenCalledTimes(1);
  expect(source.exportAsync).not.toHaveBeenCalled();
  expect(source.strokeJoin).toBeTypeOf("symbol");
  expect(doc.nodes[0].strokePaths[0].data).toContain("M -2 -2");
  expect(doc.nodes[0].unsupported).toEqual([]);
});

it("exports only a node whose image hash has no asset bytes", async () => {
  vi.resetModules();
  const exportAsync = vi.fn(async () =>
    Uint8Array.from([137, 80, 78, 71, 13, 10, 26, 10]),
  );
  const node: any = {
    id: "missing-image",
    type: "RECTANGLE",
    name: "missing",
    width: 40,
    height: 30,
    absoluteTransform: [
      [1, 0, 0],
      [0, 1, 0],
    ],
    fills: [{ type: "IMAGE", imageHash: "not-present", scaleMode: "FILL" }],
    strokes: [],
    effects: [],
    absoluteRenderBounds: { x: 0, y: 0, width: 40, height: 30 },
    exportAsync,
  };
  const page = { id: "page", children: [node], selection: [node] };
  node.parent = page;
  vi.stubGlobal("__html__", "");
  vi.stubGlobal("figma", {
    showUI: vi.fn(),
    on: vi.fn(),
    notify: vi.fn(),
    root: { getPluginData: () => "synthetic", setPluginData: vi.fn() },
    currentPage: page,
    ui: { postMessage: vi.fn() },
    clientStorage: { getAsync: async () => null },
    getImageByHash: () => null,
    base64Encode: (b: Uint8Array) => Buffer.from(b).toString("base64"),
  });
  const file = new URL(
    "../../../../../packages/figma-plugin/code.ts",
    import.meta.url,
  ).pathname;
  const { serializeSelection } = await import(file);
  const doc = await serializeSelection();
  expect(exportAsync).toHaveBeenCalledTimes(1);
  expect(doc.nodes[0].raster).toBeDefined();
  expect(doc.images).toEqual({});
});

it("preserves resolved AUTO line spacing for auto-width text without changing explicit spacing", async () => {
  vi.resetModules();
  const make = (id: string, text: string, height: number, lineHeight: any) => ({
    id, type: "TEXT", name: id, width: 240, height,
    absoluteTransform: [[1, 0, 12000], [0, 1, -4000]],
    characters: text, textAutoResize: "WIDTH_AND_HEIGHT", textAlignHorizontal: "LEFT",
    textAlignVertical: "TOP", paragraphSpacing: 12, fills: [], strokes: [], effects: [],
    getStyledTextSegments: () => [{start: 0, end: text.length, fontName: {family: "Synthetic tall font", style: "Regular"},
      fontSize: 80, fills: [], lineHeight, letterSpacing: {unit: "PIXELS", value: 0}, listOptions: {type: "NONE"}}],
  });
  const nodes = [make("single", "Label", 160, {unit: "AUTO"}),
    make("multiline", "One\nTwo", 332, {unit: "AUTO"}),
    make("explicit", "Label", 100, {unit: "PIXELS", value: 100})];
  const page: any = {id: "page", children: nodes, selection: nodes};
  nodes.forEach((node: any) => node.parent = page);
  vi.stubGlobal("__html__", "");
  vi.stubGlobal("figma", {showUI: vi.fn(), on: vi.fn(), currentPage: page,
    root: {getPluginData: () => "synthetic", setPluginData: vi.fn()}, ui: {postMessage: vi.fn()}});
  const file = new URL("../../../../../packages/figma-plugin/code.ts", import.meta.url).pathname;
  const {serializeSelection} = await import(file);
  const doc = await serializeSelection();
  expect(doc.nodes.map((n: any) => n.text.leading)).toEqual([160, 160, 100]);
  expect(doc.nodes.map((n: any) => n.text.runs[0].leading)).toEqual([160, 160, 100]);
});
