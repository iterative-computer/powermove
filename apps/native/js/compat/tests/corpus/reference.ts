/** Test-only source replay. This reads the tag, never candidate code as oracle. */
import ts from "typescript";
import * as space from "../../src/frozen/legacy/core/space-3d";
import type { Data } from "../../src/types";
export const reference = "07ed5fa17ee640659cb075f9cad1af2095011d60";
export async function tagSource(path: string): Promise<string> {
  const p = Bun.spawn(
    ["git", "show", `${reference}:apps/desktop/src/renderer/src/${path}`],
    { stdout: "pipe", stderr: "pipe" },
  );
  const source = await new Response(p.stdout).text();
  if (await p.exited)
    throw new Error("Missing pinned v1.2.0 reference: " + path);
  return source;
}
const families: Record<string, string> = {
  anim: "makeAnim",
  model: "makeModel",
  selection: "makeSelection",
  groups: "makeGroups",
  transport: "makeTransport",
  media: "makeMedia",
  render: "makeRender",
  uiState: "makeUIState",
  util: "makeUtil",
  ease: "makeEase",
  space3d: "makeSpace3D",
};
export async function sourceFacades(PM: Data): Promise<Data> {
  const source = await tagSource("kernel/install.ts"),
    tree = ts.createSourceFile(
      "install.ts",
      source,
      ts.ScriptTarget.Latest,
      true,
    );
  const functions = tree.statements
    .filter(
      (node): node is ts.FunctionDeclaration =>
        ts.isFunctionDeclaration(node) &&
        Object.values(families).includes(node.name?.text ?? ""),
    )
    .map((node) => node.getText(tree))
    .join("\n");
  const code = ts.transpile(functions, {
    target: ts.ScriptTarget.ES2023,
    module: ts.ModuleKind.ESNext,
  });
  const helpers = Object.keys(space);
  return new Function(
    "PM",
    ...helpers,
    `const EMPTY_AFFINE=[1,0,0,1,0,0], EMPTY_MAT3=[1,0,0,0,1,0,0,0,1], EMPTY_MAT4=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1];\n${code}\nreturn { ${Object.entries(
      families,
    )
      .map(([key, fn]) => `${key}: ${fn}(PM)`)
      .join(",")} };`,
  )(PM, ...helpers.map((name) => (space as Data)[name]));
}
export async function contract(): Promise<
  Record<string, { member: string; optional: boolean }[]>
> {
  const source = await tagSource("kernel/api.ts"),
    tree = ts.createSourceFile("api.ts", source, ts.ScriptTarget.Latest, true);
  const interfaces = new Map(
    tree.statements
      .filter(ts.isInterfaceDeclaration)
      .map((node) => [node.name.text, node]),
  );
  const api = interfaces.get("PowermoveAPI")!;
  const result: Record<string, { member: string; optional: boolean }[]> = {};
  for (const node of api.members) {
    if (
      !ts.isPropertySignature(node) ||
      !node.type ||
      !ts.isTypeReferenceNode(node.type)
    )
      continue;
    const definition = interfaces.get(node.type.typeName.getText(tree));
    if (!definition) continue;
    result[node.name!.getText(tree)] = [
      ...new Map(
        definition.members.map((member) => [
          member.name!.getText(tree),
          {
            member: member.name!.getText(tree),
            optional: !!(member as ts.PropertySignature).questionToken,
          },
        ]),
      ).values(),
    ];
  }
  return result;
}
