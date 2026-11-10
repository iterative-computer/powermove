import { describe, test, expect } from 'bun:test';
import { harness } from './harness';
import { RemoteDOM } from '../src/shim';
import { RemoteEvent } from '../src/events';
import { PageApplier } from '../src/applier';
import { JSDOM } from 'jsdom';
import type { Batch, PageMessage } from '../src/protocol';

describe('remote DOM mirror and real-document applier', () => {
  test('innerHTML applies browser DOMString coercion before parsing', () => {
    const h = harness();
    const reference = new JSDOM();
    try {
      for (const tag of ['div', 'template', 'svg']) {
        const remote =
          tag === 'svg'
            ? h.shim.document.createElementNS('http://www.w3.org/2000/svg', tag)
            : h.shim.document.createElement(tag);
        const real =
          tag === 'svg'
            ? reference.window.document.createElementNS('http://www.w3.org/2000/svg', tag)
            : reference.window.document.createElement(tag);
        for (const value of [42, false, undefined, null, { toString: () => '<b>Synthetic</b>' }]) {
          remote.innerHTML = value;
          (real as any).innerHTML = value;
          expect(remote.innerHTML).toBe(real.innerHTML);
        }
        expect(() => {
          remote.innerHTML = Symbol('synthetic');
        }).toThrow(TypeError);
        expect(() => {
          (real as any).innerHTML = Symbol('synthetic');
        }).toThrow();
      }
    } finally {
      h.close();
      reference.window.close();
    }
  });
  test('HTML templates, SVG namespaces, selectors and exact text/attribute/style writes', async () => {
    const h = harness(),
      d = h.shim.document;
    const template = d.createElement('template');
    template.innerHTML = '<section><b class="label">One &amp; two</b><!--anchor--></section>';
    const section = template.content.firstChild.cloneNode(true);
    section.setAttribute('data-id', 'authored');
    section.style.setProperty('color', 'red');
    const svg = d.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.innerHTML = '<path d="M0 1L2 3"></path>';
    section.append(svg);
    d.body.append(section);
    await h.settle();
    expect(h.errors).toEqual([]);
    expect(h.applier.inspect().retired).toBe(false);
    const real = h.page.window.document.querySelector('section')!;
    expect(real.textContent).toBe('One & two');
    expect(real.getAttribute('data-id')).toBe('authored');
    expect(real.querySelector('svg')!.namespaceURI).toBe('http://www.w3.org/2000/svg');
    expect((real as HTMLElement).style.color).toBe('red');
    section.querySelector('b').textContent = 'Exact 🦀';
    section.removeAttribute('data-id');
    section.style.color = 'blue';
    await h.settle();
    expect(real.textContent).toBe('Exact 🦀');
    expect(real.hasAttribute('data-id')).toBe(false);
    expect((real as HTMLElement).style.color).toBe('blue');
    h.close();
  });
  test('moves/reorders retain real node identity, detached nodes can be edited and reattached', async () => {
    const h = harness(),
      d = h.shim.document;
    const a = d.createElement('div'),
      b = d.createElement('div'),
      input = d.createElement('input');
    a.append(input);
    d.body.append(a, b);
    await h.settle();
    const real = h.applier.node(h.shim.id(input));
    b.append(input);
    await h.settle();
    expect(h.applier.node(h.shim.id(input))).toBe(real);
    expect((real as Node).parentNode).toBe(h.applier.node(h.shim.id(b)) as ParentNode);
    input.remove();
    await h.settle();
    input.value = 'retained';
    input.setAttribute('title', 'detached');
    a.prepend(input);
    await h.settle();
    expect((real as HTMLInputElement).value).toBe('retained');
    expect((real as HTMLElement).title).toBe('detached');
    expect((real as Node).parentNode).toBe(h.applier.node(h.shim.id(a)) as ParentNode);
    expect(h.errors).toEqual([]);
    h.close();
  });
  test('capture → target → bubble, this/currentTarget, once/passive and removal during dispatch', async () => {
    const h = harness(),
      d = h.shim.document,
      w = h.shim.window;
    const parent = d.createElement('div'),
      button = d.createElement('button');
    parent.append(button);
    d.body.append(parent);
    const order: string[] = [];
    w.addEventListener('click', () => order.push('window capture'), true);
    parent.addEventListener('click', () => order.push('capture'), true);
    button.addEventListener(
      'click',
      function (this: unknown, e: RemoteEvent) {
        expect(this).toBe(button);
        expect(e.currentTarget).toBe(button);
        order.push('target');
      },
      { once: true }
    );
    parent.addEventListener(
      'click',
      (e: RemoteEvent) => {
        order.push('bubble');
        e.preventDefault();
      },
      { passive: true }
    );
    const removed = () => order.push('removed');
    parent.addEventListener('click', () => parent.removeEventListener('click', removed));
    parent.addEventListener('click', removed);
    await h.settle();
    (h.applier.node(h.shim.id(button)) as HTMLElement).click();
    await h.settle();
    expect(order).toEqual(['window capture', 'capture', 'target', 'bubble']);
    expect(h.late).toEqual([]);
    order.length = 0;
    (h.applier.node(h.shim.id(button)) as HTMLElement).click();
    await h.settle();
    expect(order).toEqual(['window capture', 'capture', 'bubble']);
    expect(h.errors).toEqual([]);
    h.close();
  });
  test('input/checkbox/selection and focus state are current before callbacks, no value echo', async () => {
    const h = harness(),
      d = h.shim.document;
    const input = d.createElement('input');
    input.value = 'first';
    input.addEventListener('input', () => {
      expect(input.value).toBe('typed');
      expect(input.selectionStart).toBe(2);
    });
    d.body.append(input);
    await h.settle();
    const real = h.applier.node(h.shim.id(input)) as HTMLInputElement;
    input.focus();
    input.setSelectionRange(1, 3, 'backward');
    await h.settle();
    expect(h.page.window.document.activeElement).toBe(real);
    expect(real.selectionStart).toBe(1);
    expect(real.selectionEnd).toBe(3);
    const before = h.batches.length;
    real.value = 'typed';
    real.setSelectionRange(2, 2);
    real.dispatchEvent(new h.page.window.Event('input', { bubbles: true }));
    await h.settle();
    expect(h.batches.length).toBe(before);
    expect(h.shim.document.activeElement).toBe(input);
    const checkbox = d.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.checked = true;
    d.body.append(checkbox);
    await h.settle();
    expect((h.applier.node(h.shim.id(checkbox)) as HTMLInputElement).checked).toBe(true);
    expect(h.errors).toEqual([]);
    h.close();
  });
  test('cached layout never blocks; explicit async measurement and computed styles', async () => {
    const h = harness(),
      node = h.shim.document.createElement('div');
    node.style.color = 'red';
    h.shim.document.body.append(node);
    await h.settle();
    const real = h.applier.node(h.shim.id(node)) as HTMLElement;
    real.getBoundingClientRect = () => ({
      x: 12,
      y: 7,
      width: 100,
      height: 20,
      left: 12,
      top: 7,
      right: 112,
      bottom: 27,
      toJSON() {}
    });
    expect(node.getBoundingClientRect().width).toBe(0);
    expect(h.shim.layoutStatus(node).stale).toBe(true);
    const measured = h.shim.measure(node, ['color']);
    await h.settle();
    expect((await measured).rect.width).toBe(100);
    expect(node.getBoundingClientRect().left).toBe(12);
    expect(h.shim.computedStyle(node).color).toBe('rgb(255, 0, 0)');
    expect(h.errors).toEqual([]);
    h.close();
  });
  test('a layout read the page has not measured waits for the page, as a forced layout', async () => {
    const page = new JSDOM('<!doctype html><html><head></head><body></body></html>');
    const replies: PageMessage[] = [];
    let asked = 0,
      answer: boolean | null = true;
    const shim: RemoteDOM = new RemoteDOM({
      session: { panel: 'fixture', generation: 1 },
      // The host hands batches to the page and routes its answer back.
      send(batch) {
        applier.apply(structuredClone(batch));
        return true;
      },
      measureSync(request) {
        asked++;
        const reply = replies.find((m) => m.kind === 'layout' && m.request === request);
        return (answer && reply) || { unavailable: answer === null ? 'no-page' : 'timeout' };
      }
    });
    const applier = new PageApplier({
      document: page.window.document,
      session: shim.session,
      send: (message) => replies.push(structuredClone(message))
    });
    const caret = shim.document.createElement('span');
    shim.document.body.append(caret);
    shim.flush();
    const real = applier.node(shim.id(caret)) as HTMLElement;
    let left = 30;
    real.getBoundingClientRect = () =>
      ({
        x: left,
        y: 4,
        width: 2,
        height: 18,
        left,
        top: 4,
        right: left + 2,
        bottom: 22
      }) as DOMRect;
    let resized = 0;
    new shim.window.ResizeObserver(() => resized++).observe(caret);
    // Fresh without waiting for a page round trip in another task.
    expect(caret.getBoundingClientRect().left).toBe(30);
    expect(asked).toBe(1);
    // Unchanged document: the measured layout is current, nothing is asked.
    expect(caret.getBoundingClientRect().left).toBe(30);
    expect(asked).toBe(1);
    // A change in the document makes the next read measure again.
    left = 44;
    caret.textContent = 'x';
    expect(caret.getBoundingClientRect().left).toBe(44);
    expect(asked).toBe(2);
    // Observers hear about it after the task, never inside the read.
    expect(resized).toBe(0);
    await Promise.resolve();
    await Promise.resolve();
    expect(resized).toBe(1);
    // Without a page every read asks (the host answers at once) and keeps
    // the cached layout.
    answer = null;
    caret.textContent = 'xy';
    expect(caret.getBoundingClientRect().left).toBe(44);
    caret.textContent = 'xyz';
    expect(caret.getBoundingClientRect().left).toBe(44);
    expect(asked).toBe(4);
    // A page too slow to answer gets no more synchronous reads until its next
    // layout answer arrives.
    answer = false;
    caret.textContent = 'xyzw';
    expect(caret.getBoundingClientRect().left).toBe(44);
    caret.textContent = 'xyzwv';
    expect(caret.getBoundingClientRect().left).toBe(44);
    expect(asked).toBe(5);
    shim.flush();
    left = 50;
    shim.receive(replies.filter((m) => m.kind === 'layout').pop()!);
    answer = true;
    caret.textContent = 'u';
    expect(caret.getBoundingClientRect().left).toBe(50);
    expect(asked).toBe(6);
    shim.dispose();
    applier.dispose();
    page.window.close();
  });
  test('scroll commands and resize observation use asynchronous page snapshots', async () => {
    const h = harness(),
      node = h.shim.document.createElement('div');
    h.shim.document.body.append(node);
    await h.settle();
    node.scrollTo({ left: 20, top: 40, behavior: 'instant' });
    await h.settle();
    const real = h.applier.node(h.shim.id(node)) as HTMLElement;
    expect(real.scrollLeft).toBe(20);
    expect(real.scrollTop).toBe(40);
    let width = -1;
    const ro = new h.shim.window.ResizeObserver((entries: ResizeObserverEntry[]) => {
      width = entries[0].contentRect.width;
    });
    ro.observe(node);
    await h.settle();
    await h.shim.receive({
      kind: 'layout',
      session: h.shim.session,
      applied: h.shim.inspect().sent,
      request: null,
      entries: [
        {
          id: h.shim.id(node),
          rect: { x: 0, y: 0, width: 50, height: 10 },
          scroll_left: 20,
          scroll_top: 40,
          scroll_width: 50,
          scroll_height: 10,
          client_width: 50,
          client_height: 10,
          styles: {}
        }
      ]
    });
    expect(width).toBe(50);
    ro.disconnect();
    await h.settle();
    expect(h.errors).toEqual([]);
    h.close();
  });
  test('backpressure retains exact ordered batches; generation retirement rejects stale feedback', async () => {
    const attempts: Batch[] = [];
    let accepting = false;
    const shim = new RemoteDOM({
      session: { panel: 'p', generation: 2 },
      send: (batch) => {
        attempts.push(structuredClone(batch));
        return accepting;
      }
    });
    const node = shim.document.createElement('p');
    node.textContent = 'before';
    shim.document.body.append(node);
    shim.flush();
    node.textContent = 'after';
    accepting = true;
    shim.flush();
    shim.flush();
    expect(attempts[0]).toEqual(attempts[1]);
    expect(attempts.at(-1)!.seq).toBe(2);
    expect(shim.receive({ kind: 'applied', session: { panel: 'p', generation: 1 }, seq: 2 })).toBe(
      false
    );
    const wait = shim.measure(node);
    shim.dispose();
    await expect(wait).rejects.toThrow('retired');
    expect(shim.flush()).toBeNull();
  });
  test('browser default cancellation is explicit: declared block or diagnosed late cancellation', async () => {
    const h = harness(),
      button = h.shim.document.createElement('button');
    h.shim.document.body.append(button);
    button.addEventListener('click', (event: RemoteEvent) => event.preventDefault());
    await h.settle();
    const real = h.applier.node(h.shim.id(button)) as HTMLElement;
    expect(
      real.dispatchEvent(new h.page.window.MouseEvent('click', { bubbles: true, cancelable: true }))
    ).toBe(true);
    await h.settle();
    expect(h.late).toEqual(['click']);
    h.shim.setEventPolicy(button, 'click', 'block');
    await h.settle();
    expect(
      real.dispatchEvent(new h.page.window.MouseEvent('click', { bubbles: true, cancelable: true }))
    ).toBe(false);
    await h.settle();
    expect(h.late).toEqual(['click']);
    h.close();
  });
  test('preflight rejects malformed batches before touching the document and fences faults', () => {
    const page = new JSDOM('<html><body></body></html>'),
      messages: unknown[] = [];
    const applier = new PageApplier({
      document: page.window.document,
      session: { panel: 'p', generation: 1 },
      send: (m) => messages.push(m)
    });
    const batch: Batch = {
      version: 1,
      session: { panel: 'p', generation: 1 },
      seq: 1,
      caused_by: null,
      ops: [
        { op: 'create', id: 6, kind: 'element', name: 'p', namespace: null },
        { op: 'insert', id: 6, parent: 4, before: null },
        { op: 'property', id: 6, name: 'innerHTML', value: '<script></script>' }
      ]
    };
    expect(applier.apply(batch)).toBe(false);
    expect(page.window.document.body.childNodes.length).toBe(0);
    expect(applier.inspect().retired).toBe(true);
    expect((messages[0] as any).kind).toBe('fault');
    page.window.close();
  });
});

