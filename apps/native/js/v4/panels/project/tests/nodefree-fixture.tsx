import { createRoot } from "../../../src";
import { ProjectView } from "../index";
const globals = [
  "process",
  "Buffer",
  "window",
  "document",
  "navigator",
] as const;
const previous = globals.map(
  (key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
for (const key of globals)
  Object.defineProperty(globalThis, key, {
    value: undefined,
    configurable: true,
  });
try {
  const root = createRoot(async (commit) => ({
    accepted: true,
    revision: commit.baseRevision + 1,
  }));
  await root.render(
    <ProjectView
      view={null}
      input={() => {}}
      menu={() => {}}
      page={() => {}}
      native={{
        searchBinding: "fixture:search",
        renameBinding: "fixture:rename",
        activatePanel() {},
        menu() {},
      }}
      layout={{ columns: 1, rowHeight: 160 }}
    />,
  );
  if (!root.inspect().nodes.some((n) => n.props.label === "No Project"))
    throw new Error("missing project panel");
  await root.dispose();
} finally {
  for (const [key, value] of previous) {
    if (value) Object.defineProperty(globalThis, key, value);
    else Reflect.deleteProperty(globalThis, key);
  }
}
console.log("project panel without Node/DOM globals");
