import { test, expect } from "bun:test";
import { tasks } from "../eval/tasks";
import { runTask } from "../eval/runner";
const wire = process.env.PM_SURFACE_WIRE;
test("eval scaffold has four ten-task groups and never passes missing owner specs", async () => {
  expect(tasks).toHaveLength(40);
  for (const group of ["builtin", "wild", "held-out", "owner"])
    expect(tasks.filter((t) => t.group === group)).toHaveLength(10);
  expect(await runTask("owner-01")).toEqual({
    id: "owner-01",
    status: "pending-owner",
  });
});
for (const task of tasks.filter((t) => t.status === "ready"))
  test(`${task.id}: ${task.brief}`, async () => {
    const result = await runTask(task.id, wire);
    expect(result).toMatchObject({ id: task.id, status: "pass" });
  });
