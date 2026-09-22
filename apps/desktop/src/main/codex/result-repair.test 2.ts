import { expect, it, vi } from 'vitest';
import { AgentResultValidationError, repairAgentResult } from './result-repair';

it('only repairs known pre-publication validation errors, never uncertain side effects', async () => {
  const complete = vi.fn().mockRejectedValueOnce(new AgentResultValidationError('missing mod')).mockResolvedValue('published');
  const repair = vi.fn();
  expect(await repairAgentResult(complete, repair)).toBe('published');
  expect(repair).toHaveBeenCalledWith(expect.stringContaining('Do not repeat live project edits'));
  expect(repair).toHaveBeenCalledWith(expect.stringContaining('missing mod'));
  for (const message of ['Extensions changed while the agent was working', 'disk write failed', 'cancelled']) {
    repair.mockClear();
    await expect(repairAgentResult(async () => { throw new Error(message); }, repair)).rejects.toThrow(message);
    expect(repair).not.toHaveBeenCalled();
  }
});

it('stops after two corrections and propagates a failed repair without replaying it', async () => {
  const complete = vi.fn().mockRejectedValue(new AgentResultValidationError('still invalid'));
  const repair = vi.fn();
  await expect(repairAgentResult(complete, repair)).rejects.toThrow('still invalid');
  expect(complete).toHaveBeenCalledTimes(3);
  expect(repair).toHaveBeenCalledTimes(2);
  complete.mockClear();
  repair.mockRejectedValue(new Error('provider disconnected'));
  await expect(repairAgentResult(complete, repair)).rejects.toThrow('provider disconnected');
  expect(complete).toHaveBeenCalledOnce();
});
