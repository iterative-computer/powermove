import { test, expect } from 'bun:test';
import { compile, VERSION } from 'svelte/compiler';
import { harness } from './harness';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
const root = join(import.meta.dir, '..');
async function compileFixture(name: string) {
  const outdir = join(root, '.test-build', name);
  await mkdir(outdir, { recursive: true });
  const entry = join(outdir, 'entry.js');
  await Bun.write(
    entry,
    `import Component from '../../test/fixtures/${name}.svelte';\nimport { mount, unmount, flushSync, tick } from 'svelte';\nexport { Component, mount, unmount, flushSync, tick };`
  );
  const result = await Bun.build({
    entrypoints: [entry],
    outdir,
    target: 'browser',
    format: 'esm',
    conditions: ['browser', 'production'],
    define: { 'process.env.NODE_ENV': '"production"' },
    plugins: [
      {
        name: 'pinned-svelte',
        setup(build) {
          build.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({
            contents: compile(await Bun.file(path).text(), {
              filename: path,
              generate: 'client',
              dev: false
            }).js.code,
            loader: 'js'
          }));
        }
      }
    ]
  });
  if (!result.success) throw new Error(result.logs.map((log) => log.message).join('\n'));
  return await import(join(outdir, 'entry.js') + '?compiled=1');
}
test('Svelte 5.56.10 inspector: text, checkbox and select two-way bindings', async () => {
  expect(VERSION).toBe('5.56.10');
  const h = harness(),
    restore = h.shim.installGlobals();
  try {
    const { Component, mount, flushSync, unmount, tick } = await compileFixture('Inspector');
    let instance: any;
    flushSync(() => (instance = mount(Component, { target: h.shim.document.body })));
    await h.settle();
    expect(h.errors).toEqual([]);
    const d = h.page.window.document;
    expect(d.querySelector('output')!.textContent).toBe('Layer: normal');
    const input = d.querySelector('input')!;
    input.value = 'Caption';
    input.dispatchEvent(new h.page.window.Event('input', { bubbles: true }));
    await tick();
    await h.settle();
    expect(d.querySelector('output')!.textContent).toBe('Caption: normal');
    const checkbox = d.querySelector('input[type=checkbox]') as HTMLInputElement;
    checkbox.checked = false;
    checkbox.dispatchEvent(new h.page.window.Event('change', { bubbles: true }));
    const select = d.querySelector('select')!;
    select.value = 'multiply';
    select.dispatchEvent(new h.page.window.Event('change', { bubbles: true }));
    await tick();
    await h.settle();
    expect(d.querySelector('output')!.textContent).toBe('Caption: multiply');
    expect((d.querySelector('output') as HTMLElement).style.opacity).toBe('0.5');
    await unmount(instance);
    await h.settle();
    expect(d.body.childNodes.length).toBe(0);
    expect(h.errors).toEqual([]);
  } finally {
    restore();
    h.close();
  }
});
test('Svelte keyed list: delegated events, state, reorder and removal preserve real identity', async () => {
  const h = harness(),
    restore = h.shim.installGlobals();
  try {
    const { Component, mount, flushSync, unmount, tick } = await compileFixture('KeyedLayers');
    let instance: any;
    flushSync(() => (instance = mount(Component, { target: h.shim.document.body })));
    await h.settle();
    const d = h.page.window.document,
      original = d.querySelector('[data-id="2"]')!;
    (original.querySelector('button') as HTMLElement).click();
    await tick();
    await h.settle();
    expect(d.querySelector('output')!.textContent).toBe('2');
    expect(original.classList.contains('selected')).toBe(true);
    (d.querySelector('[data-action=reverse]') as HTMLElement).click();
    await tick();
    await h.settle();
    expect([...d.querySelectorAll('li')].map((el) => el.getAttribute('data-id'))).toEqual([
      '3',
      '2',
      '1'
    ]);
    expect(d.querySelector('[data-id="2"]')).toBe(original);
    (d.querySelector('[data-action=remove]') as HTMLElement).click();
    await tick();
    await h.settle();
    expect(d.querySelector('[data-id="2"]')).toBeNull();
    expect(h.errors).toEqual([]);
    await unmount(instance);
    await h.settle();
  } finally {
    restore();
    h.close();
  }
});
test('Svelte panel: conditional fragments, SVG, window events and listener cleanup', async () => {
  const h = harness(),
    restore = h.shim.installGlobals();
  try {
    const { Component, mount, flushSync, unmount, tick } = await compileFixture('Panel');
    let instance: any;
    flushSync(() => (instance = mount(Component, { target: h.shim.document.body })));
    await h.settle();
    const d = h.page.window.document;
    expect(d.querySelector('p')!.textContent).toBe('Closed');
    (d.querySelector('button') as HTMLElement).click();
    await tick();
    await h.settle();
    expect(d.querySelector('p')!.textContent).toBe('Ready');
    d.body.dispatchEvent(new h.page.window.KeyboardEvent('keydown', { bubbles: true, key: 'K' }));
    await tick();
    await h.settle();
    expect(d.querySelector('p')!.textContent).toBe('K');
    expect(d.querySelector('path')!.namespaceURI).toBe('http://www.w3.org/2000/svg');
    expect(h.errors).toEqual([]);
    await unmount(instance);
    await h.settle();
    expect(d.querySelector('svg')).toBeNull();
  } finally {
    restore();
    h.close();
  }
});
