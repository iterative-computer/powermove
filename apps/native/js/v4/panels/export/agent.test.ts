import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import catalog from "../../../../crates/mcp/fixtures/studio3d-native-catalog.json";
import {
  audioCodings,
  codecs,
  colorTags,
  dynamicRanges,
  fields,
  profiles,
  qualities,
  sidecars,
  studioQualities,
} from "./spec";

/** What an agent may set in `export_video`. */
const agent = (
  catalog.tools.find((t) => t.name === "export_video") as {
    inputSchema: { properties: Record<string, any> };
  }
).inputSchema.properties;

/** Sheet options an agent sets under another name. */
const renamed: Record<string, string> = {
  mblur: "motionBlur",
  // The sheet's reduced size is a 3D render scale of 0.75.
  studioReducedSize: "studioRenderScale",
};
/** Sheet options no video export has. */
const notVideo = new Set(["depth"]);

/** Every option a person sets in the sheet: each row's `select`, `toggle`
 * and `edit`, the format, and a custom size. A new row lands here by
 * itself, and fails below until the agent has it too. */
const sheet = [
  ...new Set([
    ...[
      ...readFileSync(`${import.meta.dir}/panel.tsx`, "utf8").matchAll(
        /\b(?:select|toggle|edit)\(\s*"([a-zA-Z]+)"/g,
      ),
    ].map((m) => m[1]),
    "format",
    "width",
    "height",
  ]),
];

test("every option a person has in the sheet for a video is an export_video field", () => {
  expect(sheet.length).toBeGreaterThan(15);
  const missing = sheet.filter(
    (key) => !notVideo.has(key) && !((renamed[key] ?? key) in agent),
  );
  expect(missing).toEqual([]);
  // The rows of every video format are among them.
  for (const format of ["prores", "mp4", "webm"] as const)
    for (const field of fields[format]) expect(sheet).toContain(field);
});

test("export_video offers every choice the sheet does", () => {
  const choices = (key: string) =>
    (agent[key].enum ?? agent[key].oneOf?.[0]?.enum) as string[];
  expect(choices("format")).toEqual(["mp4", "prores", "webm"]);
  const values = (list: { v: string | number }[]) =>
    list.map((o) => String(o.v));
  expect(choices("profile")).toEqual(values(profiles));
  expect(choices("codec")).toEqual(values(codecs));
  expect(choices("quality")).toEqual(values(qualities));
  expect(choices("colorTag")).toEqual(values(colorTags));
  expect(choices("audioCoding")).toEqual(values(audioCodings));
  expect(choices("dynamicRange")).toEqual(values(dynamicRanges));
  expect(choices("captionsSidecar")).toEqual(values(sidecars));
  expect(choices("range")).toEqual(["all", "work"]);
  // "composition" is what an agent gets by leaving the field out.
  expect(choices("studioQuality")).toEqual(
    values(studioQualities("")).filter((v) => v !== "composition"),
  );
});
