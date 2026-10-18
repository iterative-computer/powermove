/*
 * Runs the contract sequence (contract.ts) against a document host pool and
 * returns what the desktop side is compared with: every answer (without the
 * call ids) and the final document in canonical form. Shared by the vitest
 * worker check and the Electron contract spec.
 */
import { canonicalProject, canonicalStringify } from '../../shared/canonical-json';
import type { AgentToolResponseEvent } from '../../shared/ipc';
import { CONTRACT_SEQUENCE, type ContractCall } from './contract';
import { DocumentHostPool } from './pool';
import { BRIDGE, FIXTURE, WASM, hostServices } from './test-host';

export interface ContractTranscript {
  answers: Array<Omit<AgentToolResponseEvent, 'runId' | 'callId'>>;
  document: string;
}

export const CONTRACT_PROJECT_ID = 'Pcontract';
export const contractProject = (): Record<string, unknown> => ({ ...structuredClone(FIXTURE), id: CONTRACT_PROJECT_ID, name: 'Contract' });

/** The answer as both hosts produce it, without the ids that route it. */
export function answerOf(response: AgentToolResponseEvent | Omit<AgentToolResponseEvent, 'runId' | 'callId'>): Omit<AgentToolResponseEvent, 'runId' | 'callId'> {
  const { runId: _r, callId: _c, ...rest } = response as AgentToolResponseEvent;
  return JSON.parse(JSON.stringify(rest));
}

export const canonicalDocument = (doc: unknown): string => canonicalStringify(canonicalProject(doc as never)) ?? '';

const ID_KEYS = new Set(['id', 'i', 'historyId']);

/** Every id string the value carries under an id key. */
function idsIn(value: unknown, out = new Set<string>()): Set<string> {
  if (Array.isArray(value)) { for (const v of value) idsIn(v, out); return out; }
  if (value && typeof value === 'object') {
    for (const [k, v] of Object.entries(value)) {
      if (ID_KEYS.has(k) && typeof v === 'string') out.add(v);
      else if (typeof v === 'string' && v.startsWith('{')) { try { idsIn(JSON.parse(v), out); } catch { /* not JSON */ } }
      else idsIn(v, out);
    }
  }
  return out;
}

/**
 * The transcript with every id minted during the run renamed by order of
 * first appearance (`#1`, `#2`, …), everywhere it occurs (answers' JSON
 * text included). Ids come from Math.random on every host, and the
 * desktop's own UI draws from the same generator between calls, so two
 * desktop runs differ the same way; everything else must be byte-identical.
 */
export function normalizeMintedIds(transcript: ContractTranscript, initial: unknown): ContractTranscript {
  const known = idsIn(initial);
  const serialized = JSON.stringify(transcript);
  const minted = [...idsIn(transcript)].filter((id) => !known.has(id) && id.length >= 3);
  minted.sort((a, b) => serialized.indexOf(a) - serialized.indexOf(b));
  let out = serialized;
  minted.forEach((id, n) => { out = out.split(id).join(`#${n + 1}`); });
  return JSON.parse(out);
}

export async function runContractOnWorker(script: string, sequence: ContractCall[] = CONTRACT_SEQUENCE): Promise<ContractTranscript> {
  const host = hostServices();
  host.seed(CONTRACT_PROJECT_ID, 'Contract', contractProject());
  const pool = new DocumentHostPool({
    services: host.services, newEngine: true, script,
    env: { POWERMOVE_PM_WASM: WASM, POWERMOVE_PM_BRIDGE: BRIDGE, POWERMOVE_DOCUMENT_HOST_TEST_SEED: '1' },
  });
  try {
    let revision = 0;
    const answers: ContractTranscript['answers'] = [];
    for (const [index, call] of sequence.entries()) {
      const response = await pool.call(CONTRACT_PROJECT_ID, { runId: call.runId, callId: `contract-${index}`, tool: call.tool, arguments: call.arguments, baseRevision: revision });
      if (typeof response.revision === 'number') revision = response.revision;
      answers.push(answerOf(response));
    }
    await pool.shutdown();
    return { answers, document: canonicalDocument(host.sessions.document(CONTRACT_PROJECT_ID)) };
  } finally {
    await pool.shutdown();
  }
}
