import { describe, expect, test } from "bun:test";
import { HostClient, json } from "../../src";
import { fixture, initial } from "./fixtures";
import { ExportModel, isProgress, progress, snapshot } from "./model";
import {
  fields,
  formats,
  listedRate,
  rateOptions,
  rates,
  shows,
  sizeError,
} from "./spec";
const commands = (f: ReturnType<typeof fixture>) =>
  f.requests.flatMap((r) =>
    r.operation.type === "command" ? [r.operation.command] : [],
  );

test("format support matches tagged contract and no web export is offered", () => {
  expect(formats.map((f) => f.v)).toEqual([
    "prores",
    "mp4",
    "webm",
    "png",
    "still",
    "json",
  ]);
  expect(fields.prores).toEqual([
    "profile",
    "scale",
    "fps",
    "range",
    "colorTag",
    "timecode",
    "mblur",
    "audio",
    "audioCoding",
    "alpha",
    "useProxies",
  ]);
  // MP4 carries the color tag; a timecode track and PCM are QuickTime's.
  expect(fields.mp4).toEqual([
    "codec",
    "scale",
    "fps",
    "range",
    "quality",
    "colorTag",
    "mblur",
    "audio",
    "useProxies",
  ]);
  expect(fields.webm).toEqual([
    "scale",
    "fps",
    "range",
    "quality",
    "mblur",
    "audio",
    "useProxies",
  ]);
  expect(fields.still).toEqual(["depth", "scale", "mblur", "alpha"]);
  expect(fields.json).toEqual([]);
  expect(rates[0].v).toBe(24000 / 1001);
  expect(rates[3].v).toBe(30000 / 1001);
});
test("kind changes remember each format without mutating host defaults", async () => {
  const f = fixture();
  f.model.set("format", "prores");
  f.model.chooseKind("images");
  expect(f.model.options.format).toBe("still");
  f.model.set("format", "png");
  f.model.chooseKind("video");
  expect(f.model.options.format).toBe("prores");
  f.model.chooseKind("images");
  expect(f.model.options.format).toBe("png");
  f.model.chooseKind("project");
  expect(f.model.actionLabel).toBe("Save Project File");
  expect(f.model.state.options.format).toBe("mp4");
  expect(f.requests).toEqual([]);
});
test("custom dimensions keep aspect, reject both bounds, and preserve future fields", async () => {
  const f = fixture();
  f.model.customSize("width", 1280);
  expect(f.model.options.height).toBe(720);
  expect(f.model.canExport).toBe(false);
  await f.model.refreshPlan();
  expect(f.model.canExport).toBe(true);
  f.model.customSize("height", 1080);
  expect(f.model.options.width).toBe(1920);
  for (const n of [15, 8193, NaN, Infinity]) {
    f.model.customSize("width", n);
    expect(f.model.error).toBe(sizeError);
    await f.model.start();
    expect(commands(f)).not.toContain("export.begin");
  }
  f.model.customSize("width", 1920);
  expect(f.model.error).toBeUndefined();
  expect(f.model.options.future).toEqual({ keep: true });
});
test("unavailable format and absent audio cannot be enabled through the model", () => {
  const s = initial();
  s.project!.hasAudio = false;
  s.options.audio = false;
  s.capabilities.mp4 = {
    available: false,
    reason: "H.264 MP4 export is unavailable on this Mac",
  };
  const f = fixture(s);
  expect(f.model.canExport).toBe(false);
  f.model.set("audio", true);
  expect(f.model.options.audio).toBe(false);
  f.model.set("format", "prores");
  f.model.set("format", "mp4");
  expect(f.model.options.format).toBe("prores");
});
test("native plan acks cannot overwrite a newer draft or observation", async () => {
  const f = fixture();
  let release!: () => void;
  f.delay = () =>
    new Promise<void>((r) => {
      release = r;
    });
  f.model.set("scale", 0.5);
  const plan = f.model.refreshPlan();
  await new Promise((r) => setTimeout(r, 0));
  f.model.set("scale", 2);
  release();
  await plan;
  expect(f.model.options.scale).toBe(2);
  expect(f.model.canExport).toBe(false);
  f.delay = undefined;
  await f.model.refreshPlan();
  expect(f.model.canExport).toBe(true);
});
test("destination cancel retains every option and no local progress appears before ack", async () => {
  const f = fixture();
  f.cancelled = true;
  await f.model.show();
  f.model.customSize("width", 1280);
  await f.model.refreshPlan();
  let release!: () => void;
  f.delay = () =>
    new Promise<void>((r) => {
      release = r;
    });
  const start = f.model.start();
  await new Promise((r) => setTimeout(r, 0));
  expect(f.model.state.run.phase).toBe("choosingDestination");
  await f.model.start();
  expect(commands(f).filter((c) => c === "export.begin")).toHaveLength(1);
  release();
  await start;
  expect(f.model.state.run.phase).toBe("idle");
  expect(f.model.open).toBe(true);
  expect(f.model.options.width).toBe(1280);
  const request = f.requests.find(
    (r) =>
      r.operation.type === "command" && r.operation.command === "export.begin",
  )!;
  expect(request.operation).toMatchObject({
    args: { destination: { kind: "file", suggestedName: "Launch Film.mp4" } },
  });
});
test("PNG asks for a folder and caption options survive into the actual host request", async () => {
  const f = fixture();
  f.model.set("format", "png");
  f.model.set("captionsBurn", false);
  await f.model.refreshPlan();
  await f.model.start();
  expect(f.requests.at(-1)?.operation).toMatchObject({
    type: "command",
    command: "export.begin",
    args: {
      destination: { kind: "folder", suggestedName: "Launch Film.png" },
      options: { captionsBurn: false, future: { keep: true } },
    },
  });
});
test("preset save is guarded, acknowledged, app-wide and never includes destination or output name", async () => {
  const f = fixture();
  await f.model.savePreset();
  expect(f.requests).toHaveLength(0);
  f.model.namePreset("  Delivery  ");
  await f.model.savePreset();
  expect(f.model.state.presets).toHaveLength(1);
  expect(f.model.chosenPreset).toBe("fixture-preset-1");
  const op = f.requests.at(-1)!.operation;
  expect(op).toMatchObject({
    type: "command",
    command: "export.preset.save",
    args: { name: "Delivery" },
  });
  if (op.type === "command")
    expect(Object.keys(op.args as object)).toEqual(["name", "options"]);
  f.model.set("format", "png");
  await f.model.choosePreset("fixture-preset-1");
  expect(f.model.options.format).toBe("mp4");
});
test("both export shortcuts, Esc settings-only, Return preset-only and command-period progress cancel", async () => {
  const f = fixture();
  expect(await f.model.key("e", true)).toBe(true);
  expect(f.model.open).toBe(true);
  await f.model.key("Escape");
  expect(f.model.open).toBe(false);
  await f.model.key("e", true, true);
  expect(f.model.open).toBe(true);
  f.model.namePreset("Delivery");
  await f.model.key("Enter", false, false, true);
  // Escape ended the sheet on the host; Return then saved only the preset.
  expect(commands(f)).toEqual(["export.dismiss", "export.preset.save"]);
  await f.model.key("Enter");
  expect(f.model.state.run.phase).toBe("preparing");
  expect(await f.model.key("Escape")).toBe(false);
  expect(f.model.open).toBe(true);
  await f.model.key(".", true);
  expect(f.model.open).toBe(false);
});
test("subscriptions accept real host publications and reject stale/retired generations", async () => {
  const f = fixture();
  await f.model.connect("export.fixture");
  await f.model.show();
  const s = initial();
  s.run = { phase: "rendering", frame: 120, total: 300, elapsed: 2, eta: 120 };
  expect(await f.publish(s)).toBe(true);
  expect(f.model.state.run.phase).toBe("rendering");
  await f.model.show();
  expect(commands(f)).toContain("export.reveal");
  expect(
    f.host.publish({
      ...f.host.identity,
      version: 1,
      subscription: "export.fixture",
      revision: 0,
      sequence: 1,
      value: json(initial()),
    }),
  ).toBe(false);
  await f.model.dispose();
  expect(await f.publish(initial())).toBe(false);
});
test("completion closes and permits another export only after native reset", async () => {
  const f = fixture();
  await f.model.connect("export.fixture");
  await f.model.show();
  const s = initial();
  s.run = { phase: "done", path: "fixture.mp4" };
  await f.publish(s);
  expect(f.model.open).toBe(false);
  await f.model.show();
  expect(f.model.canExport).toBe(true);
  expect(commands(f)).toContain("export.reset");
});
test("failure retries and closes through native host; transport rejection never reports success", async () => {
  const s = initial();
  s.run = { phase: "failed", message: "Reconnect the source and try again." };
  const f = fixture(s);
  f.model.open = true;
  await f.model.key("Enter");
  expect(f.model.state.run.phase).toBe("preparing");
  const g = fixture(s);
  await g.model.closeFailure();
  expect(g.model.open).toBe(false);
  const h = fixture();
  h.rejection = "fixture rejection";
  await expect(h.model.start()).rejects.toThrow("fixture rejection");
  expect(h.model.fault).toBeDefined();
  expect(h.model.canExport).toBe(false);
  expect(h.model.state.run.phase).toBe("idle");
});
test("progress uses actual frames and measured ETA thresholds", () => {
  expect(progress({ phase: "preparing" })).toEqual({ leading: "Preparing…" });
  expect(
    progress({ phase: "rendering", frame: 7, total: 300, elapsed: 4, eta: 120 })
      .eta,
  ).toBeUndefined();
  expect(
    progress({
      phase: "rendering",
      frame: 8,
      total: 300,
      elapsed: 0.9,
      eta: 120,
    }).eta,
  ).toBeUndefined();
  expect(
    progress({
      phase: "rendering",
      frame: 120,
      total: 300,
      elapsed: 2,
      eta: 120,
      extra: "fixture",
    }),
  ).toEqual({
    leading: "Frame 120 of 300 · 40 % · fixture",
    value: 0.4,
    ax: "40 percent, frame 120 of 300",
    eta: "About 2 min left",
  });
  expect(
    progress({ phase: "rendering", frame: 8, total: 300, elapsed: 1, eta: 89 })
      .eta,
  ).toBe("About 89 s left");
});
test("checking the written file is its own phase, with its own progress", () => {
  expect(
    progress({ phase: "checking", frame: 75, total: 300, elapsed: 30 }),
  ).toEqual({
    leading: "Checking the file · frame 75 of 300",
    value: 0.25,
    ax: "Checking the file, 25 percent, frame 75 of 300",
  });
  expect(
    isProgress({ phase: "checking", frame: 0, total: 300, elapsed: 1 }),
  ).toBe(true);
});
test("malformed model facts fail visibly, without normalizing or mutating their source", () => {
  for (const mutate of [
    (s: any) =>
      (s.run = { phase: "rendering", frame: 301, total: 300, elapsed: 1 }),
    (s: any) =>
      (s.run = { phase: "checking", frame: 301, total: 300, elapsed: 1 }),
    (s: any) => (s.options.format = "web"),
    (s: any) => (s.bindings = {}),
    (s: any) => (s.sequence = -1),
  ]) {
    const s = initial();
    mutate(s);
    expect(() => snapshot(s)).toThrow();
  }
  const s = initial();
  const f = new ExportModel(fixture().host, s);
  f.options.future = false;
  expect(s.options.future).toEqual({ keep: true });
});

