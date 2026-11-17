import { createRoot } from "../../src";
import type { BuiltinEnv, Mounted } from "../../src/builtin";
import { COMMAND, EasingModel, librarySave, type EasingState } from "./model";
import { EasingPanel } from "./panel";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { native, host, transport, rootOptions, command, fields, report } = env;
  const model: EasingModel = new EasingModel(
    () => native("easing.state") as EasingState,
    (action) =>
      action.type === "save"
        ? command(
            "panel.timeline",
            librarySave(model.snapshot(), action.name, action.curve),
          )
        : command(COMMAND, action),
    report,
  );
  const root = createRoot(transport, report, rootOptions);
  const cleanups: (() => Promise<void> | void)[] = [];
  try {
    // The selection says which keyframes the actions change; the project,
    // whether there is anything to ease.
    for (const query of ["selection", "project"] as const)
      cleanups.push(
        await host.subscribe(`easing:${query}`, query, () => model.refresh()),
      );
    model.refresh();
    await root.render(
      <EasingPanel
        model={model}
        bindings={fields([
          "curve",
          "outX",
          "outY",
          "inX",
          "inY",
          "interpolation",
          "name",
        ])}
      />,
    );
  } catch (error) {
    await root.dispose();
    for (const cleanup of cleanups) await cleanup();
    throw error;
  }
  return {
    root,
    // The shared selection query tracks layers, not keyframe-only changes.
    // Read the native key selection on each publication wake too.
    publish: () => model.refresh(),
    settle: () => model.settled(),
    refresh: async () => model.refresh(),
    async dispose() {
      await root.dispose();
      for (const cleanup of cleanups) await cleanup();
    },
  };
}
