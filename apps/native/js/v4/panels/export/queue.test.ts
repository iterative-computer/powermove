import { expect, test } from "bun:test";
import { jobActions, jobFormat, jobTime } from "./queue";
import type { QueueJob } from "./model";
test("queue action policy follows each status", () => {
  expect(jobActions("Pending")).toEqual(["cancel"]);
  expect(jobActions("Rendering")).toEqual(["cancel"]);
  expect(jobActions("Failed")).toEqual(["retry", "remove"]);
  expect(jobActions("Done")).toEqual(["finder", "remove"]);
  expect(jobActions("Cancelled")).toEqual(["remove"]);
});
test("export captions use proper codec casing and history uses relative dates", () => {
  expect(jobFormat({ format: "prores", profile: "422" } as QueueJob)).toBe(
    "ProRes 422",
  );
  expect(jobFormat({ format: "mp4" } as QueueJob)).toBe("MP4");
  const now = new Date(2026, 10, 15, 17, 0);
  expect(jobTime(new Date(2026, 10, 14, 14, 2).getTime() / 1000, now)).toBe(
    "Yesterday 14:02",
  );
  expect(jobTime(new Date(2026, 10, 15, 14, 2).getTime() / 1000, now)).toBe(
    "Today 14:02",
  );
  expect(jobTime(undefined, now)).toBe("");
});
