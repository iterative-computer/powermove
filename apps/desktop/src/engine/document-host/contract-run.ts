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

export const canonicalDocument = (doc: unknown): string => canonicalStringify(canonicalProject(doc as never));

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
