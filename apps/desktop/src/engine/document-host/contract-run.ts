/*
 * Runs the contract sequence (contract.ts) against a document host pool and
 * returns what the desktop side is compared with: every answer (without the
 * call ids) and the final document in canonical form. Shared by the vitest
 * worker check and the Electron contract spec.
 */
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { canonicalProject } from '../../shared/canonical-json';
import type { AgentToolResponseEvent } from '../../shared/ipc';
import { CONTRACT_SEQUENCE, WORKSPACE_DOCUMENT_FIELDS, type ContractCall } from './contract';
import type { ExtensionRecord } from '../../shared/extensions';
import { DocumentHostPool } from './pool';
import { BRIDGE, FIXTURE, WASM, extensionHost, hostServices } from './test-host';

export interface ContractTranscript {
  answers: Array<Omit<AgentToolResponseEvent, 'runId' | 'callId'>>;
  document: string;
  /** get_workspace_state after the sequence: its document fields (WORKSPACE_DOCUMENT_FIELDS). */
  workspace?: Record<string, unknown>;
}

/** The fields of a get_workspace_state answer that both hosts must give alike. */
export function workspaceDocumentFields(answer: Omit<AgentToolResponseEvent, 'runId' | 'callId'>): Record<string, unknown> {
  const state = JSON.parse(String((answer.content?.[0] as { text?: string } | undefined)?.text ?? '{}'));
  return Object.fromEntries(WORKSPACE_DOCUMENT_FIELDS.map((key) => [key, state[key]]));
}

export const CONTRACT_PROJECT_ID = 'Pcontract';
/**
 * The fixture plus a voice-over clip whose media the project references but
 * neither host has bytes for (media status, clip edits, captions errors).
 */
export const contractProject = (): Record<string, unknown> => ({
  ...structuredClone(FIXTURE), id: CONTRACT_PROJECT_ID, name: 'Contract',
  assetFolders: { Fsource: { id: 'Fsource', name: 'Source', parent: null }, Fparent: { id: 'Fparent', name: 'Parent', parent: null } },
  assets: { Avoice: { id: 'Avoice', kind: 'audio', name: 'Voice.wav', storageKey: 'media:contract-voice', size: 1024, dur: 6, mime: 'audio/wav' } },
  layers: [{
    id: 'Lvoice', type: 'audio', name: 'Voice', from: 0, dur: 6, color: '#7f8cff', on: true, lock: false, solo: false,
    p: {}, d: { asset: 'Avoice', trim: { v: 0, kf: [], expr: null }, volume: { v: 1, kf: [], expr: null } },
  }],
});

/** The answer as both hosts produce it, without the ids that route it. */
export function answerOf(response: AgentToolResponseEvent | Omit<AgentToolResponseEvent, 'runId' | 'callId'>): Omit<AgentToolResponseEvent, 'runId' | 'callId'> {
  const { runId: _r, callId: _c, ...rest } = response as AgentToolResponseEvent;
  return JSON.parse(JSON.stringify(rest));
}

export const canonicalDocument = (doc: unknown): string => JSON.stringify(canonicalProject(doc as never)) ?? '';

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
  // Ids from Math.random may be any letters, so every new id counts; only whole values are renamed.
  const minted = [...idsIn(transcript)].filter((id) => !known.has(id) && id.length >= 3);
  const names = new Map<string, string>();
  // Whole string values only (an id never matches inside other text), in
  // order of first appearance; JSON held in strings is renamed inside.
  const rename = (value: unknown): unknown => {
    if (typeof value === 'string') {
      if (minted.includes(value)) {
        if (!names.has(value)) names.set(value, `#${names.size + 1}`);
        return names.get(value);
      }
      if (value.startsWith('{') || value.startsWith('[')) {
        try { return JSON.stringify(rename(JSON.parse(value))); } catch { /* not JSON */ }
      }
      return value;
    }
    if (Array.isArray(value)) return value.map(rename);
    // A key is renamed only where it names its own entry (`comps: { [id]: { id } }`).
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k, v]) => [minted.includes(k) && (v as { id?: unknown } | null)?.id === k ? (names.get(k) ?? (names.set(k, `#${names.size + 1}`), names.get(k)!)) : k, rename(v)]));
    return value;
  };
  return rename(transcript) as ContractTranscript;
}

/** `extensions`: the host's extension list (the desktop's, in the Electron spec). */
export async function runContractOnWorker(script: string, sequence: ContractCall[] = CONTRACT_SEQUENCE, extensions?: ExtensionRecord[], rasters?: Record<string, unknown>): Promise<ContractTranscript> {
  const host = hostServices();
  host.seed(CONTRACT_PROJECT_ID, 'Contract', contractProject());
  if (extensions) host.services.extensions = extensionHost(extensions).service;
  const referenceDir = rasters ? mkdtempSync(path.join(tmpdir(), 'pm-ink-reference-')) : null;
  if (referenceDir) writeFileSync(path.join(referenceDir, 'rasters.json'), JSON.stringify(rasters));
  const pool = new DocumentHostPool({
    services: host.services, newEngine: true, script,
    env: { POWERMOVE_PM_WASM: WASM, POWERMOVE_PM_BRIDGE: BRIDGE, POWERMOVE_DOCUMENT_HOST_TEST_SEED: '1', ...(referenceDir ? { POWERMOVE_NATIVE_EVALUATOR: 'audit', POWERMOVE_DOCUMENT_HOST_TEXT_REFERENCE: path.join(referenceDir, 'rasters.json') } : {}) },
  });
  try {
    let revision = 0;
    const answers: ContractTranscript['answers'] = [];
    for (const [index, call] of sequence.entries()) {
      const response = await pool.call(CONTRACT_PROJECT_ID, { runId: call.runId, callId: `contract-${index}`, tool: call.tool, arguments: call.arguments, baseRevision: revision });
      if (typeof response.revision === 'number') revision = response.revision;
      answers.push(answerOf(response));
    }
    const workspace = workspaceDocumentFields(await pool.call(CONTRACT_PROJECT_ID, { runId: 'run-w', callId: 'contract-workspace', tool: 'get_workspace_state', arguments: {}, baseRevision: revision }));
    const stats = await pool.stats(CONTRACT_PROJECT_ID);
    if (referenceDir && (!stats.dual || !stats.dual.comparedOps?.['edit.captureFontAnchor'] || !stats.dual.comparedOps?.['edit.anchorReference'] || stats.dual.mismatched)) throw new Error(`Ink audit failed: ${JSON.stringify(stats)}`);
    if (Object.keys(stats.fallbacks).length || stats.failures.length) throw new Error(`Strict contract used fallback: ${JSON.stringify(stats)}`);
    await pool.shutdown();
    return { answers, document: canonicalDocument(host.sessions.document(CONTRACT_PROJECT_ID)), workspace };
  } finally {
    await pool.shutdown();
    if (referenceDir) rmSync(referenceDir, { recursive: true, force: true });
  }
}
