/*
 * The contract sequence runs cleanly in a document host worker and is
 * deterministic there (two workers, same bytes). The desktop comparison is
 * e2e/document-host-contract.spec.ts.
 */
import { existsSync, readFileSync } from 'node:fs';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { CONTRACT_SEQUENCE, CONTRACT_EXCLUSIONS } from './contract';
import { runContractOnWorker, normalizeMintedIds } from './contract-run';
import { BRIDGE, WASM, buildEngineBundle } from './test-host';

import { POWERMOVE_AGENT_TOOLS } from '../../main/agent-tools/spec';
import { DOM_TOOLS, JOB_TOOLS } from './protocol';

let bundle: { script: string; dispose(): void } | null = null;
beforeAll(async () => { bundle = await buildEngineBundle(); }, 180_000);
afterAll(() => bundle?.dispose());

describe.skipIf(!existsSync(WASM) || !existsSync(BRIDGE))('the agent-tool contract sequence in a worker', () => {
  it('answers every call and leaves the same bytes on every run', async () => {
    const first = await runContractOnWorker(bundle!.script);
    for (const [index, answer] of first.answers.entries()) {
      const call = CONTRACT_SEQUENCE[index]!;
      if (call.fails) expect([index, call.tool, answer.ok, typeof answer.error]).toEqual([index, call.tool, false, 'string']);
      else expect([index, call.tool, answer.ok, answer.error ?? null]).toEqual([index, call.tool, true, null]);
    }
    const second = await runContractOnWorker(bundle!.script);
    expect(second.document).toBe(first.document);
    expect(second.documents).toEqual(first.documents);
    expect(second.answers).toEqual(first.answers);
    expect(first.document).toContain('Powermove engine');
  }, 180_000);
});


it('accounts for every document tool in the public spec and every harness dispatch', () => {
  const rust = readFileSync(new URL('../../../../../engine/crates/pm-protocol/src/document_host.rs', import.meta.url), 'utf8');
  const hostSection = rust.slice(rust.indexOf('pub const HOST_TOOLS'), rust.indexOf('pub const HOST_TOOLS') + 9000).split('];')[0]!;
  const routed = new Set<string>([...DOM_TOOLS, ...JOB_TOOLS, ...[...hostSection.matchAll(/"([a-z_]+)"/g)].map(m => m[1]!)]);
  const harness = readFileSync(new URL('../../renderer/src/legacy/assistant/harness.ts', import.meta.url), 'utf8');
  const names = new Set([...POWERMOVE_AGENT_TOOLS.map(t => t.name), ...[...harness.matchAll(/request.tool === '([^']+)'/g)].map(m => m[1]!), '__composition_info', 'check_project']);
  const covered = new Set(CONTRACT_SEQUENCE.map(c => c.tool));
  // Bridge-internal requests. `__validate_panel_target` is the panel tools' (DOM route) generation check, as `__panel_bounds`.
  const internal = new Set(['__panel_bounds', '__prepare_panel_input', '__validate_panel_target', '__agent_thread_control', '__mcp_export_video', '__mcp_save_project', '__engine_save_plan', '__media_source']);
  expect([...names].filter(n => !routed.has(n) && !internal.has(n) && !covered.has(n) && !CONTRACT_EXCLUSIONS[n]).sort()).toEqual([]);
  const actions = new Set(CONTRACT_SEQUENCE.filter(c => c.tool === 'manage_media').map(c => c.arguments.operation));
  expect([...actions].sort()).toEqual(['create_folder', 'delete', 'delete_folder', 'move', 'move_folder', 'rename', 'rename_folder']);
});


it('does not rename document values using unrelated window panel ids', () => {
  const document = JSON.stringify({ layers: [{ id: 'Lfixed', type: 'text', d: { align: 'center' } }], origin: 'agent' });
  const result = normalizeMintedIds({ answers: [], document, documents: [document], workspaceRaw: { registeredPanels: [{ id: 'text' }, { id: 'center' }, { id: 'agent' }] } }, JSON.parse(document));
  expect(result.document).toBe(document);
  expect(result.documents).toEqual([document]);
});


it('normalizes ids that occur only in an intermediate document before rollback', () => {
  const transcript = (id: string) => ({ answers: [], document: '{}', documents: [JSON.stringify({ edits: [{ id }] }), '{}'] });
  expect(normalizeMintedIds(transcript('editLeft'), {})).toEqual(normalizeMintedIds(transcript('editRight'), {}));
});
