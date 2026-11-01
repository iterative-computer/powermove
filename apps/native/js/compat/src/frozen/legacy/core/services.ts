// Extracted from v1.2.0 renderer/src/legacy/core/services.ts; local type/duplicate-helper paths adapted.
import type { Data, ServicesAPI as ServicesRegistry } from "../../../types";
type InspectorService = Data;
type ShaderHooks = Data;
type TimelineService = Data;
type ToolService = Data;
type ViewerService = Data;
import type { Data as PMRegistry } from "../../../types";

export function timelineService(PM: PMRegistry): TimelineService | null {
  return (
    (PM.Kernel?.services as ServicesRegistry | undefined)?.get<TimelineService>(
      "timeline",
    ) ?? null
  );
}

export function viewerService(PM: PMRegistry): ViewerService | null {
  return (
    (PM.Kernel?.services as ServicesRegistry | undefined)?.get<ViewerService>(
      "viewer",
    ) ?? null
  );
}

export function inspectorService(PM: PMRegistry): InspectorService | null {
  return (
    (
      PM.Kernel?.services as ServicesRegistry | undefined
    )?.get<InspectorService>("inspector") ?? null
  );
}

export function toolService(PM: PMRegistry): ToolService | null {
  return (
    (PM.Kernel?.services as ServicesRegistry | undefined)?.get<ToolService>(
      "tool",
    ) ?? null
  );
}

export function shaderHooks(PM: PMRegistry): ShaderHooks | null {
  return (
    (PM.Kernel?.services as ServicesRegistry | undefined)?.get<ShaderHooks>(
      "shaderHooks",
    ) ?? null
  );
}
