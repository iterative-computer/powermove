/** Opt-in CDP timeline for the source-performance workload; writes only to its evidence directory. */
import { createWriteStream } from 'node:fs';
import { finished } from 'node:stream/promises';
import type { CDPSession } from 'playwright';

const MAX_TRACE_BYTES = 256 * 1024 * 1024;

export async function startSourceTimeline(cdp: CDPSession, destination: string): Promise<() => Promise<{ events: number; bytes: number; truncated: boolean }>> {
  const stream = createWriteStream(destination, { mode: 0o600 });
  stream.on('error', () => {}); // finished() reports the error after Tracing.end.
  stream.write('{"traceEvents":[');
  let events = 0, bytes = 0, truncated = false;
  const onData = ({ value }: { value: unknown[] }) => {
    if (!value.length || truncated) return;
    const chunk = value.map(event => JSON.stringify(event)).join(',');
    if (bytes + Buffer.byteLength(chunk) > MAX_TRACE_BYTES) { truncated = true; return; }
    stream.write(`${events ? ',' : ''}${chunk}`);
    events += value.length;
    bytes += Buffer.byteLength(chunk);
  };
  cdp.on('Tracing.dataCollected', onData);
  try {
    await cdp.send('Tracing.start', {
      transferMode: 'ReportEvents',
      traceConfig: {
        includedCategories: [
          'toplevel', 'devtools.timeline', 'disabled-by-default-devtools.timeline',
          'disabled-by-default-devtools.timeline.frame', 'disabled-by-default-devtools.timeline.stack', 'blink.user_timing',
          'blink', 'cc', 'gpu', 'viz', 'graphics.pipeline', 'v8', 'disabled-by-default-v8.gc', '__metadata'
        ],
        recordMode: 'recordAsMuchAsPossible'
      }
    } as any);
  } catch (error) {
    cdp.off('Tracing.dataCollected', onData);
    stream.end(']}');
    await finished(stream).catch(() => {});
    throw error;
  }
  return async () => {
    let timeout: ReturnType<typeof setTimeout> | null = null;
    let done = () => {};
    try {
      const complete = new Promise<void>(resolve => {
        done = resolve;
        cdp.once('Tracing.tracingComplete', done);
      });
      await cdp.send('Tracing.end');
      await Promise.race([complete, new Promise<void>((_, reject) => {
        timeout = setTimeout(() => reject(new Error('The source CDP trace did not complete within 30 seconds.')), 30_000);
      })]);
    } finally {
      if (timeout) clearTimeout(timeout);
      cdp.off('Tracing.tracingComplete', done);
      cdp.off('Tracing.dataCollected', onData);
      stream.end(`],"truncated":${truncated}}`);
      await finished(stream);
    }
    return { events, bytes, truncated };
  };
}