test('DOM text selection and Range retain mirror identity and reach the real document', async () => {
  const h = harness(),
    d = h.shim.document,
    p = d.createElement('p');
  p.textContent = 'Hello 🦀 world';
  d.body.append(p);
  await h.settle();
  const selection = d.getSelection();
  expect(selection).toBe(h.shim.window.getSelection());
  const range = d.createRange();
  range.setStart(p.firstChild, 0);
  range.setEnd(p.firstChild, 5);
  selection.addRange(range);
  expect(selection.toString()).toBe('Hello');
  expect(selection.getRangeAt(0).startContainer).toBe(p.firstChild);
  await h.settle();
  expect(h.page.window.document.getSelection()!.toString()).toBe('Hello');
  selection.setBaseAndExtent(p.firstChild, 5, p.firstChild, 0);
  await h.settle();
  expect(selection.toString()).toBe('Hello');
  expect(selection.anchorOffset).toBe(5);
  selection.removeAllRanges();
  await h.settle();
  expect(h.page.window.document.getSelection()!.rangeCount).toBe(0);
  expect(h.errors).toEqual([]);
  h.close();
});
test('real selectionchange refreshes the stable realm Selection before the handler', async () => {
  const h = harness(),
    d = h.shim.document,
    p = d.createElement('p');
  p.textContent = 'Editable text';
  d.body.append(p);
  let selected = '';
  d.addEventListener('selectionchange', () => {
    selected = d.getSelection().toString();
  });
  await h.settle();
  const real = h.applier.node(h.shim.id(p)) as HTMLElement;
  h.page.window.document.getSelection()!.setBaseAndExtent(real.firstChild!, 0, real.firstChild!, 8);
  h.page.window.document.dispatchEvent(new h.page.window.Event('selectionchange'));
  await h.settle();
  expect(selected).toBe('Editable');
  expect(d.getSelection().anchorNode).toBe(p.firstChild);
  h.close();
});
test('Range.selectNodeContents preserves comment data offsets', async () => {
  const h = harness(),
    d = h.shim.document;
  try {
    const comment = d.createComment('Synthetic 🦀');
    d.body.append(comment);
    await h.settle();
    const reference = h.page.window.document.createRange();
    reference.selectNodeContents(h.applier.node(h.shim.id(comment)) as Comment);
    const range = d.createRange();
    range.selectNodeContents(comment);
    expect(range.startOffset).toBe(reference.startOffset);
    expect(range.endOffset).toBe(reference.endOffset);
    expect(range.toString()).toBe(reference.toString());
    expect(h.errors).toEqual([]);
  } finally {
    h.close();
  }
});
test('FormData collects successful text/checkbox/select controls without including disabled or unchecked fields', async () => {
  const h = harness(),
    d = h.shim.document;
  const form = d.createElement('form');
  form.innerHTML =
    '<input name="title" value="A"><input name="ignored" value="B" disabled><input type="checkbox" name="enabled" checked><input type="checkbox" name="unchecked"><select name="mode"><option value="one">One</option><option value="two">Two</option></select><textarea name="body">Draft</textarea><button name="submit" value="save">Save</button>';
  d.body.append(form);
  form.querySelector('input').value = 'Authored';
  form.querySelector('select').value = 'two';
  const data = new h.shim.window.FormData(form);
  expect([...data.entries()]).toEqual([
    ['title', 'Authored'],
    ['enabled', 'on'],
    ['mode', 'two'],
    ['body', 'Draft']
  ]);
  data.append('title', 'Second');
  expect(data.getAll('title')).toEqual(['Authored', 'Second']);
  data.set('title', 'Replaced');
  expect(data.get('title')).toBe('Replaced');
  data.delete('mode');
  expect(data.has('mode')).toBe(false);
  expect(form.querySelector('input').form).toBe(form);
  await h.settle();
  expect(h.errors).toEqual([]);
  h.close();
});
test('on-event properties run once, preserve return-false and listener exceptions do not stop propagation', async () => {
  const h = harness(),
    d = h.shim.document,
    b = d.createElement('button');
  d.body.append(b);
  let count = 0,
    after = false;
  b.onclick = () => {
    count++;
    return false;
  };
  b.addEventListener('click', () => {
    throw new Error('fixture exception');
  });
  d.body.addEventListener('click', () => (after = true));
  const event = new RemoteEvent('click', { bubbles: true, cancelable: true });
  expect(b.dispatchEvent(event)).toBe(false);
  expect(count).toBe(1);
  expect(after).toBe(true);
  expect(h.errors).toHaveLength(1);
  expect((h.errors[0] as Error).message).toBe('fixture exception');
  await h.settle();
  h.close();
});
test('stopPropagation/stopImmediatePropagation, nonbubbling capture, abort signals and noncancelable events follow DOM rules', () => {
  const h = harness(),
    d = h.shim.document,
    parent = d.createElement('div'),
    b = d.createElement('button');
  parent.append(b);
  d.body.append(parent);
  const calls: string[] = [];
  parent.addEventListener('focus', () => calls.push('capture'), true);
  parent.addEventListener('focus', () => calls.push('bubble'));
  b.addEventListener('focus', (e: RemoteEvent) => {
    calls.push('first');
    e.stopPropagation();
    e.preventDefault();
  });
  b.addEventListener('focus', () => calls.push('second'));
  const focus = new RemoteEvent('focus');
  expect(b.dispatchEvent(focus)).toBe(true);
  expect(calls).toEqual(['capture', 'first', 'second']);
  expect(focus.currentTarget).toBeNull();
  calls.length = 0;
  const controller = new AbortController();
  b.addEventListener('x', () => calls.push('aborted'), { signal: controller.signal });
  controller.abort();
  b.addEventListener('x', (e: RemoteEvent) => {
    calls.push('immediate');
    e.stopImmediatePropagation();
  });
  b.addEventListener('x', () => calls.push('never'));
  b.dispatchEvent(new RemoteEvent('x'));
  expect(calls).toEqual(['immediate']);
  h.close();
});
test('multiple property writes coalesce to authored final state in one microtask batch', async () => {
  const h = harness(),
    d = h.shim.document,
    input = d.createElement('input');
  d.body.append(input);
  await h.settle();
  const before = h.batches.length;
  input.value = 'a';
  input.value = 'ab';
  input.value = 'abc';
  input.disabled = true;
  await h.settle();
  expect(h.batches.length - before).toBe(1);
  expect(
    h.batches.at(-1)!.ops.filter((op) => op.op === 'property' && op.name === 'value')
  ).toHaveLength(1);
  expect((h.applier.node(h.shim.id(input)) as HTMLInputElement).value).toBe('abc');
  h.close();
});
test('stale/out-of-order events and unknown layouts never overwrite current mirror state', async () => {
  const h = harness(),
    input = h.shim.document.createElement('input');
  input.value = 'initial';
  h.shim.document.body.append(input);
  await h.settle();
  const base = {
    kind: 'event' as const,
    session: h.shim.session,
    seq: 2,
    applied: h.shim.inspect().applied,
    target: h.shim.id(input),
    event: 'input',
    bubbles: true,
    cancelable: false,
    default_prevented: false,
    data: { state: { value: 'late' } }
  };
  expect(h.shim.receive(base)).toBe(false);
  expect(input.value).toBe('initial');
  expect(h.shim.receive({ ...base, seq: 1, session: { ...base.session, generation: 2 } })).toBe(
    false
  );
  expect(input.value).toBe('initial');
  h.close();
});
for (const op of [
  {
    op: 'create',
    id: 6,
    kind: 'element',
    name: 'svg:script',
    namespace: 'http://www.w3.org/2000/svg'
  },
  { op: 'create', id: 6, kind: 'element', name: 'script', namespace: null },
  { op: 'attribute', id: 4, name: 'onclick', value: 'logic()', namespace: null },
  { op: 'attribute', id: 4, name: 'srcdoc', value: '<script/>', namespace: null },
  { op: 'attribute', id: 4, name: 'href', value: 'java\nscript:logic()', namespace: null },
  { op: 'attribute', id: 4, name: 'href', value: 'data:text/html,<script/>', namespace: null }
])
  test(`applier refuses executable page content: ${JSON.stringify(op)}`, () => {
    const page = new JSDOM('<html><body></body></html>');
    const applier = new PageApplier({
      document: page.window.document,
      session: { panel: 'p', generation: 1 },
      send() {}
    });
    expect(
      applier.apply({
        version: 1,
        session: { panel: 'p', generation: 1 },
        seq: 1,
        caused_by: null,
        ops: [op] as any
      })
    ).toBe(false);
    page.window.close();
  });

