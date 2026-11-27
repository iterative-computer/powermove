import { expect, test } from "bun:test";
import {
  RecorderCanvas,
  type RecorderHost,
  type SurfaceVersion,
} from "../src/canvas";
class RecorderDouble implements RecorderHost {
  surfaces = new Map<
    string,
    {
      width: number;
      height: number;
      version: number;
      state: Record<string, any>;
      commands: any[];
    }
  >();
  pins = new Map<string, any[]>();
  create(width: number, height: number): string {
    const id = `surface-${this.surfaces.size}`;
    this.surfaces.set(id, {
      width,
      height,
      version: 0,
      state: {},
      commands: [],
    });
    return id;
  }
  reset(id: string, width: number, height: number): void {
    const surface = this.surfaces.get(id)!;
    Object.assign(surface, {
      width,
      height,
      state: {},
      commands: [],
      version: surface.version + 1,
    });
  }
  command(id: string, name: string, args: any[]): void {
    const s = this.surfaces.get(id)!;
    s.commands.push([name, args]);
    if (name === "set") s.state[args[0]] = args[1];
    s.version++;
  }
  retainVersion(id: string): SurfaceVersion {
    const s = this.surfaces.get(id)!,
      ref = { surface: id, version: s.version };
    this.pins.set(JSON.stringify(ref), structuredClone(s.commands));
    return ref;
  }
  releaseVersion(version: SurfaceVersion): void {
    this.pins.delete(JSON.stringify(version));
  }
  read(id: string, name: string, args: any[]): any {
    if (name === "get") return this.surfaces.get(id)!.state[args[0]];
    if (name.startsWith("create") && name !== "createImageData")
      return "paint-handle";
    if (name === "toDataURL") return "data:" + args[0] + ";base64,synthetic";
    return { name, args, surface: id };
  }
  release(id: string): void {
    this.surfaces.delete(id);
  }
}
test("canvas/context identity and native state reads survive resets", () => {
  const host = new RecorderDouble(),
    canvas = new RecorderCanvas(host),
    ctx = canvas.getContext("2d");
  expect(ctx).toBe(canvas.getContext("2d"));
  expect(ctx.canvas).toBe(canvas);
  expect(ctx.fillRect).toBe(ctx.fillRect);
  ctx.fillStyle = "#ff0000";
  expect(ctx.fillStyle).toBe("#ff0000");
  ctx.fillRect(0, 0, 2, 2);
  canvas.width = 25;
  expect(ctx).toBe(canvas.getContext("2d"));
  expect(ctx.fillStyle).toBeUndefined();
  expect(canvas.width).toBe(25);
  expect(canvas.getContext("webgl")).toBeNull();
});
test("drawImage pins the source version across repaint, reset and source disposal", () => {
  const host = new RecorderDouble(),
    source = new RecorderCanvas(host),
    target = new RecorderCanvas(host);
  source.getContext("2d").fillRect(0, 0, 4, 4);
  target.getContext("2d").drawImage(source, 0, 0);
  const pinned = structuredClone([...host.pins.values()][0]);
  source.getContext("2d").clearRect(0, 0, 4, 4);
  source.width = 100;
  source.dispose();
  expect([...host.pins.values()][0]).toEqual(pinned);
  expect(host.pins.size).toBe(1);
  target.dispose();
  expect(host.pins.size).toBe(0);
});
test("paint handles, clipping/text/pixel commands and encode use native host", () => {
  const host = new RecorderDouble(),
    canvas = new RecorderCanvas(host, 4, 4),
    ctx = canvas.getContext("2d");
  const paint = ctx.createLinearGradient(0, 0, 4, 4);
  paint.addColorStop(0, "#ff0000");
  ctx.fillStyle = paint;
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, 4, 4);
  ctx.clip();
  ctx.fillText("Synthetic", 0, 0);
  ctx.restore();
  expect(ctx.measureText("Synthetic").name).toBe("measureText");
  expect(ctx.getImageData(0, 0, 4, 4).name).toBe("getImageData");
  expect(canvas.toDataURL("image/jpeg", 0.9)).toStartWith("data:image/jpeg;");
  expect(
    host.surfaces.get(canvas.surface)!.commands.map((x) => x[0]),
  ).toContain("paint.addColorStop");
});
test("lifetime and invalid state errors remain explicit", () => {
  const host = new RecorderDouble(),
    canvas = new RecorderCanvas(host),
    ctx = canvas.getContext("2d");
  expect(() => new RecorderCanvas(host, -1, 1)).toThrow();
  ctx.unknown = 1;
  expect(ctx.unknown).toBe(1);
  canvas.dispose();
  canvas.dispose();
  expect(() => ctx.fillRect(0, 0, 1, 1)).toThrow("disposed");
});
test("native surface adoption preserves render/raster canvas identity without reallocating", async () => {
  const { CompatRealm } = await import("../src");
  const { FakeHost } = await import("./fake-host");
  const host = new FakeHost(),
    recorder = new RecorderDouble();
  const surface = recorder.create(16, 8);
  host.query = (_method, args) => {
    expect(args.realm).toBe("surface");
    expect(args.generation).toBe(1);
    expect(args.revision).toBe(host.state.revision);
    return { kind: "recorder-surface", surface, width: 16, height: 8 };
  };
  const realm = new CompatRealm({
    id: "surface",
    generation: 1,
    host,
    initial: host.publication(),
    recorder,
  });
  const api = realm.extension("consumer").api;
  const canvas = api.render.renderFrameTo();
  expect(canvas).toBeInstanceOf(RecorderCanvas);
  // A layer's base raster on the same surface is the same canvas.
  expect(canvas).toBe(api.render.raster({ id: "layer" }).cv);
  expect(canvas).toBe(realm.PM.renderFrameTo());
  expect(canvas.width).toBe(16);
  expect(recorder.surfaces.size).toBe(1);
  const context = canvas.getContext("2d");
  expect(context).toBe(canvas.getContext("2d"));
  realm.dispose();
  expect(recorder.surfaces.size).toBe(0);
});
test("patterns pin canvas versions and paints can be shared across same-host contexts", () => {
  const host = new RecorderDouble(),
    source = new RecorderCanvas(host),
    target = new RecorderCanvas(host);
  const gradient = source.getContext("2d").createLinearGradient(0, 0, 1, 1);
  target.getContext("2d").fillStyle = gradient;
  expect(host.surfaces.get(target.surface)!.state.fillStyle).toEqual({
    paint: "paint-handle",
    surface: source.surface,
  });
  const pattern = target.getContext("2d").createPattern(source, "repeat");
  expect(pattern).not.toBeNull();
  expect(host.pins.size).toBe(1);
  source.dispose();
  expect(host.pins.size).toBe(1);
  target.dispose();
  expect(host.pins.size).toBe(0);
});
