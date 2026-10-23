/** Revision-pinned agent batches shared by renderer observations and MCP. */
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { ENGINE_IPC, type EngineAgentFramesRequest } from '../../shared/engine-ipc';
import type { AgentJobs } from './agent-jobs';

export function agentFramesRequest(value: unknown): value is EngineAgentFramesRequest {
  if (!value || typeof value !== 'object') return false;
  const request = value as EngineAgentFramesRequest;
  return typeof request.project === 'string' && request.project.length <= 256 * 1024 * 1024
    && Number.isSafeInteger(request.revision) && request.revision >= 0
    && Number.isInteger(request.width) && request.width >= 1 && request.width <= 8192
    && Array.isArray(request.times) && request.times.length >= 1 && request.times.length <= 5
    && request.times.every(time => typeof time === 'number' && Number.isFinite(time) && time >= 0);
}

export function installAgentFramesIpc(ipc: Pick<IpcMain, 'handle' | 'removeHandler'>, jobs: AgentJobs, trusted: (event: IpcMainInvokeEvent) => boolean): () => void {
  ipc.handle(ENGINE_IPC.agentFrames, async (event, value: unknown) => {
    if (!trusted(event)) throw new Error('Unauthorized IPC sender');
    if (!agentFramesRequest(value)) throw new Error('agentFrames needs a project, revision, width and one to five finite frame times.');
    return jobs.renderFrames(event.sender.id, `observation-${event.sender.id}`, {
      project: JSON.parse(value.project), times: value.times, width: value.width, revision: value.revision
    });
  });
  return () => ipc.removeHandler(ENGINE_IPC.agentFrames);
}
