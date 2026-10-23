import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { build } from 'esbuild';
import { expect, it } from 'vitest';

it('keeping one parsed string does not retain the discarded document', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'powermove-parse-memory-'));
  try {
    const output = path.join(directory, 'parser.mjs');
    await build({ entryPoints: [fileURLToPath(new URL('./parse-async.ts', import.meta.url))], outfile: output, bundle: true, platform: 'node', format: 'esm', logLevel: 'silent' });
    // Use a fresh heap and drain the async parser frame before collecting it;
    // otherwise the fulfilled parse promise itself retains the whole result.
    for (const kind of ['large-string', 'small-record']) {
      const script = `
        import { parseAsync } from ${JSON.stringify(pathToFileURL(output).href)};
        global.gc(); const before = process.memoryUsage().heapUsed;
        async function retain() {
          const keep = 'k'.repeat(${kind === 'large-string' ? 128 * 1024 : 1024});
          const source = JSON.stringify(${kind === 'large-string' ? '{ keep, discard: "d".repeat(16 * 1024 * 1024) }' : '{ record: { keep }, discard: "d".repeat(16 * 1024 * 1024) }'});
          const parsed = await parseAsync(source, async () => {});
          return ${kind === 'large-string' ? 'parsed.keep' : 'parsed.record.keep'};
        }
        const kept = await retain(); await new Promise(setImmediate); global.gc();
        console.log(JSON.stringify({ bytes: process.memoryUsage().heapUsed - before, length: kept.length, last: kept.charCodeAt(kept.length - 1) }));
      `;
      const result = JSON.parse(execFileSync(process.execPath, ['--expose-gc', '--input-type=module', '-e', script], { encoding: 'utf8' }));
      expect(result.length).toBe(kind === 'large-string' ? 128 * 1024 : 1024);
      expect(result.last).toBe('k'.charCodeAt(0));
      expect(result.bytes).toBeLessThan(4 * 1024 * 1024);
    }
  } finally { await rm(directory, { recursive: true, force: true }); }
});
