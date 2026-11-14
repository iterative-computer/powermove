import { ProjectPanel, ProjectService, type ViewMode } from "./index";
import type { BuiltinEnv, Mounted } from "../../src/builtin";

/** Poster keys are the host's own picture keys (`thumb/…`): the host draws
 * them, so they pass through unchanged. */
const hostPosters: ReadonlyMap<string, string> = {
  get: (key: string) => key,
} as unknown as ReadonlyMap<string, string>;

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { native, platform, host, fields, report, root } = env;
  const service = new ProjectService(host, {
    command: "panel.project",
    subscription: "project",
    projectStamp: (p: any) => p.stamp ?? null,
  });
  await service.page(0, 96);
  // The list/grid choice is the person's, kept by the host across launches.
  const viewMode: ViewMode = await platform("project.view").then(
    (mode) => (mode === "grid" ? "grid" : "list"),
    () => "list",
  );
  await service.start();
  const r = root();
  await r.render(
    <ProjectPanel
      service={service}
      layout={native("project.layout", { view: viewMode })}
      viewMode={viewMode}
      layoutFor={(view, width) => native("project.layout", { view, width })}
      onViewMode={(view) => {
        void platform("project.view", { view }).catch(report);
      }}
      posters={hostPosters}
      native={{
        ...fields(["searchBinding", "renameBinding"]),
        menu(entries, choose, context) {
          void platform("project.menu", { entries, context })
            .then((index) => {
              if (index !== null) choose(entries[index]);
            })
            .catch(report);
        },
      }}
    />,
  );
  const mounted: Mounted = {
    root: r,
    async dispose() {
      await service.dispose();
      await r.dispose();
    },
  };
  mounted.settle = () => service.idle();
  return mounted;
}