test("a late begin acknowledgement cannot regress newer native progress", async () => {
  const idle = initial();
  let release!: () => void;
  const host = new HostClient(
    { realm: "builtin.export", project: "fixture", generation: 1 },
    async (request) => {
      let value = idle;
      if (
        request.operation.type === "command" &&
        request.operation.command === "export.begin"
      ) {
        await new Promise<void>((r) => {
          release = r;
        });
        value = { ...initial(), sequence: 1, run: { phase: "preparing" } };
      }
      return { ...request, revision: 0, accepted: true, value: json(value) };
    },
  );
  const model = new ExportModel(host, idle);
  await model.connect("export.fixture");
  const start = model.start();
  await new Promise((r) => setTimeout(r, 0));
  const newer = {
    ...initial(),
    sequence: 2,
    run: { phase: "rendering", frame: 8, total: 300, elapsed: 1 },
  };
  expect(
    host.publish({
      ...host.identity,
      version: 1,
      revision: 0,
      sequence: 1,
      subscription: "export.fixture",
      value: json(newer),
    }),
  ).toBe(true);
  release();
  await start;
  expect(model.state.run.phase).toBe("rendering");
  await model.dispose();
});

test("native binding publications stay authoritative after an acknowledged local edit", async () => {
  const f = fixture();
  await f.model.connect("export.fixture");
  f.model.set("format", "prores");
  await f.model.refreshPlan();
  const next = initial();
  next.options.format = "png";
  await f.publish(next);
  expect(f.model.options.format).toBe("png");
  f.model.chooseKind("video");
  f.model.chooseKind("images");
  expect(f.model.options.format).toBe("png");
  await f.model.dispose();
});

