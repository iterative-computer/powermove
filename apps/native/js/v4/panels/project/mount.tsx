import { ProjectPanel, ProjectService } from "./index";
import type { BuiltinEnv, Mounted } from "../../src/builtin";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { native, platform, host, fields, report, root } = env;
  const service = new ProjectService(host, {
    command: "panel.project",
    subscription: "project",
    projectStamp: (p: any) => p.stamp ?? null,
  });
  await service.page(0, 96);
  await service.start();
  const r = root();
  await r.render(
    <ProjectPanel
      service={service}
      layout={native("project.layout")}
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
