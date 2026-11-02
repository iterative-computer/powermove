import { test, expect } from 'bun:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { PageApplier } from '../src/applier';
import { RemoteDOM } from '../src/shim';

test('compat host envelope runs the production applier and returns real node events', async () => {
  const page = new JSDOM('<!doctype html><html><head></head><body></body></html>', {
    pretendToBeVisual: true,
    runScripts: 'outside-only'
  });
  const window = page.window as any;
  const feedback: any[] = [];
  let enqueue: ((value: any) => void) | undefined;
  const responses: any[] = [];
  let sequence = 0;
  const send = (events: any[]) => {
    const response = { json: async () => ({ generation: 1, seq: ++sequence, events }) };
    if (enqueue) {
      const resolve = enqueue;
      enqueue = undefined;
      resolve(response);
    } else responses.push(response);
  };
  const shim = new RemoteDOM({
    session: { panel: 'synthetic', generation: 1 },
    send(batch) {
      send([{ kind: 'batch', panel: 1, batch }]);
      return true;
    }
  });
  window.PMRemoteDOM = { PageApplier };
  window.PM_HOST_CONFIG = { base: 'pmcompat://host/synthetic/', generation: 1 };
  window.XMLHttpRequest = class {
    url = '';
    responseText = '';
    open(_method: string, url: string) {
      this.url = url;
    }
    send() {
      const request = JSON.parse(new URL(this.url).searchParams.get('q')!);
      if (request.kind === 'remote_dom') {
        feedback.push(request.payload);
        shim.receive(request.payload);
      }
      this.responseText = JSON.stringify({ ok: true, value: null });
    }
  };
  window.fetch = async () =>
    responses.shift() ??
    new Promise((resolve) => {
      enqueue = resolve;
    });
  try {
    window.eval(
      readFileSync(
        new URL('../../../crates/compat-host/src/remote-dom.js', import.meta.url),
        'utf8'
      )
    );
    window.document.dispatchEvent(new window.Event('DOMContentLoaded'));
    send([
      { kind: 'create', panel: 1 },
      { kind: 'slot', panel: 1, slot: {} }
    ]);
    const button = shim.document.createElement('button');
    button.textContent = 'Synthetic';
    button.addEventListener('click', () => {
      button.textContent = 'Clicked';
    });
    shim.document.body.append(button);
    const settle = async () => {
      for (let n = 0; n < 20; n++) {
        shim.flush();
        await Promise.resolve();
      }
    };
    await settle();
    expect(feedback.some((message) => message.kind === 'applied')).toBe(true);
    window.document
      .querySelector('button')
      .dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    await settle();
    expect(window.document.querySelector('button').textContent).toBe('Clicked');
    expect(feedback.some((message) => message.kind === 'event')).toBe(true);
  } finally {
    shim.dispose();
    page.window.close();
  }
});
