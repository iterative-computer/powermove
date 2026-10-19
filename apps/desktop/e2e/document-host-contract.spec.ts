import { tmpdir } from 'node:os';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { runContractOnNativeHost } from '../src/server/native/contract-run';
/*
 * Desktop vs document host worker (contract 15, invariant 8; plan section
 * 22's gate): the agent-tool contract sequence from the same document, in
 * the editor's own harness and in a worker process, must leave
 * byte-identical canonical documents and identical answers, once the ids
 * minted during the run are renamed by order of appearance
 * (contract-run.ts normalizeMintedIds: ids are random on every host).
 */
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { CONTRACT_SEQUENCE, deterministicSeam } from '../src/engine/document-host/contract';
import { answerOf, documentBytes, contractProject, normalizeMintedIds, runContractOnWorker, workspaceDocumentFields, type ContractTranscript } from '../src/engine/document-host/contract-run';
import { BRIDGE, WASM, buildEngineBundle } from '../src/engine/document-host/test-host';

test.skip(!existsSync(WASM) || !existsSync(BRIDGE), 'pm-wasm and pm-bridge must be built');

test('the agent-tool contract leaves byte-identical documents on the desktop and in a document host worker', async ({ session }) => {
  test.setTimeout(240_000);
  await session.openEditor();
  const page = session.page;
  await page.waitForFunction(() => Boolean((window as any).PM?.AgentHarness?.test?.answerLiveAgentTool));
  // The worker's host lists the same extensions the desktop has.
  await page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.loader));
  const extensions = await page.evaluate(async () => {
    const loader = (window as any).PM.Kernel.loader;
    await loader.whenIdle();
    return JSON.parse(JSON.stringify(loader.records()));
  });

  await page.evaluate((project) => {
    const PM = (window as any).PM;
    PM.hist.clear();
    PM.replaceProject(project, {});
    PM.hist.clear();
  }, contractProject());
  await page.evaluate(deterministicSeam);
  const desktop = await page.evaluate(async (sequence) => {
    const PM = (window as any).PM;
    let revision = PM.proj.revision || 0;
    const answers: unknown[] = [];
    const documents: string[] = [];
    const rasters: Record<string, unknown> = {};
    const raster = PM.raster;
    // Canonical key serialization only; values retain Canvas's exact numbers.
    const sort = (v: any): any => Array.isArray(v) ? v.map(sort) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map(k => [k, sort(v[k])])) : v;
    PM.raster = (layer: any, scale: number, t: number) => {
      const result = raster(layer, scale, t);
      if (layer.type === 'text' && scale === 1 && !PM.EngineDoc?.api) rasters[JSON.stringify(sort({ d: layer.d, p: layer.p, t }))] = result ? JSON.parse(JSON.stringify({ selection: result.selection, fontOffset: result.fontOffset })) : null;
      return result;
    };
    // Native edits do not call Canvas. In the test only, sample the old
    // raster at each ink-kernel input before the native edit changes it.
    // Disabling the query adapter for this reference read keeps it independent.
    const edits = PM.EngineDoc?.edits;
    const originalEdit = edits?.edit;
    const originalAnswer = edits?.answer;
    const sample = (name: string, request: any) => {
      if (!['captureFontAnchor', 'anchorReference'].includes(name)) return;
      const layer = PM.proj.layers[request.layer];
      if (layer?.type !== 'text') return;
      const api = PM.EngineDoc.api;
      PM.EngineDoc.api = null;
      try { PM.raster(layer.d.animators?.length ? { ...layer, d: { ...layer.d, animators: [] } } : layer, 1, request.t); }
      finally { PM.EngineDoc.api = api; }
    };
    if (edits) {
      edits.edit = function (name: string, request: any, ...rest: any[]) { sample(name, request); return originalEdit.call(this, name, request, ...rest); };
      edits.answer = function (name: string, request: any, ...rest: any[]) { sample(name, request); return originalAnswer.call(this, name, request, ...rest); };
    }
    for (const [index, call] of sequence.entries()) {
      // The answer as the host receives it (errors included), as a worker gives it.
      const answer = await PM.AgentHarness.test.answerLiveAgentTool({ runId: call.runId, callId: `contract-${index}`, tool: call.tool, arguments: call.arguments, baseRevision: revision });
      if (typeof answer?.revision === 'number') revision = answer.revision;
      answers.push(JSON.parse(JSON.stringify(answer)));
      documents.push(JSON.stringify(PM.proj));
    }
    const workspace = await PM.AgentHarness.test.answerLiveAgentTool({ runId: 'run-w', callId: 'contract-workspace', tool: 'get_workspace_state', arguments: {}, baseRevision: revision });
    PM.raster = raster;
    if (edits) { edits.edit = originalEdit; edits.answer = originalAnswer; }
    return { rasters, answers, documents, document: JSON.stringify(PM.proj), workspace: JSON.parse(JSON.stringify(workspace)) };
  }, CONTRACT_SEQUENCE);

  // The deterministic seam and live-byte statistics are test-mode only.
  const prebuilt = process.env['POWERMOVE_CONTRACT_ENGINE_SCRIPT'];
  const bundle = prebuilt ? { script: prebuilt, dispose() {} } : await buildEngineBundle();
  let worker: ContractTranscript;
  let hidden: ContractTranscript | undefined;
  try {
    worker = await runContractOnWorker(bundle.script, CONTRACT_SEQUENCE, extensions, desktop.rasters);
    if (process.env['POWERMOVE_NATIVE_HOST_DIST']) hidden = await runContractOnNativeHost(process.env['POWERMOVE_NATIVE_HOST_DIST']!, bundle.script, extensions, desktop.rasters);
  }
  finally { bundle.dispose(); }

  const desktopTranscript = normalizeMintedIds({ answers: desktop.answers.map((a) => answerOf(a as never)), document: documentBytes(JSON.parse(desktop.document)), documents: desktop.documents, workspace: workspaceDocumentFields(answerOf(desktop.workspace as never)), workspaceRaw: JSON.parse((desktop.workspace as any).content[0].text) }, contractProject());
  const workerTranscript = normalizeMintedIds(worker, contractProject());
  const desktopAnswers = desktopTranscript.answers;
  // Both transcripts, for a side-by-side when they differ (outside the repository).
  const evidence = process.env['POWERMOVE_CONTRACT_EVIDENCE'];
  if (evidence) {
    mkdirSync(evidence, { recursive: true });
    writeFileSync(path.join(evidence, 'desktop.json'), JSON.stringify(desktopTranscript, null, 2));
    writeFileSync(path.join(evidence, 'worker.json'), JSON.stringify(workerTranscript, null, 2));
  }
  for (const [index, call] of CONTRACT_SEQUENCE.entries()) {
    expect(desktopAnswers[index], `${index}: ${call.tool} on the desktop`).toMatchObject({ ok: !call.fails });
    expect(workerTranscript.answers[index], `${index}: ${call.tool} in the worker`).toEqual(desktopAnswers[index]);
    expect(workerTranscript.documents?.[index], `${index}: ${call.tool} live document bytes`).toBe(desktopTranscript.documents?.[index]);
  }
  expect(workerTranscript.document).toBe(desktopTranscript.document);
  if (hidden) {
    const native = normalizeMintedIds(hidden, contractProject());
    for (const [index, call] of CONTRACT_SEQUENCE.entries()) {
      expect(native.answers[index], `${index}: ${call.tool} through the native hidden host`).toEqual(desktopAnswers[index]);
      expect(native.documents?.[index], `${index}: ${call.tool} native hidden document bytes`).toBe(desktopTranscript.documents?.[index]);
    }
    expect(native.document).toBe(desktopTranscript.document);
  }
  // get_workspace_state: what it reports of the document and the kernel is the same on both.
  expect(workerTranscript.workspace).toEqual(desktopTranscript.workspace);
});

