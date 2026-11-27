import { test, expect } from 'bun:test';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { harness } from './harness';
const root = join(import.meta.dir, '..');
test('browser-target bundle executes in a bare V8 context with no Node/DOM globals', async () => {
  await mkdir(join(root, '.test-build'), { recursive: true });
  const entry = join(root, '.test-build', 'realm.ts');
  await Bun.write(
    entry,
    `import { RemoteDOM } from '../src/shim'; globalThis.RemoteDOM = RemoteDOM;`
  );
  const built = await Bun.build({ entrypoints: [entry], target: 'browser', format: 'iife' });
  expect(built.success).toBe(true);
  const source = await built.outputs[0].text();
  const probe = `import vm from 'node:vm'; import { readFileSync } from 'node:fs';
    const source = readFileSync(0, 'utf8');
    const context = vm.createContext({ queueMicrotask });
    vm.runInContext(source, context);
    const result = vm.runInContext(\`(() => {
      if (typeof process !== 'undefined' || typeof require !== 'undefined' || typeof document !== 'undefined') throw Error('Leaked host global');
      const batches = []; const errors = [];
      const shim = new RemoteDOM({ realmGlobal: globalThis, session: {panel: 'bare-v8', generation: 1}, send: batch => { batches.push(batch); return true; }, reportError: e => errors.push(String(e)) });
      shim.installGlobals(); if (window !== globalThis || window.document !== document) throw Error('Global alias mismatch');
      const d = shim.document; const t = d.createElement('template'); t.innerHTML = '<section><input><output>Ready</output></section>';
      const panel = t.content.firstChild.cloneNode(true); d.body.append(panel); shim.flush();
      const input = panel.querySelector('input'); const output = panel.querySelector('output');
      input.addEventListener('input', () => output.textContent = input.value); shim.flush();
      shim.receive({ kind: 'event', session: shim.session, seq: 1, applied: 2, target: shim.id(input), event: 'input', bubbles: true, cancelable: false, default_prevented: false, data: {state: {value: 'V8 typed'}} });
      shim.flush(); const result = { text: output.textContent, errors, batches: batches.length }; shim.dispose(); return result;
    })()\`, context);
    console.log(JSON.stringify({ ...result, engine: process.versions.v8 }));`;
  const child = Bun.spawnSync(['node', '--input-type=module', '-e', probe], {
    stdin: Buffer.from(source),
    cwd: root
  });
  expect(child.exitCode).toBe(0);
  if (child.exitCode) throw new Error(child.stderr.toString());
  const result = JSON.parse(child.stdout.toString());
  expect(result.text).toBe('V8 typed');
  expect(result.errors).toEqual([]);
  expect(result.batches).toBe(3);
  expect(result.engine).toBeTruthy();
});
test('real JS mutation and page-feedback JSON roundtrip through the Rust admission router', async () => {
  const h = harness(),
    d = h.shim.document;
  const input = d.createElement('input');
  input.value = 'Name';
  input.setAttribute('aria-label', 'Name');
  input.addEventListener('input', () => {});
  d.body.append(input);
  input.focus({ preventScroll: true });
  input.setSelectionRange(1, 3);
  input.scrollTo(0, 10);
  h.shim.measure(input, ['color']);
  h.shim.flush();
  await h.settle();
  const batch = h.batches[0],
    feedback = h.messages.filter((message) => message.kind !== 'event');
  const child = Bun.spawnSync(
    process.env.PM_REMOTE_DOM_WIRE_BIN
      ? [process.env.PM_REMOTE_DOM_WIRE_BIN]
      : [
          'flock',
          '/tmp/pm-build.lock',
          'cargo',
          'run',
          '--quiet',
          '--locked',
          '--manifest-path',
          join(root, '../../crates/remote-dom/standalone/Cargo.toml'),
          '--example',
          'wire'
        ],
    {
      stdin: Buffer.from(JSON.stringify({ batch, feedback })),
      cwd: root
    }
  );
  expect(child.exitCode).toBe(0);
  if (child.exitCode) throw new Error(child.stderr.toString());
  const result = JSON.parse(child.stdout.toString());
  expect(result.batch).toEqual(batch);
  expect(result.feedback).toEqual(feedback);
  expect(result.sent).toBe(1);
  h.close();
});
