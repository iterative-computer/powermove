import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { ACTIONS, coverage, grade } from "./contract";
import type { Projection } from "./model";

const catalog = JSON.parse(
  readFileSync(
    new URL(
      "../../../../crates/mcp/fixtures/catalog-main-c7f46a056.json",
      import.meta.url,
    ),
    "utf8",
  ),
) as { tools: { name: string }[] };
const operations = [
  ...readFileSync(
    new URL("../../../../crates/mcp/src/commands.rs", import.meta.url),
    "utf8",
  )
    .split("pub const OPERATIONS: &[&str] = &[")[1]
    .split("];")[0]
    .matchAll(/"([a-z_]+)"/g),
].map((m) => m[1]);

// The native tools served beside main's catalog, and the agent's commands.
const native = JSON.parse(
  readFileSync(
    new URL(
      "../../../../crates/mcp/fixtures/native-tools.json",
      import.meta.url,
    ),
    "utf8",
  ),
) as { name: string }[];
const agentCommands = [
  ...readFileSync(
    new URL("../../../../crates/app/src/agent_commands.rs", import.meta.url),
    "utf8",
  )
    .split("pub const AGENT")[1]
    .split("];")[0]
    .matchAll(/\(\s*"([A-Za-z.]+)"/g),
].map((m) => m[1]);

test("every panel action names agent tools and operations that exist", () => {
  const tools = new Set([
    ...catalog.tools.map((t) => t.name),
    ...native.map((t) => t.name),
    ...agentCommands,
  ]);
  expect(agentCommands).toContain("color.importCdlFile");
  expect(operations).toContain("add_effect");
  for (const { action, agent } of ACTIONS) {
    expect(agent.length).toBeGreaterThan(0);
    for (const name of agent)
      expect(
        tools.has(name) || operations.includes(name),
        `${action}: ${name}`,
      ).toBe(true);
  }
});

const header = (
  id: string,
  definition: string,
  grade: boolean,
  expanded = true,
) => ({
  kind: "effect" as const,
  id,
  definition,
  grade,
  label: definition,
  enabled: true,
  expanded,
  missing: false,
  selected: false,
  ordinal: 0,
  effectCount: 3,
});
const row = (effect: string, key: string, value: unknown) => ({
  kind: "parameter" as const,
  id: `parameter:${effect}.${key}`,
  effectLabel: effect,
  enabled: true,
  parameter: {
    id: `${effect}.${key}`,
    effect,
    key,
    label: key,
    binding: `b:${effect}.${key}`,
    value,
    control: "number" as const,
    animated: false,
    keyAtPlayhead: false,
    mixed: false,
    readonly: false,
  },
});
const projection = (rows: any[], range: [number, number], count: number) =>
  ({
    version: 1,
    target: "Layer",
    layer: "L",
    name: "Clip",
    typeLabel: "Video",
    locked: false,
    revision: 3,
    documentId: 1,
    count,
    effectCount: 3,
    selectedCount: 0,
    range,
    rows,
    focus: null,
    focusLabel: null,
    clipboardCount: 0,
  }) as Projection;

test("the panel lists grading effects only, an extension's included", () => {
  const p = projection(
    [
      header("w", "colorWheels", true),
      row("w", "gainMaster", 1),
      header("b", "blur", false),
      row("b", "radius", 4),
      header("x", "ext.filmLook", true),
      row("x", "strength", 0.5),
    ],
    [0, 6],
    6,
  );
  const tools = grade(p);
  expect(tools.map((g) => g.header.definition)).toEqual([
    "colorWheels",
    "ext.filmLook",
  ]);
  expect(tools[1].parameters.map((r) => r.parameter.key)).toEqual(["strength"]);
});

test("the panel asks for every row and opens every grading tool", () => {
  const partial = projection([header("w", "colorWheels", true)], [0, 1], 40);
  expect(coverage(partial)).toEqual([{ type: "range", range: [0, 40] }]);
  const closed = projection(
    [
      header("w", "colorWheels", true, false),
      header("b", "blur", false, false),
    ],
    [0, 2],
    2,
  );
  expect(coverage(closed)).toEqual([{ type: "disclose", id: "w", open: true }]);
});
