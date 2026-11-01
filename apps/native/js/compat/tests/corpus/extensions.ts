/** Public synthetic extensions; no installed extension source or user projects. */
import type { Data } from "../../src/types";
export const manifests = [1, 2, 3].map((apiVersion) => ({
  id: `synthetic-contract-v${apiVersion}`,
  name: "Synthetic contract corpus",
  version: "1.0.0",
  apiVersion,
}));
export function activateRegistrations(api: Data, trace: any[]): void {
  api.panels.register({
    id: "synthetic-panel",
    title: "Synthetic",
    build: (body: Data) => {
      body.textContent = "mounted";
      trace.push("panel");
    },
  });
  api.commands.register({
    id: "synthetic-command",
    label: "Synthetic",
    when: () => true,
    run: (...args: any[]) => {
      trace.push(["command", args]);
      return args[0];
    },
  });
  api.keybindings.bind({ key: "cmd+shift+k", command: "synthetic-command" });
  api.effects.register({
    id: "SyntheticFx",
    label: "Synthetic",
    group: "Synthetic",
    params: [],
    frag: "vec4 effect(vec2 uv) { return texture(u_tex, uv); }",
  });
  api.transitions.register({
    id: "SyntheticTransition",
    label: "Synthetic",
    group: "Synthetic",
    params: [],
    frag: "vec4 transition(vec2 uv) { return mix(texture(u_from, uv), texture(u_to, uv), u_prog); }",
  });
  api.layers.register({
    id: "synthetic-layer",
    version: 1,
    renderer: {
      kind: "fragment",
      fragment: "void main() { fragColor = vec4(1.); }",
    },
    label: "Synthetic",
    group: "Synthetic",
    params: [],
    defaults: {},
    frag: "vec4 layer(vec2 uv) { return vec4(uv, 0., 1.); }",
  });
  api.theme.register({
    id: "synthetic-theme",
    name: "Synthetic",
    tokens: { "--accent": "#123456" },
  });
  api.theme.activate("synthetic-theme");
  api.theme.setScheme("dark");
  api.palette.registerProvider((query: string) => [
    { id: "synthetic-entry", label: query, run: () => trace.push("palette") },
  ]);
  api.menus.contribute("panel:context", (context: Data) => [
    { label: String(context.label), action: () => trace.push("menu") },
  ]);
  api.status.register({
    id: "synthetic-status",
    text: () => "Synthetic",
    onClick: () => trace.push("status"),
  });
  api.inspector.registerSection({
    id: "synthetic-section",
    title: "Synthetic",
    build: (target: Data) => {
      target.textContent = "inspector";
      return () => trace.push("inspector-cleanup");
    },
  });
  api.events.on("synthetic:event", (value: unknown) =>
    trace.push(["event", value]),
  );
  api.onDispose(() => trace.push("disposed"));
}
export function activateRuntime(api: Data, trace: any[]): void {
  const marker = {};
  api.services.register("synthetic-service", marker);
  api.commands.register({
    id: "synthetic-edit",
    label: "Synthetic",
    run: () =>
      api.history.do("Synthetic raw edit", () => {
        api.host.pm.proj.name = "Synthetic edit";
        return marker;
      }),
  });
  trace.push(api.commands.run("synthetic-edit") === marker);
  trace.push(api.project.get() === api.host.pm.proj);
  trace.push(api.project.undo(), api.project.redo());
  api.storage.set("synthetic", { value: 1 });
  trace.push(api.storage.get("synthetic"));
  api.storage.delete("synthetic");
  trace.push(api.vars.keys(), api.vars.get("SYNTHETIC_VALUE"));
}
