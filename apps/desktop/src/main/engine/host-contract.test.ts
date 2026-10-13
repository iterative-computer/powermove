// The shared host-services contract sequence (contract 14 invariant 9),
// also run by pm-host's CLI implementation (engine/crates/pm-host/tests/contract.rs).
import { mkdtemp, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import type { EngineProcess, Request } from '../../shared/engine-protocol';
import { ERROR_CODES } from './protocol';
import { HostServicesCore } from './services';

const FIXTURE = path.resolve(__dirname, '../../../../../engine/crates/pm-host/tests/fixtures/host-contract.json');

type Step = Record<string, unknown> & { op: string };

function substitute(value: unknown, vars: Map<string, unknown>): unknown {
  if (typeof value === 'string' && value.startsWith('$')) {
    if (!vars.has(value.slice(1))) throw new Error(`unknown variable ${value}`);
    return vars.get(value.slice(1));
  }
  if (Array.isArray(value)) return value.map((v) => substitute(v, vars));
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, substitute(v, vars)]));
  return value;
}

describe('host services contract', () => {
  let root: string;
  let steps: Step[];
  beforeAll(async () => {
    root = await mkdtemp(path.join(os.tmpdir(), 'pm-host-contract-'));
    steps = (JSON.parse(await readFile(FIXTURE, 'utf8')) as { steps: Step[] }).steps;
  });
  afterAll(async () => {
    await rm(root, { recursive: true, force: true });
  });

  it('serves the shared sequence like every other host', async () => {
    const services = new HostServicesCore({
      cacheDirectory: path.join(root, 'cache'),
      automaticOutputDirectory: path.join(root, 'Powermove'),
      physicalMemory: 0,
      globalMemory: 10_000
    });
    const vars = new Map<string, unknown>();
    const relative = (p: string) => path.relative(root, p).split(path.sep).join('/');
    for (const [index, raw] of steps.entries()) {
      const step = substitute(raw, vars) as Step;
      const where = `step ${index} (${step.op}${step.op === 'request' ? ` ${(step['request'] as Request).type}` : ''})`;
      switch (step.op) {
        case 'request': {
          const expected = step['expect'] as { response?: string; error?: string; fields?: Record<string, unknown>; pathEndsWith?: string; pathMatches?: string; mode?: number };
          let response: Record<string, unknown> | null = null;
          let code: string | null = null;
          try {
            response = await services.serve(step['process'] as EngineProcess, step['port'] as number, step['request'] as Request, step['session'] as number | null) as unknown as Record<string, unknown>;
          } catch (error) {
            const c = (error as { code?: unknown }).code;
            code = typeof c === 'string' && ERROR_CODES.has(c as never) ? c : 'internal';
          }
          if (expected.error) {
            expect(code, `${where}: ${JSON.stringify(response)}`).toBe(expected.error);
            break;
          }
          expect(response?.['type'], `${where}: error ${code}`).toBe(expected.response);
          for (const [field, value] of Object.entries(expected.fields ?? {})) expect(response![field], `${where} ${field}`).toEqual(value);
          if (expected.pathEndsWith) expect(relative(response!['path'] as string).endsWith(expected.pathEndsWith), `${where} ${String(response!['path'])}`).toBe(true);
          if (expected.pathMatches) expect(relative(response!['path'] as string)).toMatch(new RegExp(expected.pathMatches));
          if (expected.mode !== undefined) expect((await stat(response!['path'] as string)).mode & 0o777).toBe(expected.mode);
          for (const [name, field] of Object.entries((step['save'] ?? {}) as Record<string, string>)) vars.set(name, response![field]);
          break;
        }
        case 'write':
          await writeFile(step['path'] as string, Buffer.alloc(step['bytes'] as number, 1));
          break;
        case 'exists':
          expect(await stat(step['path'] as string).then(() => true, () => false), where).toBe(step['expect']);
          break;
        case 'initialLeases':
          services.memory.initial('interactive');
          services.memory.initial('job');
          break;
        case 'endLaunch':
          expect(await services.endLaunch(step['process'] as EngineProcess, step['port'] as number), where).toBe(step['expectRemoved']);
          break;
        case 'listOutputs':
          expect((await readdir(path.join(root, 'Powermove'))).sort(), where).toEqual(step['expect']);
          break;
        default:
          throw new Error(`unknown op ${step.op}`);
      }
    }
  });
});
