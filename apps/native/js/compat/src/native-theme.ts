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
  blockers: string[];
}

export function nativeTheme(
  definition: unknown,
  preference: string,
): NativeTheme | null {
  if (!definition || typeof definition !== "object") return null;
  const id = String((definition as { id?: unknown }).id ?? "");
  const blockers = new Set<string>();
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
    return { ...(result.nativePatch ?? {}) } as Record<string, number>;
  };
  return {
    id,
    dark: patch("dark"),
    light: patch("light"),
    blockers: [...blockers],
  };
}
