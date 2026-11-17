import { expect, test } from 'bun:test';
import { JSDOM } from 'jsdom';
import { RemoteDOM } from '../src/shim';
import { PageApplier } from '../src/applier';
import type { Batch } from '../src/protocol';

/** One page: a real document and the production applier, fed by `shim`. */
function page(shim: () => RemoteDOM) {
  const window = new JSDOM('<!doctype html><html><head></head><body></body></html>', {
    pretendToBeVisual: true
  }).window;
  const applier = new PageApplier({
    document: window.document,
    session: { panel: 'fixture', generation: 1 },
    send(message) {
      queueMicrotask(() => shim().receive(structuredClone(message)));
    }
  });
  return { window, applier };
}

/**
 * PM-509: the window's extension page stopped applying, and every panel in
 * it stayed blank until extensions restarted. A fresh page given the realm's
 * whole document has every panel back: its content, what changed while no
 * page applied, and events from it reach the realm's listeners.
 */
test('a fresh page after a fault gets the whole document back, and events work', async () => {
  let current: ReturnType<typeof page>;
  const errors: unknown[] = [];
  const shim: RemoteDOM = new RemoteDOM({
    session: { panel: 'fixture', generation: 1 },
    send(batch: Batch) {
      const target = current;
      queueMicrotask(() => target.applier.apply(structuredClone(batch)));
      return true;
    },
    reportError: (error) => errors.push(error)
  });
  const settle = async () => {
    for (let i = 0; i < 16; i++) {
      await Promise.resolve();
      shim.flush();
    }
  };
  current = page(() => shim);
  const d = shim.document;
  const panels = ['anchor-point', 'curve-editor'].map((id) => {
    const root = d.createElement('section');
    root.setAttribute('data-pm-contribution', id);
    root.innerHTML = `<h2>${id}</h2><button>Reset</button>`;
    d.body.append(root);
    return root;
  });
  let clicks = 0;
  panels[1].querySelector('button')!.addEventListener('click', () => clicks++);
  await settle();
  expect(current.window.document.body.innerHTML).toBe(d.body.innerHTML);

  // The page faults: nothing more reaches it, and the realm keeps its DOM.
  shim.receive({ kind: 'fault', session: shim.session, seq: 1, message: 'page fault' });
  panels[0].querySelector('h2')!.textContent = 'changed while no page applied';
  await settle();
  expect(shim.inspect().retired).toBe(false);

  // A fresh page, and the whole document again.
  const stale = current;
  current = page(() => shim);
  expect(shim.replay()).toBe(true);
  await settle();
  const body = current.window.document.body;
  expect(body.innerHTML).toBe(d.body.innerHTML);
  expect(body.querySelectorAll('[data-pm-contribution]').length).toBe(2);
  expect(body.textContent).toContain('changed while no page applied');
  expect(stale.window.document.body.textContent).not.toContain('changed while');

  // An event on the fresh page reaches the realm's listener.
  const button = current.applier.node(shim.id(panels[1].querySelector('button')!)) as HTMLElement;
  button.dispatchEvent(new current.window.MouseEvent('click', { bubbles: true }));
  await settle();
  expect(clicks).toBe(1);

  // And it keeps applying what changes next.
  panels[1].querySelector('h2')!.textContent = 'after the replay';
  await settle();
  expect(body.textContent).toContain('after the replay');
  expect(shim.inspect().lifecycle).not.toBe('faulted');
  shim.dispose();
});
