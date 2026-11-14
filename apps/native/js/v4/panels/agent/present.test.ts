import { expect, test } from "bun:test";
import {
  activity,
  breakable,
  durationLabel,
  entries,
  inline,
  markdown,
  presentError,
  slashCopy,
  workDuration,
} from "./present";
import { withRows } from "./fixtures";
import type { Row } from "./contract";

const step = (id: string, value: Row["value"], trace?: Row["trace"]): Row => ({
  id,
  commandId: id,
  chosen: {},
  open: false,
  value,
  ...(trace ? { trace } : {}),
});

test("a provider's JSON error reads as words; the raw text stays as details", () => {
  const raw =
    '{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}';
  expect(presentError(raw)).toEqual({
    title: "Agent is temporarily unavailable",
    message:
      "Wait a moment, then send your request again. Check your account if you’ve reached a usage limit.",
    details: raw,
  });
  const unknown =
    'stream error: {"type":"error","error":{"message":"The model gpt-x does not exist."}}';
  const p = presentError(unknown);
  expect(p.title).toBe("Something needs attention");
  expect(p.message).toBe("stream error: The model gpt-x does not exist.");
  // Unreadable JSON never becomes the message.
  expect(presentError('{"code":-32600}').message).toStartWith(
    "This operation couldn’t finish.",
  );
  expect(presentError("401 Unauthorized").title).toBe(
    "Sign-in needs attention",
  );
});
test("consecutive calls share one group; prose and questions break it", () => {
  const rows = activity([
    step("t1", { kind: "text", text: "Looking." }),
    step("c1", {
      kind: "tool",
      toolName: "get_project_state",
      label: "Get project state",
      status: "done",
      startedAt: 0,
      endedAt: 400,
    }),
    step("c2", {
      kind: "tool",
      toolName: "bash",
      label: "Run",
      detail: "ls",
      status: "error",
      startedAt: 400,
      endedAt: 2400,
    }),
    step("q", { kind: "question", status: "open", questions: [] }),
    step("c3", {
      kind: "tool",
      toolName: "apply_patch",
      label: "Edit",
      detail: "a/b.ts, c.ts",
      status: "done",
    }),
  ]);
  expect(rows.map((r) => r.kind)).toEqual([
    "text",
    "tools",
    "question",
    "tools",
  ]);
  const group = rows[1]!;
  if (group.kind !== "tools") throw new Error("group");
  expect(group.label).toBe("2 tool calls");
  expect(group.status).toBe("partial");
  expect(group.meta).toBe("1 failed · 2s");
  expect(group.summary).toBe("Inspected the project and ran commands");
  const edits = rows[3]!;
  if (edits.kind !== "tools") throw new Error("edits");
  expect(edits.files).toEqual(["b.ts", "c.ts"]);
});
test("a finished turn folds its work; the running turn stays open under its timer", () => {
  const trace = { id: "t", durationMs: 150_000, steering: false };
  const rows = [
    step("history:u", { role: "user", text: "Go" }),
    step(
      "history:t/step-0:a",
      { id: "a", kind: "tool", toolName: "bash", label: "Run", status: "done" },
      trace,
    ),
    step("history:t/step-1:b", { id: "b", kind: "text", text: "Done." }, trace),
  ];
  const done = entries(withRows(rows).snapshot);
  expect(done.map((e) => e.kind)).toEqual(["message", "work", "message"]);
  expect(done[1]!.kind === "work" && done[1]!.label).toBe("Worked for 2m 30s");
  const live = withRows([
    rows[0]!,
    step("live:a", { id: "a", kind: "tool", status: "running" }),
  ]).snapshot;
  live.runId = "run";
  live.elapsedMs = 12_000;
  expect(
    entries(live).map((e) => (e.kind === "working" ? e.label : e.kind)),
  ).toEqual(["message", "Working for 12s", "message"]);
});
test("Markdown becomes native blocks and styled runs, never markup", () => {
  expect(
    markdown("# Title\n\n- one\n- **two**\n\n```\ncode\n```\n> quote"),
  ).toEqual([
    { kind: "h", level: 1, runs: [{ t: "Title" }] },
    { kind: "li", marker: "•", depth: 0, runs: [{ t: "one" }] },
    { kind: "li", marker: "•", depth: 0, runs: [{ t: "two", b: true }] },
    { kind: "code", text: "code" },
    { kind: "quote", runs: [{ t: "quote" }] },
  ]);
  expect(inline("see [docs](https://x.y) and `a_b` in *it*")).toEqual([
    { t: "see " },
    { t: "docs", a: true },
    { t: " and " },
    { t: "a_b", c: true },
    { t: " in " },
    { t: "it", i: true },
  ]);
  // snake_case words are not emphasis.
  expect(inline("get_project_state")).toEqual([{ t: "get_project_state" }]);
});
test("durations and shortcut copy match the original panel", () => {
  expect(durationLabel(400)).toBe("<1s");
  expect(durationLabel(72_000)).toBe("1m 12s");
  expect(workDuration(5_400)).toBe("5s");
  const s = withRows([]).snapshot;
  expect(slashCopy("new", null, s)).toEqual({
    label: "/new",
    description: "Start a new conversation",
  });
  expect(slashCopy("effort-xhigh", "xhigh", s)).toEqual({
    label: "Extra high",
    description: "Set effort",
  });
});

test("long unbroken tokens get break points; ordinary words do not", () => {
  expect(breakable("a short sentence.")).toBe("a short sentence.");
  expect(breakable('{"type":"error","error":{"message":"x"}}')).toContain(
    "\u200b",
  );
  expect(
    breakable("/Users/someone/Movies/project/final.mov").replaceAll(
      "\u200b",
      "",
    ),
  ).toBe("/Users/someone/Movies/project/final.mov");
});
