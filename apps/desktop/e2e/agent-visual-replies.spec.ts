import { readFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { createServer } from 'node:http';
import { test, expect, launchApp } from './helpers/app';
import { IPC } from '../src/shared/ipc';

const fixtures = (name: string) => JSON.parse(readFileSync(path.join(__dirname, 'fixtures/visual-replies', name + '.json'), 'utf8'));

// A deterministic provider fixture enters the real run lifecycle. Tool deliveries
// use main->preload->harness IPC and the production thread store and Svelte view.
// No model account, paid call or live profile is used.
async function startRun(session: any) {
  await session.openEditor(); await session.openAgent();
  await session.page.evaluate(() => (window as any).PM.theme.apply('dark'));
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async (_p: unknown, _s: unknown, _i: unknown, options: any) => {
      options.onStart('visual-walkthrough-run');
      (window as any).__visualProviderStarted = true;
      return new Promise((resolve, reject) => {
        (window as any).__finishVisualProvider=(summary:string)=>resolve({text:JSON.stringify({summary,commands:[],artifacts:[],externalActions:[],notes:[]})});
        options.signal?.addEventListener('abort', () => reject(new DOMException('Stopped','AbortError')), {once:true});
      });
    };
  });
  await session.page.getByRole('textbox', {name:'Message Powermove agent',exact:true}).fill('Show a timing chart, storyboard and palette as interactive visual replies.');
  await session.page.getByRole('button', {name:'Send message',exact:true}).click();
  await session.openAgent();
  await session.page.waitForFunction(() => (window as any).__visualProviderStarted);
}
async function publish(session: any, input: Record<string, unknown>) {
  const result = await session.app.evaluate(async ({ipcMain, BrowserWindow}: any, {input, channel, replyChannel}: any) => {
    const owner = BrowserWindow.getAllWindows().find((w: any) => w.webContents.getURL().includes('powermove'))!.webContents;
    const request = {runId:'visual-walkthrough-run',callId:`visual-call-${Date.now()}`,tool:'publish_visual_reply',arguments:input,baseRevision:0};
    return await new Promise<any>((resolve, reject) => {
      const listener = (event: any, result: any) => {
        if(event.sender !== owner || result.callId !== request.callId) return;
        clearTimeout(timer); ipcMain.removeListener(replyChannel,listener); resolve(result);
      };
      const timer = setTimeout(() => {ipcMain.removeListener(replyChannel,listener); reject(new Error('Visual tool response timeout'));},8000);
      ipcMain.on(replyChannel,listener); owner.send(channel,request);
    });
  },{input,channel:IPC.agentToolRequest,replyChannel:IPC.agentToolResponse});
  if (result.ok) {
    const { id } = JSON.parse(result.content[0].text);
    const visual = session.page.locator(`[data-visual-reply="${id}"]`);
    await expect(visual).toBeAttached();
    await expect(visual.getByRole('status')).toHaveCount(0);
    // The initial ResizeObserver report and chat auto-scroll land after ready.
    // Wait for geometry to settle before clicking inside the opaque frame.
    await visual.evaluate(async () => {
      for (let i = 0; i < 3; i++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    });
  }
  return result;
}
async function screenshot(session: any, name: string) {
  const directory = process.env.POWERMOVE_VISUAL_EVIDENCE_DIR;
  if(directory) { mkdirSync(directory,{recursive:true}); await session.page.screenshot({path:path.join(directory,name+'.png')}); }
}

test('visual replies publish, interact, expand, replay and reopen in their durable thread', async ({ session }) => {
  await startRun(session);
  expect((await publish(session,fixtures('easing'))).ok).toBe(true);
  const chart = session.page.frameLocator('iframe[title="Timing & easing"]');
  await expect(chart.getByRole('heading',{name:'Timing & easing'})).toBeVisible();
  await chart.getByLabel('Duration').evaluate((input: HTMLInputElement) => { input.value = '1200'; input.dispatchEvent(new Event('input', {bubbles:true})); });
  await expect(chart.locator('output')).toHaveText('1200 ms');
  await screenshot(session,'01-timing-dark');
  await session.page.getByRole('button',{name:'Expand Timing & easing',exact:true}).click();
  await expect(session.page.locator('dialog[open]')).toBeVisible();
  await chart.getByRole('button',{name:'Play motion'}).click();
  await expect(chart.locator('output')).toHaveText('1200 ms');
  await screenshot(session,'02-expanded');
  await session.page.getByRole('button',{name:'Zoom in Timing & easing',exact:true}).click();
  await expect(session.page.getByRole('button',{name:'Reset zoom Timing & easing',exact:true})).toHaveText('125%');
  await session.page.getByRole('button',{name:'Replay Timing & easing',exact:true}).click();
  await expect(chart.locator('output')).toHaveText('800 ms');
  await session.page.evaluate(() => (window as any).PM.theme.apply('light'));
  await expect.poll(() => chart.locator('html').evaluate(el => getComputedStyle(el).colorScheme)).toBe('light');
  await screenshot(session,'03-light-theme');
  await session.page.getByRole('button',{name:'Close visualization',exact:true}).click();
  await expect(session.page.getByRole('button',{name:'Zoom in Timing & easing',exact:true})).toHaveCount(0);
  expect((await publish(session,fixtures('storyboard'))).ok).toBe(true);
  const storyboard = session.page.frameLocator('iframe[title="Launch storyboard"]');
  await session.page.getByRole('button',{name:'Expand Launch storyboard',exact:true}).click();
  await storyboard.getByRole('button',{name:'Next shot'}).click();
  await expect(storyboard.locator('output')).toContainText('Reveal');
  await session.page.getByRole('button',{name:'Close visualization',exact:true}).click();
  expect((await publish(session,fixtures('palette'))).ok).toBe(true);
  const palette = session.page.frameLocator('iframe[title="Dusk palette"]');
  await session.page.getByRole('button',{name:'Expand Dusk palette',exact:true}).click();
  await palette.getByRole('button',{name:'Pink',exact:true}).click();
  await expect(palette.locator('output')).toHaveText('Pink · #EC4899');
  await session.page.getByRole('button',{name:'Close visualization',exact:true}).click();
  await screenshot(session,'04-storyboard-palette');
  const failure = await publish(session,{title:'Blocked image',html:'<img src="file:///etc/passwd">'});
  expect(failure.ok).toBe(false); expect(failure.error).toContain('resources');
  expect((await publish(session,{title:'Broken interaction',html:'<h1>Broken interaction</h1><script>throw new Error("fixture failure")</script>'})).ok).toBe(true);
  await expect(session.page.getByRole('alert').filter({hasText:'script error'})).toBeVisible();
  await session.page.locator('[data-visual-reply]').last().scrollIntoViewIfNeeded();
  await expect(session.page.locator('.visual-notice .error-notice')).toBeVisible();
  await screenshot(session,'05-script-error');
  await session.page.evaluate(() => {const PM=(window as any).PM;PM.AgentUI.stop();PM.AgentUI.flushThreads();});
  await session.relaunch(); await session.openEditor(); await session.openAgent();
  await expect(session.page.frameLocator('iframe[title="Timing & easing"]').getByRole('heading')).toHaveText('Timing & easing');
  await expect(session.page.frameLocator('iframe[title="Dusk palette"]').locator('output')).toHaveText('Indigo · #6366F1');
  await screenshot(session,'06-reopened');
});

test('generated scripts cannot access host privileges, send tool messages or navigate', async ({ session }) => {
  await startRun(session);
  const html = `<h1>Security probe</h1><output></output><button id="nav">Try navigation</button><script>
    const results={node:typeof require,preload:typeof window.powermove,parentBlocked:false};
    try{parent.document.body; }catch{results.parentBlocked=true;}
    parent.postMessage({channel:'powermove-visual',type:'invoke',tool:'run_outside_sandbox',token:'forged'},'*');
    document.querySelector('output').textContent=JSON.stringify(results);
    document.querySelector('button').onclick=()=>{window.name='changed';location.href='app://powermove/index.html';};
  </script>`;
  expect((await publish(session,{title:'Security probe',html})).ok).toBe(true);
  const frame = session.page.frameLocator('iframe[title="Security probe"]');
  await expect(frame.locator('output')).toContainText('"node":"undefined"');
  await expect(frame.locator('output')).toContainText('"preload":"undefined"');
  await expect(frame.locator('output')).toContainText('"parentBlocked":true');
  await frame.getByRole('button',{name:'Try navigation'}).click();
  await expect(frame.getByRole('heading',{name:'Security probe'})).toBeVisible();
  expect(session.page.url()).toContain('app://powermove');
  expect(session.app.windows()).toHaveLength(1);
});

test('CSP blocks resource requests and external navigation from generated scripts', async ({ session }) => {
  let requests = 0;
  const server = createServer((_request, response) => { requests++; response.end('probe'); });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const address = server.address() as { port: number };
  try {
    await startRun(session);
    expect((await publish(session, { title: 'Network isolation', html: `<h1>Network isolation</h1><output>Waiting</output><button>Try external navigation</button><script>
      const url='http://127.0.0.1:${address.port}/probe';
      fetch(url).then(()=>document.querySelector('output').textContent='Unexpected access').catch(()=>document.querySelector('output').textContent='Network blocked');
      const image=new Image();image.src=url;document.body.append(image);
      document.querySelector('button').onclick=()=>{window.name='changed';window.open(url);location.href=url;};
    </script>` })).ok).toBe(true);
    const frame = session.page.frameLocator('iframe[title="Network isolation"]');
    await expect(frame.locator('output')).toHaveText('Network blocked');
    await frame.getByRole('button', { name: 'Try external navigation' }).click();
    await session.page.waitForTimeout(250);
    expect(requests).toBe(0);
    expect(session.app.windows()).toHaveLength(1);
    expect(session.page.url()).toContain('app://powermove');
    // Chromium may substitute its blocked-navigation page before Electron's
    // navigation event. It must never contact the target; Replay restores it.
    await session.page.getByRole('button', { name: 'Replay Network isolation', exact: true }).click();
    await expect(frame.getByRole('heading')).toHaveText('Network isolation');
  } finally { await new Promise<void>(resolve => server.close(() => resolve())); }
});

test('visual replies resize within bounds and malformed publication fails clearly', async ({ session }) => {
  await startRun(session);
  expect((await publish(session, { title: 'Resize probe', height: 700,
    html: '<h1>Resize probe</h1><button>Grow content</button><div id="sized" style="height:80px"></div><script>document.querySelector("button").onclick=()=>document.querySelector("#sized").style.height="500px";</script>',
  })).ok).toBe(true);
  const root = session.page.locator('[data-visual-reply]');
  const viewport = root.locator('.visual-viewport');
  await expect.poll(async () => (await viewport.boundingBox())!.height).toBeLessThan(350);
  await session.page.frameLocator('iframe[title="Resize probe"]').getByRole('button', { name: 'Grow content' }).click();
  await expect.poll(async () => (await root.locator('iframe').boundingBox())!.height).toBeGreaterThan(500);
  expect((await viewport.boundingBox())!.height).toBeLessThanOrEqual(180);
  await session.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.setSize(1000, 760));
  await expect.poll(() => root.locator('.visual-attachment-header').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
  for (const input of [{title:'',html:'x'}, {title:'Invalid height',html:'x',height:100}, {title:'Oversized',html:'x'.repeat(393217)}]) {
    const result = await publish(session, input);
    expect(result.ok).toBe(false); expect(result.error).toContain('Visual reply requires');
  }
  expect(await root.count()).toBe(1);
});

test('the real provider run publishes through its authenticated MCP session', async () => {
  const session = await launchApp({ normalWindow: true, env: {
    CODEX_BINARY: path.join(__dirname, 'fixtures/visual-replies/fake-visual-codex.mjs'),
  } });
  try {
    await session.openEditor(); await session.openAgent();
    await session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Show an interactive timing chart.');
    await session.page.getByRole('button', { name: 'Send message', exact: true }).click();
    await session.openAgent();
    const chart = session.page.frameLocator('iframe[title="Timing & easing"]');
    await expect(chart.getByRole('heading', { name: 'Timing & easing' })).toBeAttached();
    await expect(session.page.locator('.agent-visual-attachments')).toHaveCount(1);
    await expect(session.page.locator('[data-visual-reply]')).toHaveCount(3);
    expect((await session.page.locator('.agent-visual-attachments').boundingBox())!.height).toBeLessThan(250);
    await expect(session.page.locator('dialog[open]')).toHaveCount(0);
    await expect(session.page.getByText('Published an interactive timing chart through the native agent tool.', { exact: true })).toBeVisible();
    await expect(chart.locator('output')).toHaveText('800 ms');
    await screenshot(session, '07-actual-provider-publication');
    const preferences = await session.app.evaluate(({ BrowserWindow }) => {
      const p = BrowserWindow.getAllWindows()[0]!.webContents.getLastWebPreferences();
      return { sandbox: p.sandbox, contextIsolation: p.contextIsolation, nodeIntegration: p.nodeIntegration, nodeIntegrationInSubFrames: p.nodeIntegrationInSubFrames };
    });
    expect(preferences).toEqual({ sandbox: true, contextIsolation: true, nodeIntegration: false, nodeIntegrationInSubFrames: false });
  } finally { await session.close(); }
});


test('attachments preserve interaction while the response adds pages and remain compact at narrow widths', async ({session}) => {
  await startRun(session);
  expect((await publish(session,fixtures('easing'))).ok).toBe(true);
  const chart=session.page.frameLocator('iframe[title="Timing & easing"]');
  await chart.getByLabel('Duration').evaluate((input:HTMLInputElement)=>{input.value='1200';input.dispatchEvent(new Event('input',{bubbles:true}));});
  expect((await publish(session,fixtures('palette'))).ok).toBe(true);
  await expect(chart.locator('output')).toHaveText('1200 ms');
  const rail=session.page.locator('.agent-visual-attachments');
  await expect(rail).toHaveCount(1);
  await rail.evaluate(el=>{(el as HTMLElement).style.width='240px';});
  expect((await rail.boundingBox())!.height).toBeLessThan(250);
  expect(await rail.evaluate(el=>el.scrollWidth>el.clientWidth)).toBe(true);
  await session.page.getByRole('button',{name:'Expand Timing & easing',exact:true}).click();
  await expect(chart.locator('output')).toHaveText('1200 ms');
  await session.page.keyboard.press('Escape');
  await expect(session.page.locator('dialog[open]')).toHaveCount(0);
  await expect(chart.locator('output')).toHaveText('1200 ms');
  await expect(session.page.getByRole('button',{name:'Expand Timing & easing',exact:true})).toBeFocused();
  const summary=Array.from({length:12},(_,i)=>`Reasoning paragraph ${i+1}: this explanation remains complete beside the attached visuals.`).join('\n\n');
  await session.page.evaluate(summary=>(window as any).__finishVisualProvider(summary),summary);
  await expect(session.page.getByText('Reasoning paragraph 12: this explanation remains complete beside the attached visuals.',{exact:true})).toBeVisible();
  await expect(session.page.locator('.agent-reply .agent-md-p')).toHaveCount(12);
  await expect(rail).toHaveCount(1);
  await screenshot(session,'08-compact-attachments');
});

test('visual attachment chrome matches native modal tokens in light and dark and supports keyboard controls', async ({session}) => {
  await startRun(session);
  expect((await publish(session,fixtures('easing'))).ok).toBe(true);
  const page=session.page;
  const chart=page.frameLocator('iframe[title="Timing & easing"]');
  const root=page.locator('[data-visual-reply]');
  await expect(root.locator('.agent-attachment-type')).toHaveText('HTML');
  await expect(root.locator('[data-icon="aspectRatio"]')).toHaveAttribute('data-icon-set','phosphor');
  for (const mode of ['light','dark']) {
    await page.evaluate(mode=>(window as any).PM.theme.apply(mode),mode);
    await page.evaluate(()=>{(window as any).__nativeVisualReference=(window as any).PM.modal({title:'Appearance reference',body:'Native control reference',actions:[{label:'Done'}]});});
    const native=page.getByRole('dialog',{name:'Appearance reference',exact:true});
    await expect(native).toBeVisible();
    const reference=await native.evaluate(el=>{
      const surface=getComputedStyle(el),title=getComputedStyle(el.querySelector('h3')!),button=getComputedStyle(el.querySelector('button')!);
      return {background:surface.backgroundColor,radius:surface.borderRadius,shadow:surface.boxShadow,titleSize:title.fontSize,controlSize:button.fontSize,controlRadius:button.borderRadius};
    });
    await page.evaluate(()=>(window as any).__nativeVisualReference.close());
    await expect(native).toHaveCount(0);
    await page.getByRole('button',{name:'Expand Timing & easing',exact:true}).click();
    const viewer=page.getByRole('dialog',{name:'Expanded Timing & easing',exact:true});
    const actual=await viewer.evaluate(el=>{
      const surface=getComputedStyle(el),title=getComputedStyle(el.querySelector('.visual-title')!),button=getComputedStyle(el.querySelector('.visual-zoom')!);
      return {background:surface.backgroundColor,radius:surface.borderRadius,shadow:surface.boxShadow,titleSize:title.fontSize,controlSize:button.fontSize,controlRadius:button.borderRadius};
    });
    expect(actual).toEqual(reference);
    await expect(chart.locator('output')).toHaveText('800 ms');
    await expect(viewer.locator('[data-icon="x"]')).toHaveAttribute('data-icon-set','phosphor');
    const close=page.getByRole('button',{name:'Close visualization',exact:true});
    await close.focus();await page.keyboard.press('Shift+Tab');
    const replay=page.getByRole('button',{name:'Replay Timing & easing',exact:true});
    await expect(replay).toBeFocused();
    expect(await replay.evaluate(el=>getComputedStyle(el).outlineWidth)).toBe('2px');
    for(let i=0;i<4;i++) await page.getByRole('button',{name:'Zoom in Timing & easing',exact:true}).click();
    const zoomIn=page.getByRole('button',{name:'Zoom in Timing & easing',exact:true});
    await expect(zoomIn).toBeDisabled();
    expect(await zoomIn.evaluate(el=>getComputedStyle(el).opacity)).toBe('0.5');
    await page.getByRole('button',{name:'Reset zoom Timing & easing',exact:true}).click();
    await screenshot(session,`09-native-${mode}`);
    await page.keyboard.press('Escape');
    await expect(viewer).not.toBeVisible();
    await expect(page.getByRole('button',{name:'Expand Timing & easing',exact:true})).toBeFocused();
  }
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.getByRole('button',{name:'Expand Timing & easing',exact:true}).press('Enter');
  expect(await page.locator('dialog[open]').evaluate(el=>getComputedStyle(el).animationName)).toBe('none');
  await page.keyboard.press('Escape');
  await expect(page.locator('dialog[open]')).toHaveCount(0);
  await root.locator('.visual-attachment-header').evaluate(el=>(el as HTMLElement).style.width='200px');
  expect(await root.locator('.visual-attachment-header').evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
});
