import { expect, it, vi } from 'vitest';
import { agentFramesRequest, installAgentFramesIpc } from './agent-frames';
import { ENGINE_IPC } from '../../shared/engine-ipc';

it('authenticates batches before parsing or opening sessions', async () => {
  let handler: any;
  const ipc = { handle: vi.fn((_channel, value) => { handler = value; }), removeHandler: vi.fn() };
  const jobs = { renderFrames: vi.fn(async () => ({ status: 'rendered', images: [] })) };
  const trusted = vi.fn(() => false);
  const remove = installAgentFramesIpc(ipc as never, jobs as never, trusted);
  const request = { project: '{"id":"p"}', revision: 3, times: [0, 1], width: 720 };
  await expect(handler({ sender: { id: 8 } }, request)).rejects.toThrow('Unauthorized');
  expect(jobs.renderFrames).not.toHaveBeenCalled();
  trusted.mockReturnValue(true);
  await handler({ sender: { id: 8 } }, request);
  expect(jobs.renderFrames).toHaveBeenCalledWith(8, 'observation-8', { project: { id: 'p' }, revision: 3, times: [0, 1], width: 720 });
  remove(); expect(ipc.removeHandler).toHaveBeenCalledWith(ENGINE_IPC.agentFrames);
  for (const patch of [{ times: [] }, { times: [NaN] }, { times: [0, 1, 2, 3, 4, 5] }, { revision: -1 }, { width: Infinity }]) expect(agentFramesRequest({ ...request, ...patch })).toBe(false);
});