test('queued older form mutations cannot overwrite a newer native edit; mutations caused by that edit apply', () => {
  const page = new JSDOM('<html><body></body></html>');
  const outgoing: Batch[] = [],
    feedback: any[] = [];
  const shim = new RemoteDOM({
    session: { panel: 'race', generation: 1 },
    send: (batch) => {
      outgoing.push(structuredClone(batch));
      return true;
    }
  });
  const applier = new PageApplier({
    document: page.window.document,
    session: shim.session,
    send: (message) => feedback.push(structuredClone(message))
  });
  const input = shim.document.createElement('input');
  input.value = 'initial';
  input.addEventListener('input', () => (input.value = input.value.toUpperCase()));
  shim.document.body.append(input);
  shim.flush();
  expect(applier.apply(outgoing.shift()!)).toBe(true);
  for (const message of feedback.splice(0)) shim.receive(message);
  input.value = 'older queued write';
  shim.flush();
  const old = outgoing.shift()!;
  const real = applier.node(shim.id(input)) as HTMLInputElement;
  real.value = 'new edit';
  real.dispatchEvent(new page.window.Event('input', { bubbles: true }));
  expect(applier.apply(old)).toBe(true);
  expect(real.value).toBe('new edit');
  for (const message of feedback.splice(0)) shim.receive(message);
  shim.flush();
  expect(input.value).toBe('NEW EDIT');
  expect(applier.apply(outgoing.shift()!)).toBe(true);
  expect(real.value).toBe('NEW EDIT');
  shim.dispose();
  applier.dispose();
  page.window.close();
});
test('large mounts split into bounded ordered batches and preserve every authored node', async () => {
  const page = new JSDOM('<html><body></body></html>');
  const batches: Batch[] = [],
    errors: unknown[] = [];
  const shim = new RemoteDOM({
    session: { panel: 'large', generation: 1 },
    maxBatchOperations: 20,
    maxBatchBytes: 4096,
    send(batch) {
      batches.push(structuredClone(batch));
      queueMicrotask(() => applier.apply(batch));
      return true;
    },
    reportError: (error) => errors.push(error)
  });
  const applier = new PageApplier({
    document: page.window.document,
    session: shim.session,
    send(message) {
      queueMicrotask(() => shim.receive(message));
    }
  });
  for (let i = 0; i < 100; i++) {
    const node = shim.document.createElement('p');
    node.textContent = `Row ${i}`;
    shim.document.body.append(node);
  }
  shim.flush();
  for (let i = 0; i < 200; i++) {
    await Promise.resolve();
    shim.flush();
  }
  expect(batches.length).toBeGreaterThan(10);
  expect(batches.every((batch) => batch.ops.length <= 20)).toBe(true);
  expect(batches.map((batch) => batch.seq)).toEqual(
    Array.from({ length: batches.length }, (_, i) => i + 1)
  );
  expect([...page.window.document.querySelectorAll('p')].map((node) => node.textContent)).toEqual(
    Array.from({ length: 100 }, (_, i) => `Row ${i}`)
  );
  expect(errors).toEqual([]);
  expect(shim.inspect().pending).toBe(false);
  shim.dispose();
  applier.dispose();
  page.window.close();
});
test('multi-select, live :checked queries, programmatic deselection and checkbox clicks use property state', async () => {
  const h = harness(),
    d = h.shim.document;
  const select = d.createElement('select');
  select.multiple = true;
  select.innerHTML =
    '<option value="a">A</option><option value="b">B</option><option value="c">C</option>';
  select.options[0].selected = true;
  select.options[2].selected = true;
  d.body.append(select);
  await h.settle();
  expect([...select.querySelectorAll(':checked')].map((o: any) => o.value)).toEqual(['a', 'c']);
  const real = h.applier.node(h.shim.id(select)) as HTMLSelectElement;
  real.options[0].selected = false;
  real.options[1].selected = true;
  real.dispatchEvent(new h.page.window.Event('change', { bubbles: true }));
  await h.settle();
  expect([...select.querySelectorAll(':checked')].map((o: any) => o.value)).toEqual(['b', 'c']);
  select.selectedIndex = -1;
  await h.settle();
  expect(select.value).toBe('');
  expect(real.selectedIndex).toBe(-1);
  const checkbox = d.createElement('input');
  checkbox.type = 'checkbox';
  let changed = 0;
  checkbox.addEventListener('change', () => changed++);
  d.body.append(checkbox);
  checkbox.click();
  expect(checkbox.checked).toBe(true);
  expect(changed).toBe(1);
  await h.settle();
  expect((h.applier.node(h.shim.id(checkbox)) as HTMLInputElement).checked).toBe(true);
  h.close();
});

