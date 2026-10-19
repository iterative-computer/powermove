/** Native hidden-host half of the desktop document contract. Test support only. */
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { NativeHiddenHost } from './host';
import { createRequire } from 'node:module';
import { WebIpcMain } from '../clients';
import { ProjectSessions } from '../sessions';
import { createStore } from '../../main/storage';
import { WEB } from '../../shared/wire';
import { IPC, type AgentToolRequestEvent, type AgentToolResponseEvent } from '../../shared/ipc';
import { CONTRACT_PROJECT_ID, contractProject, answerOf, workspaceDocumentFields, type ContractTranscript } from '../../engine/document-host/contract-run';
import { CONTRACT_SEQUENCE } from '../../engine/document-host/contract';
import { BRIDGE, extensionHost } from '../../engine/document-host/test-host';
import type { ExtensionRecord } from '../../shared/extensions';

export async function runContractOnNativeHost(distDir: string, script: string, extensions: ExtensionRecord[], rasters: Record<string, unknown>): Promise<ContractTranscript> {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-native-contract-'));
  let host: NativeHiddenHost | undefined;
  try {
    const store = createStore(path.join(directory, 'store')); await store.load();
    await store.set(`project.${CONTRACT_PROJECT_ID}`, { v: 'test', proj: contractProject() });
    await store.set('projects', [{ id: CONTRACT_PROJECT_ID, name: 'Contract' }]); await store.flushAll();
    const sessions = new ProjectSessions({ read: key => { const raw = store.getSerialized?.(key); return raw == null ? null : JSON.parse(raw); }, set: (key, value) => { void store.set(key, value); } }, { channel: WEB.syncPatch, version: 'test' });
    const reference = path.join(directory, 'rasters.json'); await writeFile(reference, JSON.stringify(rasters));
    const unavailable = async (): Promise<never> => { throw new Error('This document contract does not ask for browser pixels.'); };
    const { NativeHiddenHost: Host } = createRequire(__filename)(path.join(distDir, 'native-host.cjs')) as { NativeHiddenHost: typeof NativeHiddenHost };
    const ipc = new WebIpcMain();
    host = new Host({ userData: directory, exportsDir: path.join(directory, 'exports'), distDir,
      rendererDir: path.join(distDir, 'renderer'), ipc, store, sessions, extensions: extensionHost(extensions).service,
      documentScript: script, documentEnv: { POWERMOVE_PM_BRIDGE: BRIDGE, POWERMOVE_DOCUMENT_HOST_TEST_SEED: '1', POWERMOVE_NATIVE_EVALUATOR: 'audit', POWERMOVE_DOCUMENT_HOST_TEXT_REFERENCE: reference },
      browser: { call: unavailable, shader: unavailable, compat: unavailable, close: async () => {} }, log: () => {} });
    await host.start();
    const call = (request: AgentToolRequestEvent): Promise<AgentToolResponseEvent> => new Promise((resolve, reject) => {
      const timer = setTimeout(() => { ipc.off(IPC.agentToolResponse, receive); reject(new Error(`Native host timed out: ${request.tool}`)); }, 60000);
      const receive = (_event: unknown, value: unknown) => {
        const response = value as AgentToolResponseEvent;
        if (response.callId !== request.callId) return;
        clearTimeout(timer); ipc.off(IPC.agentToolResponse, receive); resolve(response);
      };
      ipc.on(IPC.agentToolResponse, receive);
      host!.owner(CONTRACT_PROJECT_ID).send(IPC.agentToolRequest, request);
    });
    let revision = 0;
    const answers: ContractTranscript['answers'] = [], documents: string[] = [];
    for (const [index, call] of CONTRACT_SEQUENCE.entries()) {
      const response = await call({ runId: call.runId, callId: `native-contract-${index}`, tool: call.tool, arguments: call.arguments, baseRevision: revision });
      revision = response.revision ?? revision; answers.push(answerOf(response));
      const stats = await host.pool.stats(CONTRACT_PROJECT_ID);
      if (!stats.testDocument) throw new Error('The contract worker did not expose its live document bytes'); documents.push(stats.testDocument);
    }
    const workspace = await host.pool.call(CONTRACT_PROJECT_ID, { runId: 'run-w', callId: 'native-contract-workspace', tool: 'get_workspace_state', arguments: {}, baseRevision: revision });
    const stats = await host.pool.stats(CONTRACT_PROJECT_ID);
    if (Object.keys(stats.fallbacks).length || stats.failures.length || stats.dual?.mismatched) throw new Error(`Native host contract failed: ${JSON.stringify(stats)}`);
    return { answers, documents, document: stats.testDocument!, workspace: workspaceDocumentFields(workspace), workspaceRaw: JSON.parse((workspace.content[0] as { text: string }).text) };
  } finally { await host?.close(); await rm(directory, { recursive: true, force: true }); }
}