// This exercises the complete no-window MCP answer rather than the worker's
// document-only workspace projection. The harness uses real isolated profiles,
// a persisted user panel and the full built-in extension registry.
test('the native no-window workspace bytes match the existing hidden host including every panel', async () => {
  test.skip(!process.env['POWERMOVE_NATIVE_HOST_DIST'], 'Build the native CLI host and set POWERMOVE_NATIVE_HOST_DIST.');
  test.setTimeout(240_000);
  const evidence = process.env['POWERMOVE_NATIVE_PARITY_EVIDENCE'] ?? mkdtempSync(path.join(tmpdir(), 'pm-workspace-contract-'));
  const { stdout } = await promisify(execFile)(process.env['POWERMOVE_TEST_NODE'] ?? process.execPath,
    [path.resolve(__dirname, '../scripts/test-native-hidden-parity.mjs')], {
      env: { ...process.env, POWERMOVE_NATIVE_PARITY_EVIDENCE: evidence }, timeout: 225_000, maxBuffer: 4 * 1024 * 1024
    });
  console.log(stdout);
  const expected = readFileSync(path.join(evidence!, 'workspace-legacy.json'));
  const actual = readFileSync(path.join(evidence!, 'workspace-native.json'));
  expect(actual).toEqual(expected);
  const result = JSON.parse(readFileSync(path.join(evidence!, 'result.json'), 'utf8'));
  expect(result.workspaceBytesEqual).toBe(true);
  expect(result.userPanel).toBe(true);
  expect(result.completeRegistry).toBe(true);
});