test('mixed tree mutations and keyed moves agree with an independent real DOM over 100 batches', async () => {
  const h = harness(),
    d = h.shim.document;
  const parents = Array.from({ length: 3 }, () => d.createElement('section'));
  d.body.append(...parents);
  const nodes = Array.from({ length: 12 }, (_, i) => {
    const n = d.createElement('span');
    n.textContent = `Item ${i}`;
    parents[i % 3].append(n);
    return n;
  });
  function shape(node: any): any {
    if (node.nodeType === 3 || node.nodeType === 8) return [node.nodeType, node.data];
    return [
      node.localName,
      [...(node.attributes ?? [])].map((a: any) => [a.name, a.value]).sort(),
      [...node.childNodes].map(shape)
    ];
  }
  let seed = 987654;
  const random = (n: number) => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed % n;
  };
  for (let i = 0; i < 100; i++) {
    const node = nodes[random(nodes.length)],
      parent = parents[random(3)];
    switch (random(6)) {
      case 0:
        parent.prepend(node);
        break;
      case 1:
        parent.append(node);
        break;
      case 2:
        node.remove();
        node.textContent = `Detached ${i}`;
        break;
      case 3:
        node.setAttribute('data-step', String(i));
        node.classList.toggle('active');
        break;
      case 4:
        node.innerHTML = `<b>Nested ${i}</b><!--anchor-->`;
        break;
      case 5: {
        const fragment = d.createDocumentFragment();
        fragment.append(node);
        parent.append(fragment);
        break;
      }
    }
    await h.settle();
    expect(shape(h.page.window.document.body)).toEqual(shape(d.body));
    expect(h.errors).toEqual([]);
    expect(h.applier.inspect().retired).toBe(false);
  }
  h.close();
});
test('native focus updates activeElement even without an authored focus listener', async () => {
  const h = harness(),
    input = h.shim.document.createElement('input');
  h.shim.document.body.append(input);
  await h.settle();
  const real = h.applier.node(h.shim.id(input)) as HTMLInputElement;
  real.focus();
  await h.settle();
  expect(h.shim.document.activeElement).toBe(input);
  real.blur();
  await h.settle();
  expect(h.shim.document.activeElement).toBe(h.shim.document.body);
  h.close();
});
test('a page cannot acknowledge an unsent batch while transport is blocked', () => {
  const shim = new RemoteDOM({ session: { panel: 'blocked', generation: 1 }, send: () => false });
  shim.flush();
  expect(shim.receive({ kind: 'applied', session: shim.session, seq: 1 })).toBe(false);
  expect(shim.inspect().applied).toBe(0);
  shim.dispose();
});
test('MutationObserver attribute updates run at the microtask checkpoint and preserve old values', async () => {
  const h = harness(),
    node = h.shim.document.createElement('div');
  h.shim.document.body.append(node);
  await h.settle();
  const records: any[] = [];
  const observer = new h.shim.window.MutationObserver((next: any[]) => records.push(...next));
  observer.observe(node, {
    attributes: true,
    attributeOldValue: true,
    attributeFilter: ['data-test']
  });
  node.setAttribute('data-test', 'first');
  node.setAttribute('data-test', 'second');
  node.setAttribute('ignored', 'value');
  expect(records).toHaveLength(0);
  await h.settle();
  expect(records.map((record) => [record.attributeName, record.oldValue])).toEqual([
    ['data-test', null],
    ['data-test', 'first']
  ]);
  observer.disconnect();
  h.close();
});

