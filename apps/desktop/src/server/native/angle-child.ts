/** Sidecar for direct pm-cli use. No Chromium until a shader request arrives. */
import path from 'node:path';
import { angleService } from './angle';
const directory = process.argv[2];
if (!directory || !path.isAbsolute(directory)) throw new Error('ANGLE sidecar requires its distribution directory');
void (async () => {
  const service = await angleService(directory, process.env);
  process.stdout.write(JSON.stringify(service.env) + '\n');
  let closing = false;
  const close = () => { if (closing) return; closing = true; void service.close().finally(() => process.exit(0)); };
  process.stdin.resume(); process.stdin.once('end', close);
  process.once('SIGTERM', close); process.once('SIGINT', close);
})().catch(error => { process.stderr.write(`angle-runner-unavailable: ${String(error)}\n`); process.exitCode = 1; });