test("unavailable remembered formats remain inspectable and invalid size drafts cannot block project delivery", async () => {
  const s = initial();
  s.capabilities.mp4 = {
    available: false,
    reason: "H.264 MP4 export is unavailable on this Mac",
  };
  const f = fixture(s);
  f.model.chooseKind("images");
  f.model.chooseKind("video");
  expect(f.model.kind).toBe("video");
  expect(f.model.options.format).toBe("mp4");
  expect(f.model.canExport).toBe(false);
  f.model.chooseKind("images");
  f.model.customSize("width", -1);
  expect(f.model.error).toBe(sizeError);
  expect(f.model.size.width).toBe(-1);
  expect(f.model.options.width).toBe(1920);
  f.model.chooseKind("project");
  await f.model.refreshPlan();
  expect(f.model.canExport).toBe(true);
  await f.model.start();
  expect(f.requests.at(-1)?.operation).toMatchObject({
    command: "export.begin",
    args: { options: { format: "json", width: 1920 } },
  });
});

test("malformed accepted command projections fence further export starts", async () => {
  const host = new HostClient(
    { realm: "builtin.export", project: "fixture", generation: 1 },
    async (request) => ({
      ...request,
      revision: 0,
      accepted: true,
      value: null,
    }),
  );
  const model = new ExportModel(host, initial());
  await expect(model.start()).rejects.toThrow("Invalid export projection");
  expect(model.fault).toBeDefined();
  expect(model.canExport).toBe(false);
});