test('namespaced attributes survive clone, updates and removal on the real SVG element', async () => {
  const h = harness(),
    d = h.shim.document,
    ns = 'http://www.w3.org/1999/xlink';
  const svg = d.createElementNS('http://www.w3.org/2000/svg', 'svg'),
    use = d.createElementNS('http://www.w3.org/2000/svg', 'use');
  use.setAttributeNS(ns, 'xlink:href', '#authored');
  svg.append(use);
  d.body.append(svg.cloneNode(true));
  await h.settle();
  const shadow = d.querySelector('use'),
    real = h.page.window.document.querySelector('use')!;
  expect(shadow.getAttributeNS(ns, 'href')).toBe('#authored');
  expect(real.getAttributeNS(ns, 'href')).toBe('#authored');
  shadow.setAttributeNS(ns, 'xlink:href', '#updated');
  await h.settle();
  expect(real.getAttributeNS(ns, 'href')).toBe('#updated');
  shadow.removeAttributeNS(ns, 'href');
  await h.settle();
  expect(real.hasAttributeNS(ns, 'href')).toBe(false);
  expect(h.errors).toEqual([]);
  h.close();
});
test('cloneNode copies current form values without cloning event listeners', async () => {
  const h = harness(),
    d = h.shim.document,
    input = d.createElement('input');
  input.value = 'Current draft';
  let calls = 0;
  input.addEventListener('input', () => calls++);
  const clone = input.cloneNode(true);
  d.body.append(clone);
  await h.settle();
  expect(clone.value).toBe('Current draft');
  expect((h.applier.node(h.shim.id(clone)) as HTMLInputElement).value).toBe('Current draft');
  clone.dispatchEvent(new RemoteEvent('input'));
  expect(calls).toBe(0);
  h.close();
});
