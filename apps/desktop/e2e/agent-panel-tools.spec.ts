import { expect, test } from './helpers/app';

test('agent opens and uses an extension panel through its real controls', async ({ session }) => {
  await session.openEditor();
  await session.page.waitForSelector('#panel-agent textarea', { state: 'attached' });
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.PANELS['footage-fixture'] = { id: 'footage-fixture', title: 'Footage fixture', size: 260, build(body: HTMLElement) {
      body.innerHTML = '<label>Search footage<input type="search"></label><button>Search</button><p>Ready</p><button aria-label="Add title">Add title</button>';
      body.querySelector('button')!.onclick = () => { body.querySelector('p')!.textContent = `Results for ${body.querySelector('input')!.value}`; };
      body.querySelectorAll('button')[1]!.onclick = () => PM.Edit.apply({ type: 'add_layer', layerType: 'text', name: 'Panel title', content: { text: 'Cinema' } }, { origin: 'panel', label: 'Add panel title' });
    } };
    const baseRevision = PM.proj.revision || 0;
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({ runId: 'panel-e2e', callId: tool, tool, arguments: args, baseRevision });
    let state = JSON.parse((await call('open_panel', { panelId: 'footage-fixture' })).content[0].text);
    const action = async (label: string, action: string, value?: string) => {
      const ref = state.controls.find((c: any) => c.label === label).ref;
      state = JSON.parse((await call('interact_panel', { panelId: 'footage-fixture', ref, action, value })).content[0].text);
    };
    await action('Search footage', 'fill', 'cinema');
    const filled = state.controls.find((c: any) => c.label === 'Search footage')?.value;
    await action('Search', 'click');
    const text = state.text;
    await action('Add title', 'click');
    const exists = PM.proj.layers.some((l: any) => l.name === 'Panel title');
    const finish = await call('__finish_run', { commit: true });
    PM.hist.undo();
    return { text, filled, exists, finished: finish.ok, changed: finish.changed, undone: !PM.proj.layers.some((l: any) => l.name === 'Panel title') };
  });
  expect(result).toMatchObject({ exists: true, finished: true, changed: true, undone: true });
  expect(result.filled).toBe('cinema');
  expect(result.text).toContain('Results for cinema');
});
