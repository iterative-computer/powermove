import { expect, test } from "bun:test";
import { replayScenario, scenarios } from "./replay";
for (const name of scenarios)
  test(`frozen ${name}: public-boundary projection`, async () => {
    const result = await replayScenario(name);
    expect(result.differences).toEqual([]);
    expect(result.matched).toBe(result.boundaries);
  });
