import { expect, it } from 'vitest';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { build } from 'esbuild';
import { NodeEngineTransport } from './ports';

it('buffers until start, delivers in order and closes both endpoints once', () => {
  const transport = new NodeEngineTransport(), { port1, port2 } = transport.channel();
  const seen: unknown[] = [], closes: string[] = [];
  port1.postMessage(1); port1.postMessage(2);
  port2.on('message', event => seen.push(event.data));
  expect(seen).toEqual([]); port2.start(); expect(seen).toEqual([1, 2]);
  port1.on('close', () => closes.push('a')); port2.on('close', () => closes.push('b'));
  port1.close(); port2.close(); port1.postMessage(3);
  expect(closes).toEqual(['a', 'b']); expect(seen).toEqual([1, 2]);
});

it('transfers queued binary messages over Node IPC and closes safely with messages in flight', async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-ports-'));
  let child: ReturnType<ReturnType<NodeEngineTransport['spawn']>> | undefined;
  try {
    const source = path.join(__dirname, 'ports.ts');
    await writeFile(path.join(dir, 'child.ts'), `import { childParent } from ${JSON.stringify(source)};
      const parent = childParent(); parent.on('message', event => { const control = event.ports![0]!;
        control.on('message', event => { const port = event.ports![0]; if (port) { port.on('message', e => port.postMessage(e.data)); port.start(); } else control.postMessage(event.data); }); control.start(); });`);
    await build({ entryPoints: [path.join(dir, 'child.ts')], outfile: path.join(dir, 'child.cjs'), bundle: true, platform: 'node', format: 'cjs' });
    const transport = new NodeEngineTransport(); child = transport.spawn(path.join(dir, 'child.cjs'), '', () => {})('job');
    const { port1, port2 } = transport.channel();
    // Queued before bootstrap and before start must still reach the child.
    port2.postMessage(new Uint8Array([1, 2, 255]));
    child.postMessage({ type: 'control' }, [port1]);
    const reply = new Promise<unknown>(resolve => { port2.on('message', e => resolve(e.data)); port2.start(); });
    expect(await reply).toEqual(new Uint8Array([1, 2, 255]));
    const nested = transport.channel();
    const nestedReply = new Promise<unknown>(resolve => { nested.port2.on('message', e => resolve(e.data)); nested.port2.start(); });
    port2.postMessage({}, [nested.port1]); nested.port2.postMessage(new Uint8Array([42]));
    expect(await nestedReply).toEqual(new Uint8Array([42]));
    nested.port2.close(); port2.postMessage('alive');
    const alive = new Promise<unknown>(resolve => port2.on('message', e => resolve(e.data)));
    expect(await alive).toBe('alive');
  } finally { if (child) { const exit = new Promise<void>(resolve => child!.on('exit', () => resolve())); child.kill(); await exit; } await rm(dir, { recursive: true, force: true }); }
}, 15_000);
