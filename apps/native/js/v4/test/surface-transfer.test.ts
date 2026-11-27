import { expect, test } from "bun:test";
import { chunkedSurfaceTransport } from "../app/surface-transfer";
import type { Commit } from "../src/protocol";

for (const count of [40, 436, 1536, 1600]) {
  test(`timeline ${count} descriptors stay below host argument quota`, async () => {
    const commit: Commit = {
      baseRevision: 7,
      ops: Array.from({ length: count }, (_, id) => ({
        op: "setText",
        id,
        text: `layer ${id} ${'😀\\"'.repeat(400)}`,
      })),
    };
    const pieces: string[] = [];
    let publications = 0;
    const transport = chunkedSurfaceTransport((operation, raw) => {
      const input = raw as { index: number; text: string; done: boolean };
      expect(operation).toBe("surface.chunk");
      expect(
        new TextEncoder().encode(JSON.stringify(input)).length,
      ).toBeLessThan(100_000);
      expect(input.index).toBe(pieces.length);
      pieces.push(input.text);
      if (!input.done) {
        expect(publications).toBe(0);
        return { accepted: true, staged: true, revision: 7 };
      }
      expect(JSON.parse(pieces.join(""))).toEqual(commit);
      publications++;
      return { accepted: true, revision: 8 };
    });
    expect(await transport(commit)).toEqual({ accepted: true, revision: 8 });
    expect(publications).toBe(1);
  });
}

test("a rejected piece stops the transfer and propagates rejection", async () => {
  let calls = 0;
  const transport = chunkedSurfaceTransport(() => {
    calls++;
    return { accepted: false, revision: 0, error: "limit" };
  });
  expect(
    (
      await transport({
        baseRevision: 0,
        ops: [{ op: "setText", id: 1, text: "a".repeat(100_000) }],
      })
    ).accepted,
  ).toBe(false);
  expect(calls).toBe(1);
});
