/*
 * Desktop vs document host worker (contract 15, invariant 8; plan section
 * 22's gate): the agent-tool contract sequence from the same document, in
 * the editor's own harness and in a worker process, must leave
 * byte-identical canonical documents and identical answers, once the ids
 * minted during the run are renamed by order of appearance
 * (contract-run.ts normalizeMintedIds: ids are random on every host).
 */
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { CONTRACT_SEQUENCE, deterministicSeam } from '../src/engine/document-host/contract';
import { answerOf, canonicalDocument, contractProject, normalizeMintedIds, runContractOnWorker, workspaceDocumentFields, type ContractTranscript } from '../src/engine/document-host/contract-run';
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
  const bundle = await buildEngineBundle();
  let worker: ContractTranscript;
  try {
    worker = await runContractOnWorker(bundle.script, CONTRACT_SEQUENCE, extensions);
  } finally {
    bundle.dispose();
  }

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
    for (const [index, call] of sequence.entries()) {
      // The answer as the host receives it (errors included), as a worker gives it.
      const answer = await PM.AgentHarness.test.answerLiveAgentTool({ runId: call.runId, callId: `contract-${index}`, tool: call.tool, arguments: call.arguments, baseRevision: revision });
      if (typeof answer?.revision === 'number') revision = answer.revision;
      answers.push(JSON.parse(JSON.stringify(answer)));
    }
    const workspace = await PM.AgentHarness.test.answerLiveAgentTool({ runId: 'run-w', callId: 'contract-workspace', tool: 'get_workspace_state', arguments: {}, baseRevision: revision });
    return { answers, document: JSON.stringify(PM.proj), workspace: JSON.parse(JSON.stringify(workspace)) };
  }, CONTRACT_SEQUENCE);

  const desktopTranscript = normalizeMintedIds({ answers: desktop.answers.map((a) => answerOf(a as never)), document: canonicalDocument(JSON.parse(desktop.document)), workspace: workspaceDocumentFields(answerOf(desktop.workspace as never)) }, contractProject());
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
  }
  expect(workerTranscript.document).toBe(desktopTranscript.document);
  // get_workspace_state: what it reports of the document and the kernel is the same on both.
  expect(workerTranscript.workspace).toEqual(desktopTranscript.workspace);
});
