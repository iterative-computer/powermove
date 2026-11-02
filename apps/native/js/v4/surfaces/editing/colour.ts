/** S035 worker state; uses the existing native colour math, no browser APIs. */
import {
  type Color,
  type Notation as BaseNotation,
  formatColor,
  parseColor,
  toOklab,
  toOklch,
  toP3,
  toHsv,
  fromHsv,
  fromP3,
  fromOklab,
  clipped,
  fromOklchKeepingChroma,
  fromOklchKeepingLightness,
  outsideSrgb,
} from "../../src/color-space";

export type Notation = "Hex" | "RGB" | "HSL" | "OKLCH" | "OKLab" | "P3";
export type Space = "sRGB" | "Display P3";
export type HueMapping =
  | "Find Closest Color"
  | "Maintain Chroma"
  | "Maintain Lightness";
export const notations: Notation[] = [
  "Hex",
  "RGB",
  "HSL",
  "OKLCH",
  "OKLab",
  "P3",
];
export const hueMappings: HueMapping[] = [
  "Find Closest Color",
  "Maintain Chroma",
  "Maintain Lightness",
];
const base: Record<Exclude<Notation, "OKLab">, BaseNotation> = {
  Hex: "hex",
  RGB: "rgb",
  HSL: "hsl",
  OKLCH: "oklch",
  P3: "display-p3",
};
export function notationString(colour: Color, notation: Notation): string {
  if (notation === "OKLab") {
    const lab = toOklab(colour);
    return `${(lab.l * 100).toFixed(1)}% ${lab.a.toFixed(3)} ${lab.b.toFixed(3)}`;
  }
  return formatColor(colour, base[notation]);
}
/** The spec copies the notation field verbatim; opacity is its separate well. */
export function copyString(colour: Color, notation: Notation): string {
  return notationString(colour, notation);
}
export function parseNotation(value: string, notation: Notation): Color | null {
  value = value.replaceAll("°", "");
  if (notation === "OKLab") {
    const m = value
      .trim()
      .match(
        /^([+-]?(?:\d+\.?\d*|\.\d+))(%?)\s+([+-]?(?:\d+\.?\d*|\.\d+))\s+([+-]?(?:\d+\.?\d*|\.\d+))(?:\s*\/\s*([+-]?(?:\d+\.?\d*|\.\d+))(%?))?$/,
      );
    if (!m) return null;
    const alpha = m[5] === undefined ? 1 : Number(m[5]) / (m[6] ? 100 : 1);
    if (alpha < 0 || alpha > 1) return null;
    return fromOklab(
      Number(m[1]) / (m[2] ? 100 : 1),
      Number(m[3]),
      Number(m[4]),
      alpha,
    );
  }
  if (notation === "Hex") return parseColor(value);
  if (notation === "P3") return parseColor(`color(display-p3 ${value})`);
  return parseColor(`${notation.toLowerCase()}(${value})`);
}
export class ColourPicker {
  readonly previous: Color;
  colour: Color;
  private currentSpace: Space = "sRGB";
  get space(): Space {
    return this.currentSpace;
  }
  set space(value: Space) {
    this.currentSpace = value;
    this.hue = this.plane().h;
  }
  notation: Notation = "Hex";
  mapping: HueMapping = "Find Closest Color";
  private hue: number;
  constructor(colour: Color) {
    this.previous = { ...colour };
    this.colour = { ...colour };
    this.hue = this.plane().h;
  }
  restore() {
    this.colour = { ...this.previous };
    this.hue = this.plane().h;
  }
  draft(value: string): boolean {
    const colour = parseNotation(value, this.notation);
    if (!colour || !Object.values(colour).every(Number.isFinite)) return false;
    this.colour = colour;
    this.hue = this.plane().h;
    return true;
  }
  warning(project: Space): string | undefined {
    return project === "sRGB" && outsideSrgb(this.colour)
      ? "Outside sRGB; saved as the nearest sRGB colour"
      : undefined;
  }
  changeHue(degrees: number): boolean {
    if (!Number.isFinite(degrees)) return false;
    const next = ((degrees % 360) + 360) % 360;
    if (next === this.hue) return true;
    const old = toOklch(this.colour);
    const hsv = this.plane();
    if (this.mapping === "Find Closest Color" || hsv.s === 0) {
      const plane = fromHsv(next, hsv.s, hsv.v, this.colour.a);
      this.colour = this.space === "Display P3" ? fromP3(plane) : plane;
    } else {
      const shifted = (old.h + next - this.hue + 360) % 360;
      this.colour =
        this.mapping === "Maintain Chroma"
          ? fromOklchKeepingChroma(old.l, old.c, shifted, this.colour.a)
          : fromOklchKeepingLightness(old.l, old.c, shifted, this.colour.a);
    }
    this.hue = next;
    return true;
  }
  private plane() {
    return toHsv(
      this.space === "Display P3" ? toP3(this.colour) : clipped(this.colour),
    );
  }
}
