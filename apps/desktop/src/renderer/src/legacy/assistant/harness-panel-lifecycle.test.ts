// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('../../kernel/extensions.svelte', () => ({ records: () => [] }));
vi.mock('../../kernel/bridge', () => ({ bridge: () => undefined }));
import { install } from './harness';

afterEach(() => { document.body.innerHTML = ''; vi.restoreAllMocks(); });

function fixture() {
  const layout = { docks: [{ panels: [{ id: 'extension-panel', collapsed: false }] }] };
  const PM: any = { proj: { id: 'test-project', revision: 1, layers: [] }, PANELS: { 'extension-panel': { title: 'Panel' } },
    WS: { current: { layout } }, Layout: { apply: vi.fn() } };
  install(PM);
  let serial = 0;
  const call = (tool: string, args: Record<string, unknown> = {}) => PM.AgentHarness.test.handleLiveAgentTool({
    runId: 'test-run', callId: `test-call-${++serial}`, baseRevision: 1, tool, arguments: args
  });
  return { PM, call };
}

it('cancels a pending panel mount on run completion and allows a later run to inspect panels', async () => {
  const { PM, call } = fixture();
  const opening = call('open_panel', { panelId: 'extension-panel' });
  const outcome = opening.catch((error: Error) => error);
  expect((await call('__finish_run', { commit: true })).ok).toBe(true);
  expect(await outcome).toBeInstanceOf(Error);
  expect(PM.Layout.apply).not.toHaveBeenCalled();
  document.body.innerHTML = '<div id="panel-extension-panel"><button>Current panel</button></div>';
  vi.spyOn(HTMLElement.prototype, 'getClientRects').mockReturnValue([{}] as any);
  const read = await call('get_panel_state', { panelId: 'extension-panel' });
  expect(JSON.parse(read.content[0].text).controls[0].label).toBe('Current panel');
});

it('validates target generations against the current registry without ending the run', async () => {
  const { PM, call } = fixture();
  document.body.innerHTML = '<div id="panel-extension-panel"></div>';
  const first = await call('__panel_bounds', { panelId: 'extension-panel' });
  const { target } = JSON.parse(first.content[0].text);
  PM.PANELS['extension-panel'] = { title: 'Re-enabled panel' };
  await expect(call('__validate_panel_target', { panelId: 'extension-panel', target })).rejects.toThrow('unavailable');
  const next = await call('__panel_bounds', { panelId: 'extension-panel' });
  const current = JSON.parse(next.content[0].text).target;
  expect(current).not.toBe(target);
  expect((await call('__validate_panel_target', { panelId: 'extension-panel', target: current })).ok).toBe(true);
});
