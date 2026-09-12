/** Absolute paths of the player entry modules inside apps/desktop. */
export const entries: { readonly player: string; readonly 'svg-player': string };

/** Bundle one browser-only entry to minified ESM text, plus the absolute paths of every input file. */
export function bundlePlayerEntry(entryPath: string): Promise<{ text: string; inputs: string[] }>;
