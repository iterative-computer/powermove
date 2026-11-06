/** Verbatim copy of the current reference's pure runtime closure. Run with Bun. */
import { dirname, join, normalize, relative } from "node:path";
const root = join(import.meta.dir, "../../../../..");
const reference = "c7f46a05641b23944d3692c9d80f9677c7e1ef1d";
const base = "apps/desktop/src/renderer/src/";
const target = join(import.meta.dir, "../src/frozen/reference");
const entries = [
  "legacy/core/anim.ts",
  "legacy/core/easing.ts",
  "legacy/core/selection.ts",
  "legacy/core/ui-state.ts",
  "legacy/core/layer-groups.ts",
  "legacy/core/project-index.ts",
  ...[
    "NumField",
    "ColorField",
    "FillField",
    "RampField",
    "PointField",
    "SliderField",
    "FontField",
    "SelectField",
    "TextField",
    "ToggleField",
    "Row",
    "Section",
    "Segmented",
    "Disclosure",
  ].map((name) => `controls/${name}.svelte`),
  "state/document.svelte.ts",
  "state/selection.svelte.ts",
  "state/transport.svelte.ts",
];
const read = async (file: string) => {
  const process = Bun.spawn(["git", "show", `${reference}:${base}${file}`], {
    cwd: root,
    stdout: "pipe",
    stderr: "ignore",
  });
  const text = await new Response(process.stdout).text();
  return (await process.exited) ? null : text;
};
const candidates = (file: string) =>
  /\.(ts|css|svelte)$/.test(file)
    ? [file, `${file}.ts`]
    : [`${file}.ts`, `${file}.svelte.ts`, `${file}/index.ts`];
const imports =
  /(?:^|\n)\s*(?:import|export)\s+(type\s+)?[^'";]*?from\s+['"]([^'"]+)['"]|(?:^|\n)\s*import\s+['"]([^'"]+)['"]/g;
const seen = new Map<string, string>();
const queue = [...entries];
while (queue.length) {
  const file = queue.pop()!;
  if (seen.has(file)) continue;
  const text = await read(file);
  if (text === null) throw Error(`Cannot read frozen ${file}`);
  seen.set(file, text);
  for (const match of text.matchAll(imports)) {
    if (match[1]) continue;
    const specifier = match[2] ?? match[3]!;
    if (!specifier.startsWith(".")) continue;
    const resolved = normalize(join(dirname(file), specifier));
    if (resolved.startsWith("..")) continue;
    let found = false;
    for (const candidate of candidates(resolved))
      if (seen.has(candidate) || (await read(candidate)) !== null) {
        queue.push(candidate);
        found = true;
        break;
      }
    if (!found) throw Error(`Cannot resolve ${specifier} from ${file}`);
  }
}
const model = (await read("legacy/core/model.ts"))!;
const media = (await read("legacy/core/media.ts"))!;
const blockOf = (text: string, from: string) => {
  const start = text.indexOf(from);
  if (start < 0) throw Error(`Unsupported source: ${from}`);
  let depth = 0;
  for (let at = text.indexOf("{", start); at < text.length; at++) {
    if (text[at] === "{") depth++;
    else if (text[at] === "}" && --depth === 0) return text.slice(start, at + 1);
  }
  throw Error(`Unterminated source: ${from}`);
};
seen.set(
  "legacy/core/pure-model.ts",
  `/* Selection and media timing of legacy/core/model.ts and media.ts. */\nimport { resolveContent } from "./content-properties";\nimport { clipCuesToEnd, rebaseCuesForStart } from "../../captions/model";\nimport { timelineService } from "./services";\nimport type { PMRegistry } from "../registry";\n${blockOf(model, "export function selectLayers")}\nexport function installMediaTiming(PM: any): void {\n${blockOf(media, "PM.MediaTiming = {")};\n}\n`,
);
queue.push("captions/model.ts", "legacy/core/services.ts", "legacy/registry.ts");
while (queue.length) {
  const file = queue.pop()!;
  if (seen.has(file)) continue;
  const text = await read(file);
  if (text === null) throw Error(`Cannot read frozen ${file}`);
  seen.set(file, text);
  for (const match of text.matchAll(imports)) {
    if (match[1] || !(match[2] ?? match[3])!.startsWith(".")) continue;
    const resolved = normalize(join(dirname(file), (match[2] ?? match[3])!));
    for (const candidate of candidates(resolved))
      if (seen.has(candidate) || (await read(candidate)) !== null) {
        queue.push(candidate);
        break;
      }
  }
}
const util = (await read("legacy/core/util.ts"))!;
const slice = (from: string, to: string) => {
  const start = util.indexOf(from);
  const end = util.indexOf(to, start);
  if (start < 0 || end < 0) throw Error(`Unsupported util source: ${from}`);
  return util.slice(start, end);
};
seen.set(
  "legacy/core/pure-util.ts",
  `/* Pure math and time helpers of legacy/core/util.ts. */\nexport function install(PM: any): void {\n${slice("/* ── math", "PM.uid =")}${slice("/* Colours are", "/* ── event bus")}}\n`,
);
const shaders = (await read("legacy/gl/shaders.ts"))!;
const literal = (from: string) => {
  const start = shaders.indexOf("`", shaders.indexOf(from)) + 1;
  const end = shaders.indexOf("`", start);
  if (!start || end < 0) throw Error(`Unsupported shader source: ${from}`);
  return JSON.stringify(shaders.slice(start, end));
};
seen.set(
  "legacy/gl/shader-prelude.ts",
  `/* The effect and shader-layer preambles of legacy/gl/shaders.ts. */\nexport const GLSL_PRE = ${literal("const PRE = `")};\nexport const SHADER_HEADER = ${literal("PM.SHADER_HEADER = `")};\n`,
);
await Bun.$`rm -rf ${target}`;
for (const [file, text] of seen)
  await Bun.write(join(target, file), text);
console.log(`${seen.size} files from ${reference}`, relative(root, target));
