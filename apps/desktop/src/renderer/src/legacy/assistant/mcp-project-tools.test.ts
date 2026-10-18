import { expect, it, vi } from 'vitest';
import { mcpProjectTool } from './mcp-project-tools';

it('acknowledges media bindings before handing an agent export its project snapshot', async () => {
  let acknowledge!: () => void;
  const PM: any = { proj: { id: 'P1', name: 'test' }, HostMedia: {
    flushBindings: vi.fn(() => new Promise<void>(resolve => { acknowledge = resolve; }))
  } };
  const original = PM.proj;
  let answered = false;
  const context = mcpProjectTool(PM, '__mcp_context', {}).then(value => { answered = true; return value; });
  expect(PM.HostMedia.flushBindings).toHaveBeenCalledWith(PM.proj);
  await Promise.resolve();
  expect(answered).toBe(false);
  PM.proj = { id: 'P2', name: 'other' };
  acknowledge();
  expect(await context).toMatchObject({ projectId: 'P1', projectJSON: JSON.stringify(original) });
});
