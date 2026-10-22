import { validateFrameRate } from '../../../../shared/frame-rate';
import { z } from 'zod';
import type { PMRegistry } from '../registry';

const empty = z.object({}).strict();
const creation = z.object({ name: z.string().trim().min(1).max(120), width: z.number().int().min(2).max(8192).default(1920), height: z.number().int().min(2).max(8192).default(1080), fps: z.number().finite().min(1).max(120).default(30), duration: z.number().finite().min(0.1).max(36000).default(10) }).strict();
const video = z.object({ name: z.string().min(1).max(120).optional(), format: z.enum(['mp4', 'prores']).default('mp4'), range: z.enum(['all', 'work']).default('all'), audio: z.boolean().default(true), motionBlur: z.boolean().default(true), alpha: z.boolean().default(false) }).strict();

/** Authenticated main-only operations. They use the normal project and exporter APIs. */
export async function mcpProjectTool(PM: PMRegistry, tool: string, args: Record<string, unknown>): Promise<unknown> {
  if (tool === '__mcp_context') {
    empty.parse(args);
    const project = PM.proj, home = PM.isHomeProject?.() === true;
    await PM.HostMedia?.flushBindings(project);
    // The window's compatibility producers (a `raster` override): an engine
    // export of this project asks this window for those layers (kernel/raster-compat.ts).
    const compat = Array.isArray((PM as any).EngineRender?.producing) ? [...(PM as any).EngineRender.producing] : [];
    return { projectId: project.id, projectName: project.name, projectJSON: JSON.stringify(project), home, compat };
  }
  if (tool === '__mcp_list_projects') {
    empty.parse(args); return { currentProjectId: PM.isHomeProject?.() ? null : PM.proj.id, projects: PM.Projects.list() };
  }
  if (tool === '__mcp_create_project') {
    const input = creation.parse(args);
    // The single frame-rate validator: rates that divide the engine's tick.
    const rate = validateFrameRate(input.fps, 'new');
    if (!rate.ok) throw new Error(rate.message);
    PM.newBlankProject({ name: input.name, w: input.width, h: input.height, fps: rate.fps, fpsRational: rate.fpsRational, dur: input.duration });
    PM.Projects.put(PM.proj);
    return { projectId: PM.proj.id, name: PM.proj.name, width: PM.proj.w, height: PM.proj.h, fps: PM.proj.fps, duration: PM.proj.dur };
  }
  if (tool === '__mcp_open_project') {
    const { projectId } = z.object({ projectId: z.string().regex(/^[A-Za-z0-9_-]{1,120}$/) }).strict().parse(args);
    if (!await PM.openProjectHere(projectId)) throw new Error('This project could not be opened. It may be open in another window.');
    return { projectId: PM.proj.id, name: PM.proj.name };
  }
  if (tool === '__mcp_save_project' || tool === '__mcp_export_video') {
    if (PM.Export.busy) throw new Error('An export is already running.');
    let options;
    if (tool === '__mcp_save_project') { empty.parse(args); options = { format: 'json', reportPath: true }; }
    else {
      const input = video.parse(args);
      if (input.alpha && input.format !== 'prores') throw new Error('Transparent video requires ProRes.');
      options = { ...input, mblur: input.motionBlur, fps: PM.proj.fps, scale: 1, reportPath: true };
    }
    const result = await PM.Export.run(options);
    if (!result || result.error || result.cancelled || !result.path) throw new Error(result?.error || (result?.cancelled ? 'Export cancelled.' : 'Export produced no saved file.'));
    PM.Projects.put(PM.proj);
    return { path: result.path, projectId: PM.proj.id, ...(tool === '__mcp_export_video' ? { width: PM.proj.w, height: PM.proj.h, fps: PM.proj.fps } : {}) };
  }
  throw new Error('Unknown MCP project operation.');
}
