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
  /(?:^|\n)\s*(?:import|export)\s+(type\s+)?[^'"\n;]*?from\s+['"]([^'"]+)['"]|(?:^|\n)\s*import\s+['"]([^'"]+)['"]/g;
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
await Bun.$`rm -rf ${target}`;
for (const [file, text] of seen)
  await Bun.write(join(target, file), text);
console.log(`${seen.size} files from ${reference}`, relative(root, target));
