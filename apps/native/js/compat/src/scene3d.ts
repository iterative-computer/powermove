/** v1–3's `api.scene3d` (core/scene3d/api.ts) over the native Studio command.
 * The native viewer shows the composition camera only: the editor view, its
 * navigation and the DOM gizmo hosts belong to the 3D studio, which this build
 * lacks, so changing them says so instead of pretending. */
import { HostError } from "./channel";
import type { Data } from "./types";
import {
  createLight,
  createObject,
  createScene,
  SCENE3D_DEFINITION,
} from "./frozen/reference/core/scene3d/schema";
import {
  isModelGroup,
  layer3DRole,
} from "./frozen/reference/core/scene3d/layers";

export function scene3dAPI(
  PM: Data,
  edit: (args: Data, meta: Data) => Data,
  describe: (time: number) => Data,
  emit: (selection: { layerId: string | null; ids: string[] }) => void,
): Data {
  const studio = (what: string): never => {
    throw new HostError(
      "Unavailable",
      `scene3d.${what} needs the 3D studio, which this build does not have`,
    );
  };
  const is3D = (id: string) => {
    const layer = PM.L?.(id);
    return !!layer3DRole(layer) || isModelGroup(PM, layer);
  };
  const selection = () => {
    const ids: string[] = (PM.sel?.layers ?? []).filter(is3D);
    return { layerId: ids[0] ?? null, ids };
  };
  // Fixed states never change, so their listeners are never called.
  const fixed = (name: string, value: string) => ({
    get: () => value,
    set: (next: string) => {
      if (next !== value) studio(`${name}('${next}')`);
    },
    on: () => ({ dispose() {} }),
  });
  const view = fixed("setView", "camera");
  const navigation = fixed("setNavigationMode", "select");
  const mode = fixed("setMode", "translate");
  const space = fixed("setSpace", "world");
  return {
    isGroup: (layerId: string) => isModelGroup(PM, PM.L?.(layerId)),
    getView: view.get,
    setView: view.set,
    onViewChange: view.on,
    getNavigationMode: navigation.get,
    setNavigationMode: navigation.set,
    onNavigationChange: navigation.on,
    createNavigation: () => studio("createNavigation"),
    frame: () => studio("frame"),
    getMode: mode.get,
    setMode: mode.set,
    getSpace: space.get,
    setSpace: space.set,
    onGizmoChange: mode.on,
    createGizmo: () => studio("createGizmo"),
    // A standalone model imports as itself; packing OBJ/MTL/textures into one
    // GLB is the studio's.
    prepareImport: async (files: File[]) => {
      const list = [...(files ?? [])];
      return list.length === 1 ? list[0] : studio("prepareImport");
    },
    // Projects upgrade old 3D layers when they open (as main's convert did).
    convert: (layerId: string) => {
      const layer = PM.L?.(layerId);
      if (!layer) return { ok: false, message: "3D layer not found" };
      if (layer3DRole(layer))
        return {
          ok: true,
          message: "Already a 3D layer",
          data: { result: { layerId, id: layerId } },
        };
      return {
        ok: false,
        message:
          layer.d?.definition === SCENE3D_DEFINITION
            ? "This 3D layer upgrades when the project opens"
            : "Not a 3D model layer",
      };
    },
    createScene,
    createObject,
    createLight,
    edit: (args: Data, meta: Data = {}) => edit(args, meta),
    // The kernel evaluates the scene: its material model is newer than the
    // frozen reference's schema.
    describe: () => describe(PM.time),
    selection,
    select: (layerId: string | null, ids: string[]) => {
      const selected = ids?.length ? ids : layerId ? [layerId] : [];
      PM.selectLayers(selected.filter(is3D));
      emit(selection());
      PM.bus?.emit?.("scene3d:selection");
    },
  };
}
