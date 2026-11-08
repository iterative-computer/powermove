/**
 * The active v1–3 theme for the native shell (plan 3.6.5): the shipped theme
 * translator, run in the trusted realm, turns its tokens and CSS into a patch
 * of pm-design token fields. Chrome and native panels take that patch; WebKit
 * panels keep their own CSS. Both schemes are evaluated so the window can
 * follow its appearance without asking the realm again.
 */
import { evaluateTheme, translateTheme } from "../../themes/index.mjs";

/** The shell as the translator sees it: root tokens reach every surface. */
const SHELL = {
  id: "root",
  tag: "html",
  children: [{ id: "shell", tag: "body" }],
};

export interface NativeTheme {
  id: string;
  dark: Record<string, number>;
  light: Record<string, number>;
  /** The timeline surface's inputs (track, gutter, accent, text, line) as
   * 0xRRGGBB, per scheme: the first surface rules beyond tokens. */
  timeline: { dark: Record<string, number>; light: Record<string, number> };
  blockers: string[];
}

/** `#rgb`, `#rrggbb` or `rgb(r g b)` / `rgb(r, g, b)` as 0xRRGGBB. */
function rgb(value: unknown): number | undefined {
  const text = String(value ?? "").trim();
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(text)?.[1];
  if (hex)
    return parseInt(
      hex.length === 3 ? [...hex].map((c) => c + c).join("") : hex,
      16,
    );
  const parts = /^rgba?\(([^)]*)\)$/i
    .exec(text)?.[1]
    .split(/[\s,/]+/)
    .filter(Boolean)
    .slice(0, 3)
    .map(Number);
  if (parts?.length === 3 && parts.every((n) => Number.isFinite(n)))
    return (parts[0] << 16) | (parts[1] << 8) | parts[2];
  return undefined;
}

export function nativeTheme(
  definition: unknown,
  preference: string,
): NativeTheme | null {
  if (!definition || typeof definition !== "object") return null;
  const id = String((definition as { id?: unknown }).id ?? "");
  const blockers = new Set<string>();
  const timeline = { dark: {}, light: {} } as NativeTheme["timeline"];
  const patch = (systemScheme: "dark" | "light") => {
    const program = translateTheme(definition, { preference, systemScheme });
    const result = evaluateTheme(program, SHELL, {
      systemScheme,
      viewport: { width: 1440, height: 900 },
    });
    for (const blocker of result.blockers ?? [])
      blockers.add(
        typeof blocker === "string"
          ? blocker
          : String(blocker?.code ?? blocker?.reason ?? JSON.stringify(blocker)),
      );
    const inputs = (result as any).painterInputs?.timeline ?? {};
    for (const key of ["track", "gutter", "accent", "text", "line"]) {
      const value = rgb(inputs[key]);
      if (value !== undefined) timeline[systemScheme][key] = value;
    }
    return { ...(result.nativePatch ?? {}) } as Record<string, number>;
  };
  return {
    id,
    dark: patch("dark"),
    light: patch("light"),
    timeline,
    blockers: [...blockers],
  };
}
