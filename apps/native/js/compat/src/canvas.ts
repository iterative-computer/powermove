import type { Data } from "./types";

export interface NativeSurface {
  kind: "recorder-surface";
  surface: string;
  width: number;
  height: number;
}
export interface SurfaceVersion {
  surface: string;
  version: number;
}
/** Recorder API is synchronous on the realm worker, backed by native raster/text. */
export interface RecorderHost {
  create(width: number, height: number): string;
  reset(surface: string, width: number, height: number): void;
  command(surface: string, name: string, args: any[]): void;
  /** Pin the current source pixels/commands, not a mutable canvas reference. */
  retainVersion(surface: string, scale?: number): SurfaceVersion;
  releaseVersion(version: SurfaceVersion): void;
  read(surface: string, name: string, args: any[]): any;
  release(surface: string): void;
}
const writes = new Set([
  "save",
  "restore",
  "reset",
  "scale",
  "rotate",
  "translate",
  "transform",
  "setTransform",
  "resetTransform",
  "clearRect",
  "fillRect",
  "strokeRect",
  "beginPath",
  "closePath",
  "moveTo",
  "lineTo",
  "bezierCurveTo",
  "quadraticCurveTo",
  "arc",
  "arcTo",
  "ellipse",
  "rect",
  "roundRect",
  "fill",
  "stroke",
  "clip",
  "fillText",
  "strokeText",
  "setLineDash",
  "putImageData",
]);
const reads = new Set([
  "getImageData",
  "createImageData",
  "measureText",
  "isPointInPath",
  "isPointInStroke",
  "getTransform",
  "getLineDash",
]);
const properties = new Set([
  "direction",
  "fillStyle",
  "filter",
  "font",
  "fontKerning",
  "fontStretch",
  "fontVariantCaps",
  "globalAlpha",
  "globalCompositeOperation",
  "imageSmoothingEnabled",
  "imageSmoothingQuality",
  "letterSpacing",
  "lineCap",
  "lineDashOffset",
  "lineJoin",
  "lineWidth",
  "miterLimit",
  "shadowBlur",
  "shadowColor",
  "shadowOffsetX",
  "shadowOffsetY",
  "strokeStyle",
  "textAlign",
  "textBaseline",
  "textRendering",
  "wordSpacing",
]);
class Paint {
  constructor(
    readonly canvas: RecorderCanvas,
    readonly handle: string,
  ) {}
  addColorStop(offset: number, color: string): void {
    this.canvas.paint(this.handle, "addColorStop", [offset, color]);
  }
  setTransform(matrix?: Data): void {
    this.canvas.paint(this.handle, "setTransform", [matrix]);
  }
}
/** A decoded image the host holds (`<img>`, `ImageBitmap`): drawn like a canvas
 * source. `scale` is how much larger than intrinsic a vector image is drawn. */
