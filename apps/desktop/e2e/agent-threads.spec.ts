import { test, expect } from './helpers/app';

test('background run badge stays compact when creating and switching threads', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = () => new Promise(resolve => {
      (window as any).__finishBackgroundRun = () => resolve({
        text: JSON.stringify({ summary: 'Background work finished', commands: [], artifacts: [], externalActions: [], notes: [] })
      });
    });
  });
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  const picker = page.getByRole('button', { name: 'Switch thread', exact: true });
  const badge = picker.locator('.thread-running');
  const first = await page.evaluate(() => (window as any).PM.AgentUI.state.threadId);
  await composer.fill('Keep working while I create another thread');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof (window as any).__finishBackgroundRun)).toBe('function');
  await page.getByRole('button', { name: 'New thread', exact: true }).click();
  await expect(composer).toHaveText('');
  await expect(badge).toHaveText('1');

  // Exercise actual Chromium layout: DOM-only tests cannot detect a stretched dot.
  for (const width of [240, 400]) {
    await page.locator('.thread-bar').evaluate((bar, width) => {
      (bar as HTMLElement).style.width = `${width}px`;
    }, width);
    const dot = await badge.locator('.thread-dot').boundingBox();
    expect(dot?.width).toBe(6);
    expect(dot?.height).toBe(6);
    expect((await badge.boundingBox())!.width).toBeLessThan(40);
  }
  await page.locator('.thread-bar').evaluate(bar => (bar as HTMLElement).style.removeProperty('width'));

  await picker.click();
  const workingThread = page.getByRole('option').filter({ hasText: 'Working…' });
  await expect.poll(async () => (await workingThread.locator('.thread-dot').boundingBox())?.width).toBe(6);
  await workingThread.click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.threadId)).toBe(first);
  await expect(badge).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Stop current run', exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'New thread', exact: true }).click();
  await expect(badge).toHaveText('1');
  await page.evaluate(() => (window as any).__finishBackgroundRun());
  await expect(badge).toHaveCount(0);
  await expect(composer).toHaveText('');
  await expect(page.getByRole('log')).not.toContainText('Background work finished');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Command+A selects the full agent composer draft', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Select this entire composer draft');
  await composer.press('Meta+A');
  await expect.poll(() => composer.evaluate(element => {
    const selection = window.getSelection();
    return { text: selection?.toString(), inside: !!selection?.anchorNode && element.contains(selection.anchorNode) };
  })).toEqual({ text: 'Select this entire composer draft', inside: true });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('selected agent text copies and pastes normally without copying layers', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  const reply = 'Selection-ready agent reply';
  await session.app.evaluate(({ clipboard }) => clipboard.writeText(''));
  await page.evaluate((text) => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async () => ({
      text: JSON.stringify({ summary: text, commands: [], artifacts: [], externalActions: [], notes: [] })
    });
  }, reply);

  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Give me selectable text');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const answer = page.locator('.agent-msg.assistant p').filter({ hasText: reply });
  await expect(answer).toBeVisible();
  await answer.evaluate((element) => {
    (document.activeElement as HTMLElement | null)?.blur?.();
    const selection = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(element);
    selection?.removeAllRanges();
    selection?.addRange(range);
  });

  const selectedLayers = await page.evaluate(() => [...(window as any).PM.sel.layers]);
  await page.keyboard.press('Meta+C');
  await expect.poll(() => session.app.evaluate(({ clipboard }) => clipboard.readText())).toBe(reply);
  expect(await page.evaluate(() => [...(window as any).PM.sel.layers])).toEqual(selectedLayers);

  await composer.click();
  await page.keyboard.press('Meta+V');
  await expect(composer).toHaveText(reply);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('steering stays in the active run and preserves the transcript before it', async ({ session }, testInfo) => {
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    (window as any).__steeringProof = { requests: 0, steering: [] as string[] };
    PM.CodexBridge.request = async (_prompt: string, _schema: unknown, _images: unknown[], options: any) => {
      (window as any).__steeringProof.requests += 1;
      options.onTrace({ kind: 'answer', text: 'I have started building the blur.' });
      return await new Promise(() => {});
    };
    PM.CodexBridge.steer = async (prompt: string) => {
      (window as any).__steeringProof.steering.push(prompt);
      return true;
    };
  });

  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Make a progressive blur effect');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Stop current run', exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__steeringProof.requests)).toBe(1);
  await expect(page.locator('.agent-trace-text')).toHaveText('I have started building the blur.');

  await composer.fill('continue');
  await composer.press('Enter');
  await expect.poll(() => page.evaluate(() => (window as any).__steeringProof.steering.length)).toBe(1);
  const userTurns = page.locator('.agent-msg.user');
  await expect(userTurns).toHaveCount(2);
  await expect(userTurns.nth(1)).toHaveClass(/is-steering/);
  await expect(userTurns.nth(1).locator('.agent-bubble')).toHaveText('continue');
  const transcriptOrder = await page.locator('.agent-scroll > :is(.agent-msg, .agent-trace.is-archived)').evaluateAll((items) =>
    items.map((item) => item.textContent?.trim()).filter(Boolean));
  expect(transcriptOrder).toEqual([
    'Make a progressive blur effect',
    'I have started building the blur.',
    'continue'
  ]);
  await expect(page.locator('.agent-working-timer')).toHaveCount(1);
  expect(await page.locator('.agent-working-timer').evaluate((timer) => ({
    previous: timer.previousElementSibling?.textContent?.trim(),
    next: timer.nextElementSibling?.textContent?.trim(),
  }))).toEqual({
    previous: 'Make a progressive blur effect',
    next: 'I have started building the blur.',
  });
  const gap = await page.evaluate(() => {
    const trace = document.querySelector('.agent-trace.is-archived')?.getBoundingClientRect();
    const steer = document.querySelector('.agent-msg.user.is-steering')?.getBoundingClientRect();
    return trace && steer ? steer.top - trace.bottom : Number.POSITIVE_INFINITY;
  });
  expect(gap).toBeLessThanOrEqual(10);
  expect(await page.evaluate(() => (window as any).__steeringProof.requests)).toBe(1);
  await testInfo.attach('hidden-agent-steering', { body: await page.screenshot(), contentType: 'image/png' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('hidden renderer switches threads, keeps drafts, and restores history after relaunch', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  const picker = page.getByRole('button', { name: 'Switch thread', exact: true });
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  const activeThread = () => page.evaluate(() => (window as any).PM.AgentUI.state.threadId as string);
  const pick = async (id: string) => {
    await picker.click();
    await page.locator(`.thread-row`).nth(await page.evaluate((threadId) => (window as any).PM.AgentUI.state.threads
      .findIndex((thread: any) => thread.id === threadId), id)).click();
  };
  await expect(picker).toBeVisible();
  const first = await activeThread();
  await composer.fill('First thread draft');
  await page.getByRole('button', { name: 'New thread', exact: true }).click();
  const second = await activeThread();
  expect(second).not.toBe(first);
  await expect(composer).toHaveText('');
  await composer.fill('Second thread draft');
  await pick(first);
  await expect(composer).toHaveText('First thread draft');
  await pick(second);
  await expect(composer).toHaveText('Second thread draft');
  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.store.get(`agentThreads.${PM.proj.id}`)?.activeId;
  })).toBe(second);
  await session.relaunch();
  await session.openEditor();
  await expect.poll(() => session.page.evaluate(() => {
    const state = (window as any).PM.AgentUI.state;
    return { activeId: state.threadId, threadIds: state.threads.map((thread: any) => thread.id) };
  })).toEqual({ activeId: second, threadIds: [second, first] });
  await session.page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  await expect(session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true })).toHaveText('Second thread draft');
  await session.page.getByRole('button', { name: 'Switch thread', exact: true }).click();
  await session.page.locator('.thread-row').last().click();
  await expect(session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true })).toHaveText('First thread draft');
});

