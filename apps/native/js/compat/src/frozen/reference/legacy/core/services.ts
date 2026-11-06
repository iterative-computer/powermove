import type {
  InspectorService,
  ServicesRegistry,
  ShaderHooks,
  TimelineService,
  ToolService,
  ViewerService,
} from '../../kernel/services';
import type { PMRegistry } from '../registry';

export function timelineService(PM: PMRegistry): TimelineService | null {
  return (PM.Kernel?.services as ServicesRegistry | undefined)?.get<TimelineService>('timeline') ?? null;
}

export function viewerService(PM: PMRegistry): ViewerService | null {
  return (PM.Kernel?.services as ServicesRegistry | undefined)?.get<ViewerService>('viewer') ?? null;
}

export function inspectorService(PM: PMRegistry): InspectorService | null {
  return (PM.Kernel?.services as ServicesRegistry | undefined)?.get<InspectorService>('inspector') ?? null;
}

export function toolService(PM: PMRegistry): ToolService | null {
  return (PM.Kernel?.services as ServicesRegistry | undefined)?.get<ToolService>('tool') ?? null;
}

export function shaderHooks(PM: PMRegistry): ShaderHooks | null {
  return (PM.Kernel?.services as ServicesRegistry | undefined)?.get<ShaderHooks>('shaderHooks') ?? null;
}
