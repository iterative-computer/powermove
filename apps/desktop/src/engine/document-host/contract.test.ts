/*
 * The contract sequence runs cleanly in a document host worker and is
 * deterministic there (two workers, same bytes). The desktop comparison is
 * e2e/document-host-contract.spec.ts.
 */
import { existsSync } from 'node:fs';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { CONTRACT_SEQUENCE } from './contract';
import { runContractOnWorker } from './contract-run';
import { BRIDGE, WASM, buildEngineBundle } from './test-host';

let bundle: { script: string; dispose(): void } | null = null;
beforeAll(async () => { bundle = await buildEngineBundle(); }, 180_000);
afterAll(() => bundle?.dispose());

describe.skipIf(!existsSync(WASM) || !existsSync(BRIDGE))('the agent-tool contract sequence in a worker', () => {
  it('answers every call and leaves the same bytes on every run', async () => {
    const first = await runContractOnWorker(bundle!.script);
    for (const [index, answer] of first.answers.entries()) {
      expect([CONTRACT_SEQUENCE[index]!.tool, answer.ok, answer.error ?? null]).toEqual([CONTRACT_SEQUENCE[index]!.tool, true, null]);
    }
    const second = await runContractOnWorker(bundle!.script);
    expect(second.document).toBe(first.document);
    expect(second.answers).toEqual(first.answers);
    expect(first.document).toContain('Powermove engine');
  }, 180_000);
});
