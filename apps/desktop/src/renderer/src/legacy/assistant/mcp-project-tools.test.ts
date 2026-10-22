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

it('reports missing clips after an unattended legacy agent export', async () => {
  const PM: any = {
    proj: { id: 'P1', compId: 'root', name: 'test', w: 320, h: 180, fps: 24,
      layers: [{ type: 'video', d: { asset: 'gone' } }],
      assets: { gone: { name: 'Missing.mov' } } },
    projectAssetsReady: Promise.resolve(),
    assets: { get: () => null },
    Export: { busy: false, run: vi.fn(async () => ({ path: '/exports/test.mp4' })) },
    Projects: { put: vi.fn() }
  };
  await expect(mcpProjectTool(PM, '__mcp_export_video', {})).resolves.toMatchObject({
    path: '/exports/test.mp4',
    missingMedia: [{ asset: 'gone', name: 'Missing.mov' }]
  });
});