test('background app is never visible or focused, but can draw and accept input', async ({ session }, testInfo) => {
  await session.openEditor();
  const state = () => session.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().map(w => ({
    visible: w.isVisible(), focused: w.isFocused(), devtools: w.webContents.isDevToolsOpened(),
  })));
  expect(await state()).toEqual([{ visible: false, focused: false, devtools: false }]);
  await session.page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  await session.page.getByRole('button', { name: 'New thread', exact: true }).click();
  await session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Background input works');
  await expect(session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true })).toHaveText('Background input works');
  await testInfo.attach('hidden-agent-threads', { body: await session.page.screenshot(), contentType: 'image/png' });
  expect(await state()).toEqual([{ visible: false, focused: false, devtools: false }]);
});

test('thread transcripts and titles survive a hidden relaunch without leaking into another thread', async ({ session }) => {
  await session.openEditor();
  let page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({state:{},times:[],images:[]});
    PM.AgentThreadTitles.generate = async () => JSON.stringify({title:'First conversation request'});
    PM.CodexBridge.request = async () => ({text:JSON.stringify({summary:'A saved test reply',commands:[],artifacts:[],externalActions:[],notes:[]})});
  });
  await page.getByRole('textbox', {name:'Message Powermove agent',exact:true}).fill('First conversation request');
  await page.getByRole('button', {name:'Send message',exact:true}).click();
  await expect(page.getByRole('log')).toContainText('A saved test reply');
  await expect(page.getByRole('button', {name:'Switch thread',exact:true})).toContainText('First conversation request');
  await page.getByRole('button', {name:'New thread',exact:true}).click();
  await expect(page.getByRole('log')).not.toContainText('First conversation request');
  await page.getByRole('button', {name:'Switch thread',exact:true}).click();
  await page.getByRole('option', {name:'First conversation request'}).click();
  await expect(page.getByRole('log')).toContainText('A saved test reply');
  // Deliberately close immediately after typing: the native quit barrier must
  // flush the draft before its debounce timer fires.
  await page.getByRole('textbox', {name:'Message Powermove agent',exact:true}).fill('Last keystroke');
  await session.relaunch(); page = session.page;
  await session.openEditor();
  await page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  await expect(page.getByRole('log')).toContainText('First conversation request');
  await expect(page.getByRole('log')).toContainText('A saved test reply');
  await expect(page.getByRole('textbox', {name:'Message Powermove agent',exact:true})).toHaveText('Last keystroke');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('agent run survives clicking another project tab and resumes on return', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  const { first, second } = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const first = PM.proj.id;
    const project = PM.mkProject({ name: 'Other agent project', dur: 4, w: 640, h: 360, fps: 24, bg: '#09090A' });
    PM.Projects.put(project);
    await PM.Tabs.activate(project.id);
    await PM.Tabs.activate(first);
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = (_prompt: string, _schema: unknown, _images: unknown[], options: any) => new Promise(resolve => {
      (window as any).__tabRun = {
        signal: options.signal,
        finish: () => resolve({ text: JSON.stringify({ summary: 'Original project finished', commands: [], artifacts: [], externalActions: [], notes: [] }) }),
      };
    });
    return { first, second: project.id };
  });
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Keep working while I switch projects');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => Boolean((window as any).__tabRun))).toBe(true);
  await page.locator(`#tabs .project-doc[data-tab-id="${second}"]`).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.id)).toBe(second);
  expect(await page.evaluate(() => (window as any).__tabRun.signal.aborted)).toBe(false);
  await page.evaluate(() => (window as any).__tabRun.finish());
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.backgroundRuns)).toBe(1);
  expect(await page.evaluate(() => (window as any).PM.AgentUI.state.conversation)).toEqual([]);
  await page.locator(`#tabs .project-doc[data-tab-id="${first}"]`).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.phase)).toBe('result');
  expect(await page.evaluate(() => (window as any).PM.AgentUI.state.run.projectId)).toBe(first);
  await expect(page.getByRole('log')).toContainText('Original project finished');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