test("planning and preset failures retain authored drafts and recover on actual acknowledgement", async () => {
  const f = fixture();
  f.model.set("quality", "draft");
  f.rejection = "planner unavailable";
  await expect(f.model.refreshPlan()).rejects.toThrow("planner unavailable");
  expect(f.model.planningError).toContain("planner unavailable");
  expect(f.model.canExport).toBe(false);
  expect(f.model.options.quality).toBe("draft");
  f.rejection = undefined;
  await f.model.refreshPlan();
  expect(f.model.planningError).toBeUndefined();
  expect(f.model.canExport).toBe(true);
  f.model.namePreset("Authored name");
  f.rejection = "store unavailable";
  await expect(f.model.savePreset()).rejects.toThrow("store unavailable");
  expect(f.model.presetError).toBe(true);
  expect(f.model.presetName).toBe("Authored name");
  f.rejection = undefined;
  await f.model.savePreset();
  expect(f.model.presetError).toBe(false);
});

test("ProRes 422 has no alpha; settings saved before profiles keep ProRes 4444", async () => {
  const seed = initial();
  delete (seed.options as Record<string, unknown>).profile;
  delete (seed.options as Record<string, unknown>).codec;
  delete (seed.options as Record<string, unknown>).depth;
  const f = fixture(seed);
  expect(f.model.options.profile).toBe("4444");
  expect(f.model.options.codec).toBe("h264");
  expect(f.model.options.depth).toBe(8);
  f.model.set("format", "prores");
  expect(shows(f.model.options, "alpha")).toBe(true);
  f.model.set("profile", "422hq");
  expect(shows(f.model.options, "alpha")).toBe(false);
  f.model.set("format", "mp4");
  f.model.set("codec", "hevc");
  expect(() => f.model.set("depth", 10 as 8)).toThrow();
  await f.model.refreshPlan();
  await f.model.start();
  const begin = f.requests.find(
    (r) =>
      r.operation.type === "command" && r.operation.command === "export.begin",
  )!;
  const options = (
    begin.operation as { args: { options: Record<string, unknown> } }
  ).args.options;
  expect(options.codec).toBe("hevc");
  expect(options.profile).toBe("422hq");
});

