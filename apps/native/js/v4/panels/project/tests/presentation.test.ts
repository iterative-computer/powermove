import { expect, test } from "bun:test";
import { renditions } from "../presentation";

test("queued and generating previews use quiet per-clip preparation text", () => {
  expect(renditions({ proxy: "Queued" })).toBe("Preparing preview");
  expect(renditions({ proxy: { Generating: { percent: 42 } } })).toBe("Preparing preview 42%");
  expect(renditions({ proxy: { Ready: {} } })).toBe("Proxy");
  expect(renditions({ proxy: null, optimized: "Queued" })).toBe("Optimized Waiting");
});
