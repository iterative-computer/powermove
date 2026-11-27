/** The shipped theme translator (`apps/native/js/themes`), plain ES modules. */
declare module "*/themes/index.mjs" {
  export function translateTheme(
    definition: unknown,
    options?: unknown,
  ): unknown;
  export function evaluateTheme(
    program: unknown,
    tree: unknown,
    environment?: unknown,
  ): { nativePatch?: Record<string, number>; blockers?: any[] };
}