test("color tag, start timecode and audio format are options of the export, and old options keep today's file", async () => {
  const f = fixture();
  // Options saved before the fields existed read as today's output.
  expect(f.model.options.colorTag).toBe("rec709QuickTime");
  expect(f.model.options.timecode).toBe("");
  expect(f.model.options.audioCoding).toBe("pcm24");
  f.model.set("format", "prores");
  f.model.set("profile", "422hq");
  f.model.set("colorTag", "rec709");
  f.model.set("timecode", "01:00:00:00");
  f.model.set("audioCoding", "pcm16");
  expect(() => f.model.set("colorTag", "p3" as "rec709")).toThrow();
  expect(() => f.model.set("audioCoding", "aac" as "pcm16")).toThrow();
  for (const field of ["colorTag", "timecode", "audioCoding"] as const)
    expect(shows(f.model.options, field)).toBe(true);
  // The planner (the host) checks the timecode against the frame rate.
  await f.model.refreshPlan();
  await f.model.start();
  const sent = (command: string) =>
    (
      f.requests.find(
        (r) =>
          r.operation.type === "command" && r.operation.command === command,
      )!.operation as unknown as { args: { options: Record<string, unknown> } }
    ).args.options;
  for (const command of ["export.plan", "export.begin"]) {
    expect(sent(command).colorTag).toBe("rec709");
    expect(sent(command).timecode).toBe("01:00:00:00");
    expect(sent(command).audioCoding).toBe("pcm16");
  }
  // MP4 keeps the label and has no timecode track or PCM; WebM has none.
  expect(shows({ format: "mp4" }, "colorTag")).toBe(true);
  expect(shows({ format: "mp4" }, "timecode")).toBe(false);
  expect(shows({ format: "mp4" }, "audioCoding")).toBe(false);
  expect(shows({ format: "webm" }, "colorTag")).toBe(false);
});

test("the Frame Rate row shows the rate the export uses", () => {
  // A project stores 29.97 as that float; the engine exports it as
  // 30000/1001, and the pop-up must not fall back to its first item.
  expect(listedRate(29.97)).toBe(30000 / 1001);
  expect(listedRate(23.976)).toBe(24000 / 1001);
  expect(listedRate(59.94)).toBe(60000 / 1001);
  expect(listedRate(30)).toBe(30);
  expect(rateOptions(29.97)).toEqual(rates);
  // A rate that is none of the listed ones is offered as itself.
  expect(listedRate(12)).toBe(12);
  expect(rateOptions(12)[0]).toEqual({ v: 12, label: "12 fps" });
  expect(rateOptions(12).slice(1)).toEqual(rates);
});
