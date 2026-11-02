import type {
  LayerRow,
  NativeTimelineModel,
  Intent,
  TimelineFrame,
  WindowRequest,
  Row,
} from "../model";

/** Synthetic presentation facts; not a JS implementation of Rust behaviour. */
export function layer(id = "headline", index = 1): LayerRow {
  return {
    id: `layer:${id}`,
    kind: "layer",
    layer: id,
    label: "Headline",
    accessibilityLabel: "Headline, text layer, 0:00 to 0:10",
    depth: 0,
    index,
    canOpen: true,
    expanded: false,
    selected: false,
    focused: false,
    hovered: false,
    locked: false,
    visible: true,
    solo: false,
    transformable: true,
    parent: { name: "None", set: false },
    media: "ready",
  };
}
export function frame(patch: Partial<TimelineFrame> = {}): TimelineFrame {
  return {
    document: "18446744073709551614",
    revision: "9007199254740993",
    hasComposition: true,
    mode: "Layers",
    view: "DopeSheet",
    graphEmpty: true,
    playing: false,
    snapping: true,
    timecode: "00:00:04:15",
    spokenTime: "0 hours 0 minutes 4 seconds 15 frames",
    frameRate: "30 fps",
    tabs: [
      { id: "launch", name: "Launch Film", active: true, showClose: false },
    ],
    window: { total: 1, first: 0, rows: [layer()] },
    indexDigits: 1,
    selection: null,
    renderSummary: "Rendered 0:00 to 0:12, rendering 0:12 to 0:14",
    feedback: null,
    ...patch,
  };
}
function rows(...values: Row[]) {
  return { total: values.length, first: 0, rows: values };
}
export const states: Record<string, () => TimelineFrame> = {
  "empty-no-project": () =>
    frame({
      document: null,
      hasComposition: false,
      tabs: [],
      window: rows(),
      renderSummary: "",
    }),
  "empty-composition": () => frame({ window: rows() }),
  loading: () => frame({ window: rows({ ...layer(), media: "loading" }) }),
  selected: () => frame({ window: rows({ ...layer(), selected: true }) }),
  hover: () => frame({ window: rows({ ...layer(), hovered: true }) }),
  focus: () => frame({ window: rows({ ...layer(), focused: true }) }),
  disabled: () =>
    frame({
      window: rows(
        { ...layer(), locked: true },
        {
          id: "property:opacity",
          kind: "property",
          layer: "headline",
          path: "opacity",
          label: "Opacity",
          accessibilityLabel: "Opacity, 100 percent",
          depth: 1,
          selected: false,
          focused: false,
          hovered: false,
          locked: true,
          animated: true,
          keyAtPlayhead: true,
          previous: false,
          next: true,
          readout: "100 %",
          numeric: {
            binding: "native:opacity",
            value: 100,
            unit: "%",
            step: 1,
          },
        },
      ),
    }),
  error: () =>
    frame({
      window: rows({ ...layer(), media: "offline" }),
      announcement: { sequence: "2", value: { kind: "outsideComposition" } },
      feedback: { failedEdit: "outsideComposition" },
    }),
  "drag-drop": () =>
    frame({
      window: rows({ ...layer(), hovered: true }),
      feedback: { drop: { row: "layer:headline", frame: 12, asFolder: true } },
    }),
  "multi-select": () =>
    frame({
      window: rows(
        { ...layer(), selected: true },
        { ...layer("kicker", 2), selected: true },
      ),
      selection: { kind: "layers", count: 2 },
    }),
  playing: () =>
    frame({
      playing: true,
      timecode: "00:00:04:16",
      spokenTime: "0 hours 0 minutes 4 seconds 16 frames",
    }),
  "60k-rows": () =>
    frame({
      indexDigits: 5,
      window: {
        total: 60_000,
        first: 29_990,
        rows: Array.from({ length: 30 }, (_, i) =>
          layer(`row-${29_990 + i}`, 29_991 + i),
        ),
      },
    }),
  "graph-empty": () => frame({ view: "Combined", graphEmpty: true }),
  "graph-selected": () =>
    frame({
      view: "Graph",
      graphEmpty: false,
      feedback: {
        graph: {
          focusedCurve: ["headline", "opacity"],
          selectedKeys: ["key-original"],
        },
      },
    }),
  tracks: () =>
    frame({
      mode: "Tracks",
      window: rows({
        id: "track:audio:0",
        kind: "track",
        track: "audio:0",
        label: "A1",
        accessibilityLabel: "A1",
        depth: 0,
        selected: false,
        focused: false,
        hovered: false,
        locked: false,
        audio: true,
        enabled: true,
        expanded: false,
      }),
    }),
  "time-edit": () =>
    frame({ timeEditor: { binding: "native:time", value: "4.5s" } }),
  "rename-and-tabs": () =>
    frame({
      window: rows({
        ...layer(),
        nameEditor: { binding: "native:name", value: "Headline" },
      }),
      tabs: [
        { id: "launch", name: "Launch Film", active: true, showClose: true },
        { id: "end", name: "Endcard", active: false, showClose: false },
      ],
    }),
  "scale-compound": () =>
    frame({
      window: rows({
        id: "property:scale",
        kind: "property",
        layer: "headline",
        path: "scale",
        label: "Scale",
        accessibilityLabel:
          "Scale, 100 by 100 percent, animated, keyframe at playhead",
        depth: 1,
        selected: false,
        focused: false,
        hovered: false,
        locked: false,
        animated: true,
        keyAtPlayhead: true,
        previous: true,
        next: true,
        readout: "100, 100 %",
        scaleLinked: true,
      }),
    }),
};
export class InspectModel implements NativeTimelineModel {
  value = frame();
  actions: Intent[] = [];
  windows: WindowRequest[] = [];
  disposals = 0;
  reject?: Error;
  wait?: Promise<void>;
  async inspect(window: WindowRequest) {
    this.windows.push({ ...window });
    return this.value;
  }
  async act(intent: Intent) {
    this.actions.push(intent);
    await this.wait;
    if (this.reject) throw this.reject;
  }
  async dispose() {
    this.disposals++;
  }
}
