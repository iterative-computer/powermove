import { expect, test } from './helpers/app';

test.describe('@security Electron renderer boundaries', () => {
  test('blocks traversal, popups, and Node/Electron escape hatches', async ({ session }) => {
    const { app, page } = session;
    const traversalStatus = await page.evaluate(async () => {
      const response = await fetch('app://powermove/../package.json');
      return response.status;
    });
    expect(traversalStatus).not.toBe(200);

    const windowsBefore = app.windows().length;
    const popupWasDenied = await page.evaluate(() => window.open('https://example.com') === null);
    expect(popupWasDenied).toBe(true);
    await page.waitForTimeout(250);
    expect(app.windows()).toHaveLength(windowsBefore);

    const bridge = await page.evaluate(() => {
      const powermove = (window as any).powermove;
      return {
        present: Boolean(powermove),
        hasIpcRenderer: Boolean(powermove && ('ipcRenderer' in powermove)),
        hasRequire: Boolean(powermove && ('require' in powermove)),
        globalRequire: typeof (window as any).require
      };
    });
    expect(bridge).toEqual({
      present: true,
      hasIpcRenderer: false,
      hasRequire: false,
      globalRequire: 'undefined'
    });
  });

  test('runs generated code inside the sandbox host', async ({ session }) => {
    const hostStatus = await session.page.evaluate(async () => (await fetch('host/sandbox.html')).status);
    test.skip(hostStatus === 404, 'host/sandbox.html is not present in this build');

    const answer = await session.page.evaluate(() => new Promise<number>((resolve, reject) => {
      const channel = 'powermove-isolated-script-result';
      const token = `e2e-${Date.now()}`;
      const iframe = document.createElement('iframe');
      iframe.sandbox.add('allow-scripts');
      iframe.src = 'host/sandbox.html';
      iframe.hidden = true;
      const cleanup = () => {
        window.removeEventListener('message', onMessage);
        iframe.remove();
        clearTimeout(timer);
      };
      const onMessage = (event: MessageEvent) => {
        if (event.data?.channel !== channel || event.data?.token !== token) return;
        cleanup();
        if (!event.data.ok) reject(new Error(event.data.error || 'Sandbox failed'));
        else resolve(event.data.result);
      };
      const timer = setTimeout(() => {
        cleanup();
        reject(new Error('Timed out waiting for host/sandbox.html'));
      }, 5_000);
      window.addEventListener('message', onMessage);
      iframe.addEventListener('load', () => iframe.contentWindow?.postMessage({
        channel: 'powermove-isolated-script',
        token,
        code: 'return 6 * 7;',
        project: { layers: [] },
        input: {},
        runId: 'e2e',
        runtime: 1_000
      }, '*'), { once: true });
      document.body.appendChild(iframe);
    }));

    expect(answer).toBe(42);
  });
});
