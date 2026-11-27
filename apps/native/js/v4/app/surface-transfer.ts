import type { Ack, Commit, Transport } from "../src/protocol";

/** At most ~96 KiB after JSON escaping, below the 1 MiB host-call quota.
 * Rust stages these pieces and validates/publishes the complete tree once.
 */
export function chunkedSurfaceTransport(
  native: (operation: string, input: unknown) => any,
): Transport {
  return async (commit: Commit): Promise<Ack> => {
    const text = JSON.stringify(commit);
    if (text.length <= 16_000) return native("surface.commit", commit);
    let index = 0;
    for (let start = 0; start < text.length; ) {
      let end = Math.min(start + 16_000, text.length);
      // JSON text can contain literal Unicode: never split a surrogate pair.
      const last = text.charCodeAt(end - 1);
      if (end < text.length && last >= 0xd800 && last <= 0xdbff) end--;
      const done = end === text.length;
      const ack = native("surface.chunk", {
        index,
        text: text.slice(start, end),
        done,
      });
      if (!ack.accepted || done) return ack;
      if (!ack.staged) throw new Error("Surface chunk was not staged");
      start = end;
      index++;
    }
    throw new Error("Empty surface transfer");
  };
}
