/** Synthetic fixtures only. No user project or operational data. */
import { HostClient, json, type HostRequest } from "../../src";
import { ExportModel, type ExportSnapshot } from "./model";
import { formats } from "./spec";
export function initial(): ExportSnapshot {
  return {
    version: 1,
    sequence: 0,
    project: {
      id: "fixture",
      name: "Launch Film",
      width: 1920,
      height: 1080,
      hasAudio: true,
      captionLayers: 1,
      hasWorkArea: true,
      summary: "300 frames · 10 s · 1920×1080",
    },
    options: {
      format: "mp4",
      scale: 1,
      width: 1920,
      height: 1080,
      fps: 30,
      range: "work",
      quality: "high",
      mblur: true,
      audio: true,
      alpha: false,
      captionsBurn: true,
      captionsSidecar: "none",
      future: { keep: true },
    },
    bindings: Object.fromEntries(
      [
        "format",
        "scale",
        "width",
        "height",
        "fps",
        "range",
        "quality",
        "mblur",
        "audio",
        "alpha",
        "captionsBurn",
        "captionsSidecar",
        "preset",
        "presetName",
      ].map((k) => [k, `fixture:export:${k}`]),
    ),
    capabilities: Object.fromEntries(
      formats.map((f) => [f.v, { available: true }]),
    ) as ExportSnapshot["capabilities"],
    presets: [],
    run: { phase: "idle" },
  };
}
export function fixture(seed = initial()) {
  let state = structuredClone(seed);
  const requests: HostRequest[] = [];
  let sequence = 0;
  let destinationCancelled = false;
  let reject: string | undefined;
  let delay: (() => Promise<void>) | undefined;
  const host = new HostClient(
    { realm: "builtin.export", generation: 1, project: "fixture" },
    async (request) => {
      requests.push(request);
      const op = request.operation;
      if (op.type === "command") {
        await delay?.();
        if (reject)
          return {
            ...request,
            revision: request.expectedRevision,
            accepted: false,
            error: reject,
          };
        const args = op.args as Record<string, any>;
        state.sequence++;
        switch (op.command) {
          case "export.plan":
            state.options = structuredClone(args.options);
            // Test planner has only fixture semantics; production planner is Rust.
            state.project!.summary =
              state.options.format === "json"
                ? "Launch Film.pmv"
                : state.options.format === "still"
                  ? "1920×1080 · frame at 00:00:00:00"
                  : "300 frames · 10 s · 1920×1080";
            break;
          case "export.begin":
            state.options = args.options;
            state.run = { phase: destinationCancelled ? "idle" : "preparing" };
            break;
          case "export.cancel":
            state.run = { phase: "cancelled" };
            break;
          case "export.reset":
          case "export.closeFailure":
            state.run = { phase: "idle" };
            break;
          case "export.retry":
            state.run = { phase: "preparing" };
            break;
          case "export.preset.save":
            state.presets.push({
              id: `fixture-preset-${state.presets.length + 1}`,
              name: args.name,
              options: args.options,
            });
            break;
        }
      }
      return {
        ...request,
        revision: request.expectedRevision,
        accepted: true,
        value: json(state),
      };
    },
  );
  const model = new ExportModel(host, state);
  return {
    model,
    host,
    requests,
    set cancelled(value: boolean) {
      destinationCancelled = value;
    },
    set rejection(value: string | undefined) {
      reject = value;
    },
    set delay(value: (() => Promise<void>) | undefined) {
      delay = value;
    },
    async publish(next: ExportSnapshot) {
      const nextSequence = state.sequence + 1;
      state = structuredClone(next);
      state.sequence = nextSequence;
      return host.publish({
        ...host.identity,
        version: 1,
        subscription: "export.fixture",
        revision: 0,
        sequence: ++sequence,
        value: json(state),
      });
    },
  };
}
