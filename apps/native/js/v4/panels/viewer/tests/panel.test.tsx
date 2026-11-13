import { beforeAll, test, expect } from "bun:test";
import { ViewerPanel, type ViewerActions } from "../panel";
import { snapshot, zoomTitle, resolutionTitle, percent } from "../contract";
import type { ViewerSnapshot } from "../contract";
import { binaries, fixture, recorder, replay } from "./helpers";

// A cold Rust build is setup work, not part of a five-second UI assertion.
beforeAll(() => {
  binaries();
}, 120_000);

const actions: ViewerActions = { drive() {}, open() {}, command() {} };
const cases: [string, (s: ViewerSnapshot) => void][] = [
  [
    "empty-no-project",
    (s) => {
      s.model = null;
      s.host.frame.resource = null;
    },
  ],
  [
    "empty-composition",
    (s) => {
      s.model!.empty = true;
      s.model!.selectionCount = 0;
    },
  ],
  [
    "loading-first-frame",
    (s) => {
      s.host.frame.resource = null;
      s.host.frame.loading = true;
    },
  ],
  [
    "loading-retains-last-good",
    (s) => {
      s.host.frame.loading = true;
    },
  ],
  [
    "selected",
    (s) => {
      s.model!.selectionCount = 1;
    },
  ],
  [
    "hover",
    (s) => {
      s.host.interaction = "hover";
    },
  ],
  [
    "focus",
    (s) => {
      s.host.interaction = "focus";
    },
  ],
  // No-project hides the bar (§3); there are no focusable disabled pop-ups.
  [
    "disabled-no-project",
    (s) => {
      s.model = null;
      s.host.frame.resource = null;
    },
  ],
  [
    "error-last-good",
    (s) => {
      s.host.frame.failed = true;
    },
  ],
  [
    "drag-media",
    (s) => {
      s.host.interaction = "media-drop";
    },
  ],
  [
    "drag-effect",
    (s) => {
      s.host.interaction = "effect-drop";
    },
  ],
  [
    "multi-select",
    (s) => {
      s.model!.selectionCount = 2;
    },
  ],
  [
    "source-image",
    (s) => {
      s.host.source = {
        id: "a",
        name: "Still.png",
        kind: "image",
        time: 0,
        duration: null,
        playing: false,
        epoch: "1",
      };
    },
  ],
  [
    "source-video",
    (s) => {
      s.host.source = {
        id: "a",
        name: "Take.mov",
        kind: "video",
        time: 0.125,
        duration: 3,
        playing: false,
        epoch: "2",
      };
    },
  ],
  [
    "source-audio",
    (s) => {
      s.host.source = {
        id: "a",
        name: "VO_take03.wav",
        kind: "audio",
        time: 0.125,
        duration: 3,
        playing: true,
        epoch: "3",
      };
    },
  ],
  [
    // The native timecode layer draws the time while playing; the bar's
    // readout steps aside and Play reads Pause.
    "transport-playing-looping",
    (s) => {
      s.host.transport!.playing = true;
      s.host.transport!.looping = true;
    },
  ],
  [
    "playing-auto-half",
    (s) => {
      s.model!.effectiveQuality = 0.5;
    },
  ],
  [
    "playing-auto-full",
    (s) => {
      s.model!.effectiveQuality = 1;
    },
  ],
  [
    "focus-workspace",
    (s) => {
      s.host.presentation = "Focus";
    },
  ],
  [
    "full-screen",
    (s) => {
      s.host.presentation = "FullScreen";
    },
  ],
  [
    "lost-canvas",
    (s) => {
      s.model!.canvasLost = true;
    },
  ],
  [
    "hidden-controls",
    (s) => {
      s.model!.showControls = false;
    },
  ],
  [
    "custom-zoom",
    (s) => {
      s.model!.fit = false;
      s.model!.shownZoom = 0.54321;
    },
  ],
  [
    "60000-selected-no-row-list",
    (s) => {
      s.model!.selectionCount = 60_000;
    },
  ],
];
for (const [name, change] of cases) {
  test(`wire tree / AX: ${name} (placeholders do not certify rendering)`, async () => {
    const state = fixture();
    change(state);
    const { root, commits } = recorder();
    await root.render(
      <ViewerPanel state={snapshot(state)} actions={actions} />,
    );
    const result = replay(commits).at(-1);
    expect(result.accepted).toBe(true);
    expect(result.wireDiagnostics).toEqual([]);
    for (const appearance of result.appearances)
      expect(appearance.diagnostics).toEqual([]);
    expect(result.nodes).toMatchSnapshot(name);
    const nodes = Object.values(result.nodes) as any[];
    const focus = result.focusOrder.map(
      (id: number) => result.nodes[id].props.id,
    );
    const lost =
      state.model && !state.host.source && state.model.canvasLost
        ? ["viewer.canvas.return"]
        : [];
    expect(focus).toEqual(
      !state.model
        ? ["viewer.open"]
        : state.host.presentation !== "Docked"
          ? [
              "viewer.canvas",
              ...lost,
              "viewer.channels",
              "viewer.zoom",
              "viewer.quality",
            ]
          : [
              ...(state.host.source
                ? ["viewer.source.close"]
                : [
                    "gotoStart",
                    "prevFrame",
                    "play",
                    "nextFrame",
                    "gotoEnd",
                    "loop",
                  ].map((id) => `viewer.transport.${id}`)),
              "viewer.channels",
              "viewer.zoom",
              "viewer.quality",
              "viewer.canvas",
              ...lost,
            ],
    );
    for (const { accessibility: ax } of result.appearances) {
      const byId = (id: string) =>
        ax.find((n: any) => n.id === nodes.find((n) => n.props.id === id)?.id);
      if (state.model) {
        expect(byId("viewer.canvas")).toMatchObject({
          role: "Group",
          label: "Viewer",
          disabled: false,
        });
        expect(byId("viewer.canvas").value).toContain(
          `${state.model.selectionCount} ${state.model.selectionCount === 1 ? "layer" : "layers"} selected`,
        );
        {
          // Docked bar or Focus/full-screen overlay: same controls, same labels.
          expect(byId("viewer.zoom")).toMatchObject({
            role: "ComboBox",
            label:
              "Zoom, " +
              (state.model.fit ? "Fit, " : "") +
              percent(state.model.shownZoom) +
              " percent",
          });
          expect(byId("viewer.quality").label).toContain("Preview resolution");
        }
        if (state.host.source && state.host.presentation === "Docked")
          expect(byId("viewer.source.close")).toMatchObject({
            role: "Button",
            label: "Back to Composition",
          });
      } else
        expect(byId("viewer.open")).toMatchObject({
          role: "Button",
          label: "Open…",
        });
    }
    expect(nodes.length).toBeLessThan(40);
    await root.dispose();
  });
}
test("popup copy is exact; custom zoom has no invented menu item", async () => {
  const state = fixture();
  state.model!.fit = false;
  state.model!.shownZoom = 0.54321;
  const { root } = recorder();
  await root.render(<ViewerPanel state={state} actions={actions} />);
  const zoom = root.inspect().nodes.find((n) => n.props.id === "viewer.zoom")!;
  expect((zoom.props.options as any[]).map((o) => o.label)).toEqual([
    "Fit",
    "5 %",
    "12.5 %",
    "25 %",
    "50 %",
    "100 %",
    "200 %",
    "400 %",
    "800 %",
    "Zoom In",
    "Zoom Out",
    "Actual Size",
    "Fit Composition",
  ]);
  expect(zoom.props.value).toBe(0.54321);
  expect(zoomTitle(state.model!)).toBe("54 %");
  state.model!.shownZoom = 0.125;
  expect(zoomTitle(state.model!)).toBe("12.5 %");
  expect(resolutionTitle(state.model!)).toBe("Auto (Half)");
  state.model!.effectiveQuality = 1;
  expect(resolutionTitle(state.model!)).toBe("Auto");
  state.model!.effectiveQuality = 0.25;
  expect(resolutionTitle(state.model!)).toBe("Auto (Quarter)");
  // Design review D2: intermediate adaptive scales show as a percentage.
  state.model!.effectiveQuality = 0.75;
  expect(resolutionTitle(state.model!)).toBe("Auto (75 %)");
  await root.dispose();
});
test("every zoom and resolution change emits the matching model ID once", async () => {
  const seen: unknown[][] = [];
  const { root } = recorder();
  await root.render(
    <ViewerPanel
      state={fixture()}
      actions={{
        open() {},
        command() {},
        drive: (...args) => {
          seen.push(args);
        },
      }}
    />,
  );
  const zoom = root.inspect().nodes.find((n) => n.props.id === "viewer.zoom")!;
  const quality = root
    .inspect()
    .nodes.find((n) => n.props.id === "viewer.quality")!;
  for (const value of [
    "fit",
    0.05,
    0.125,
    0.25,
    0.5,
    1,
    2,
    4,
    8,
    "viewer.zoom.in",
    "viewer.zoom.out",
    "actual",
    "viewer.fit",
  ])
    await root.drive(zoom.id, "change", { value });
  expect(seen).toEqual([
    ["viewer.fit"],
    ...[0.05, 0.125, 0.25, 0.5, 1, 2, 4, 8].map((v) => ["viewer.zoom", v]),
    ["viewer.zoom.in"],
    ["viewer.zoom.out"],
    ["viewer.zoom", 1],
    ["viewer.fit"],
  ]);
  seen.length = 0;
  for (const value of ["auto", "1", "0.5", "0.25", "forged", null])
    await root.drive(quality.id, "change", { value });
  expect(seen).toEqual(
    ["auto", "1", "0.5", "0.25"].map((v) => ["viewer.quality", v]),
  );
  await root.drive(zoom.id, "change", { value: 7 });
  expect(seen).toHaveLength(4);
  await root.dispose();
});
test("source Back and empty Open callbacks; no visible viewer title or unrelated controls", async () => {
  const seen: string[] = [];
  const state = fixture();
  state.host.source = {
    id: "a",
    name: "Audio.wav",
    kind: "audio",
    time: 0,
    duration: 1,
    playing: false,
    epoch: "1",
  };
  const { root } = recorder();
  const actions = {
    command() {},
    drive: (id: string) => {
      seen.push(id);
    },
    open: () => {
      seen.push("open");
    },
  };
  await root.render(<ViewerPanel state={state} actions={actions} />);
  await root.drive(
    root.inspect().nodes.find((n) => n.props.id === "viewer.source.close")!.id,
    "click",
    {},
  );
  state.model = null;
  state.host.source = null;
  await root.render(<ViewerPanel state={state} actions={actions} />);
  await root.drive(
    root.inspect().nodes.find((n) => n.props.id === "viewer.open")!.id,
    "click",
    {},
  );
  expect(seen).toEqual(["viewer.source.close", "open"]);
  expect(
    root
      .inspect()
      .nodes.some((n) => /safe|grid|guides/.test(String(n.props.id))),
  ).toBe(false);
  expect(root.inspect().nodes.some((n) => n.text === "Viewer")).toBe(false);
  await root.dispose();
});
test("the transport runs the app's own commands, once each", async () => {
  const seen: string[] = [];
  const { root } = recorder();
  await root.render(
    <ViewerPanel
      state={fixture()}
      actions={{ open() {}, drive() {}, command: (name) => seen.push(name) }}
    />,
  );
  for (const id of [
    "gotoStart",
    "prevFrame",
    "play",
    "nextFrame",
    "gotoEnd",
    "loop",
  ]) {
    const node = root
      .inspect()
      .nodes.find((n) => n.props.id === `viewer.transport.${id}`)!;
    expect(node).toBeDefined();
    await root.drive(node.id, "click", {});
  }
  expect(seen).toEqual([
    "gotoStart",
    "prevFrame",
    "play",
    "nextFrame",
    "gotoEnd",
    "toggleLoop",
  ]);
  await root.dispose();
});
