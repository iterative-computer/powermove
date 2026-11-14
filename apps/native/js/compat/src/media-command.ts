/** Asset-to-layer commands retain the shipped media API without editing the document. */
import { createObject } from "./frozen/reference/core/scene3d/schema";
import { LAYER3D_DEFINITIONS } from "./frozen/reference/core/scene3d/layers";
import {
  makeVectorPath,
  makeVertex,
} from "./frozen/reference/legacy/core/vector-paths";
const IMPORT_DEFAULTS_SERVICE = "media.importDefaults";

function materializeSvgPaths(PM: any, imported: any) {
  return imported.paths.map((record: any) => {
    const path = makeVectorPath(PM, record.name);
    Object.assign(path.p, {
      closed: PM.P(record.closed),
      fill: PM.P(record.fill),
      fillEnabled: PM.P(record.fillEnabled),
      fillOpacity: PM.P(record.fillOpacity),
      stroke: PM.P(record.stroke),
      strokeWidth: PM.P(record.strokeWidth),
      strokeOpacity: PM.P(record.strokeOpacity),
    });
    path.vertices = record.vertices.map((recordVertex: any) => {
      const result = makeVertex(PM, recordVertex.x, recordVertex.y);
      for (const key of ["inX", "inY", "outX", "outY"] as const)
        result.p[key].v = recordVertex[key];
      return result;
    });
    return path;
  });
}

export function mediaCommand(PM: any, kernel: any) {
  const nativeCommandForAsset = (id?: any, at: any = PM.time) => {
    const meta: any = PM.proj.assets[id];
    if (!meta) return;
    const a: any = PM.assets?.get?.(id) || meta;
    if (a.kind === "model") {
      const definition = a.layerDefinition || LAYER3D_DEFINITIONS.object;
      if (!PM.layerDefinition?.(definition)) return;
      return {
        type: "add_layer",
        layerType: "extension",
        name: a.name,
        from: Math.round(at * PM.proj.fps) / PM.proj.fps,
        duration: Math.max(1 / PM.proj.fps, PM.proj.dur - at),
        content:
          definition === LAYER3D_DEFINITIONS.object
            ? {
                definition,
                data: {
                  object: {
                    source: { assetId: id },
                    material: createObject("model").material,
                    useSourceMaterials:
                      a.format === "glb" || a.format === "gltf",
                    castShadow: true,
                    receiveShadow: true,
                  },
                },
              }
            : {
                definition,
                data: { assetId: id, objects: [{ id: "model", assetId: id }] },
              },
        select: true,
      };
    }
    if (a.kind === "image" && a.format === "svg" && a.svg?.paths?.length) {
      return {
        type: "add_layer",
        layerType: "shape",
        name: String(a.name || "SVG").replace(/\.svg$/i, ""),
        from: Math.round(at * PM.proj.fps) / PM.proj.fps,
        duration: Math.max(1 / PM.proj.fps, PM.proj.dur - at),
        content: {
          paths: materializeSvgPaths(PM, a.svg),
          svgSourceAsset: id,
          svgWidth: a.svg.width,
          svgHeight: a.svg.height,
          svgWarnings: a.svg.warnings,
        },
        select: true,
      };
    }
    if (a.kind === "lut") return null;
    const type: any =
      a.kind === "audio" ? "audio" : a.kind === "video" ? "video" : "image";
    const content: any =
      type === "audio"
        ? { asset: id, trim: 0, gain: 1, fadeIn: 0, fadeOut: 0 }
        : type === "video"
          ? {
              asset: id,
              trim: 0,
              speed: 1,
              embeddedAudio: a.hasAudio === true,
              w: a.w || PM.proj.w,
              h: a.h || PM.proj.h,
            }
          : { asset: id, w: a.w || PM.proj.w, h: a.h || PM.proj.h };
    return {
      type: "add_layer",
      layerType: type,
      name: a.name,
      from: Math.round(at * PM.proj.fps) / PM.proj.fps,
      /* Keep the whole source even when it extends beyond the current comp. The
       composition still clips playback/export, but extending it later reveals
       the rest instead of permanently discarding the imported media. */
      duration: a.dur || Math.max(1 / PM.proj.fps, PM.proj.dur - at),
      content,
      select: true,
    };
  };
  return (id?: any, at: any = PM.time) => {
    const command = nativeCommandForAsset(id, at);
    const defaults = kernel.services.get(IMPORT_DEFAULTS_SERVICE);
    if (
      !command ||
      !defaults ||
      !["image", "video", "shape"].includes(command.layerType)
    )
      return command;
    // Source dimensions give the detached image/video/SVG bounds; the native
    // bounds query only accepts layers already in the document.
    const layer = PM.mkLayer(command.layerType, { d: command.content });
    const bounds = {
      x0: 0,
      y0: 0,
      x1: command.content.w ?? command.content.svgWidth,
      y1: command.content.h ?? command.content.svgHeight,
    };
    const x = bounds.x0 + (bounds.x1 - bounds.x0) * defaults.anchor.x;
    const y = bounds.y0 + (bounds.y1 - bounds.y0) * defaults.anchor.y;
    if (![x, y].every(Number.isFinite))
      throw new Error("Import defaults received invalid layer bounds.");
    return {
      ...command,
      properties: {
        "anchor.x": x,
        "anchor.y": y,
        // Compensate the new anchor so the imported content stays in place.
        "position.x": layer.p["position.x"].v + x - layer.p["anchor.x"].v,
        "position.y": layer.p["position.y"].v + y - layer.p["anchor.y"].v,
      },
    };
  };
}
