/** The native compiler's module graph as the link request V8 takes: module
 * ids become URLs under `base`, and `runtime:` targets become ES modules over
 * the realm's `__powermove_runtime` table. Trusted realms and the Store helper
 * link through this one function. */
import type { Data } from "./types";

const RESERVED = new Set(
  "await break case catch class const continue debugger default delete do else enum export extends false finally for function if implements import in instanceof interface let new null package private protected public return static super switch this throw true try typeof var void while with yield".split(
    " ",
  ),
);
const IDENTIFIER = /^[A-Za-z_$][\w$]*$/;

/** Runtime modules linked as fixed source instead of a table entry. Compiled
 * Svelte components import disclose-version only to stamp a version global. */
export const ADAPTED_MODULES: Record<string, string> = {
  "svelte/internal/disclose-version": "export {}",
};

function runtimeSource(table: Data, specifier: string): string {
  if (specifier in ADAPTED_MODULES) return ADAPTED_MODULES[specifier]!;
  if (!(specifier in table))
    throw new Error(`Unsupported runtime import: ${specifier}`);
  const keys = Object.keys(table[specifier]).filter(
    (k) => k !== "default" && IDENTIFIER.test(k),
  );
  return [
    `const m = globalThis.__powermove_runtime[${JSON.stringify(specifier)}];`,
    "export default m;",
    ...keys
      .filter((k) => !RESERVED.has(k))
      .map((k) => `export const ${k} = m.${k};`),
    ...keys
      .filter((k) => RESERVED.has(k))
      .map(
        (k, i) =>
          `const __reserved_${i} = m.${k}; export { __reserved_${i} as ${k} };`,
      ),
  ].join("\n");
}

export function linkRequest(graph: Data, base: string, table: Data): string {
  const url = (id: string) =>
    id.startsWith("runtime:") ? `pmruntime://${id.slice(8)}` : base + id;
  const sources: Data = {},
    edges: Data = {};
  for (const [id, source] of Object.entries(graph.sources))
    sources[url(id)] = source;
  for (const [id, targets] of Object.entries(graph.edges as Data)) {
    edges[url(id)] = {};
    for (const [specifier, target] of Object.entries(targets as Data)) {
      edges[url(id)][specifier] = url(target as string);
      if ((target as string).startsWith("runtime:"))
        sources[url(target as string)] ??= runtimeSource(
          table,
          (target as string).slice(8),
        );
    }
  }
  return JSON.stringify({ sources, edges, entry: url("powermove:bundle") });
}