export class NativeImage {
  constructor(
    readonly host: RecorderHost,
    readonly surface: string,
    readonly width: number,
    readonly height: number,
    readonly vector: boolean,
  ) {}
  retain(scale = 1): SurfaceVersion {
    return this.host.retainVersion(this.surface, this.vector ? scale : 1);
  }
}
/** The scale a drawImage call draws its source at, from its geometry. */
function drawScale(source: NativeImage, geometry: any[]): number {
  const [dw, dh] =
    geometry.length === 8
      ? [geometry[6] / (geometry[2] || 1), geometry[7] / (geometry[3] || 1)]
      : geometry.length === 4
        ? [geometry[2] / (source.width || 1), geometry[3] / (source.height || 1)]
        : [1, 1];
  return Math.max(Math.abs(dw), Math.abs(dh), 0) || 1;
}
export class RecorderCanvas {
  readonly surface: string;
  private context?: Data;
  private disposed = false;
  private pins: SurfaceVersion[] = [];
  private w: number;
  private h: number;
  constructor(
    private host: RecorderHost,
    width = 300,
    height = 150,
    surface?: string,
  ) {
    this.w = this.dimension(width);
    this.h = this.dimension(height);
    this.surface = surface ?? host.create(this.w, this.h);
  }
  private dimension(value: number): number {
    if (!Number.isInteger(value) || value < 0 || value > 0xffffffff)
      throw new RangeError("Invalid canvas dimension");
    return value;
  }
  private live(): void {
    if (this.disposed) throw new Error("Canvas is disposed");
  }
  get width(): number {
    return this.w;
  }
  set width(value: number) {
    this.resize(this.dimension(value), this.h);
  }
  get height(): number {
    return this.h;
  }
  set height(value: number) {
    this.resize(this.w, this.dimension(value));
  }
  private resize(width: number, height: number): void {
    this.live();
    this.host.reset(this.surface, width, height);
    this.w = width;
    this.h = height;
    this.releasePins(); // Reset also resets context state in native host, retaining JS context identity.
  }
  private encode(value: any): any {
    if (value instanceof Paint) {
      if (value.canvas.host !== this.host)
        throw new TypeError("Paint belongs to another recorder host");
      return { paint: value.handle, surface: value.canvas.surface };
    }
    if (value instanceof RecorderCanvas || value instanceof NativeImage)
      throw new TypeError("Canvas source requires a pinned drawImage version");
    return value;
  }
  paint(handle: string, name: string, args: any[]): void {
    this.live();
    this.host.command(this.surface, "paint." + name, [handle, ...args]);
  }
  getContext(kind: string, options?: Data): any {
    this.live();
    if (kind !== "2d") return null;
    if (this.context) return this.context;
    this.host.command(this.surface, "context", [options ?? {}]);
    const methods = new Map<string, Function>();
    const method = (name: string): Function => {
      if (!methods.has(name))
        methods.set(name, (...args: any[]) => {
          this.live();
          if (name === "drawImage") {
            const [source, ...geometry] = args;
            if (
              !(source instanceof RecorderCanvas) &&
              !(source instanceof NativeImage)
            )
              throw new TypeError(
                "drawImage: the source is not an image, ImageBitmap or canvas",
              );
            if (source instanceof RecorderCanvas) source.live();
            if (source.host !== this.host)
              throw new TypeError(
                "Canvas source belongs to another recorder host",
              );
            // Retina scale (2) so vector images stay sharp on the page.
            const version =
              source instanceof NativeImage
                ? source.retain(drawScale(source, geometry) * 2)
                : source.host.retainVersion(source.surface);
            try {
              this.host.command(this.surface, "drawImage", [
                version,
                ...geometry,
              ]);
              this.pins.push(version);
            } catch (error) {
              source.host.releaseVersion(version);
              throw error;
            }
            return;
          }
          if (
            name === "createPattern" &&
            (args[0] instanceof RecorderCanvas || args[0] instanceof NativeImage)
          ) {
            const source = args[0] as RecorderCanvas | NativeImage;
            if (source instanceof RecorderCanvas) source.live();
            if (source.host !== this.host)
              throw new TypeError(
                "Canvas source belongs to another recorder host",
              );
            const version =
              source instanceof NativeImage
                ? source.retain(1)
                : this.host.retainVersion(source.surface);
            try {
              const handle = this.host.read(this.surface, name, [
                version,
                ...args.slice(1),
              ]);
              if (handle == null) {
                this.host.releaseVersion(version);
                return null;
              }
              this.pins.push(version);
              return new Paint(this, handle);
            } catch (error) {
              this.host.releaseVersion(version);
              throw error;
            }
          }
          if (name.startsWith("create") && !reads.has(name)) {
            const handle = this.host.read(this.surface, name, args);
            return handle == null ? null : new Paint(this, handle);
          }
          if (reads.has(name)) return this.host.read(this.surface, name, args);
          this.host.command(
            this.surface,
            name,
            args.map((v) => this.encode(v)),
          );
        });
      return methods.get(name)!;
    };
    this.context = new Proxy(
      {},
      {
        get: (target, key) => {
          if (key === "canvas") return this;
          if (typeof key !== "string") return Reflect.get(target, key);
          if (
            writes.has(key) ||
            reads.has(key) ||
            key === "drawImage" ||
            [
              "createLinearGradient",
              "createRadialGradient",
              "createConicGradient",
              "createPattern",
            ].includes(key)
          )
            return method(key);
          if (properties.has(key)) {
            this.live();
            return this.host.read(this.surface, "get", [key]);
          }
          return Reflect.get(target, key);
        },
        set: (target, key, value) => {
          this.live();
          if (typeof key !== "string" || !properties.has(key))
            return Reflect.set(target, key, value);
          this.host.command(this.surface, "set", [key, this.encode(value)]);
          return true;
        },
      },
    );
    return this.context;
  }
  toDataURL(type = "image/png", quality?: number): string {
    this.live();
    return this.host.read(this.surface, "toDataURL", [type, quality]);
  }
  private releasePins(): void {
    for (const version of this.pins) this.host.releaseVersion(version);
    this.pins = [];
  }
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.releasePins();
    this.host.release(this.surface);
  }
}
