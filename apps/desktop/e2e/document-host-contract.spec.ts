/*
 * Desktop vs document host worker (contract 15, invariant 8; plan section
 * 22's gate): the agent-tool contract sequence from the same document, in
 * the editor's own harness and in a worker process, must leave
 * byte-identical canonical documents and identical answers.
 */
import { existsSync } from 'node:fs';
import { expect, test } from './helpers/app';
import { CONTRACT_SEQUENCE, deterministicSeam } from '../src/engine/document-host/contract';
import { answerOf, canonicalDocument, contractProject, runContractOnWorker, type ContractTranscript } from '../src/engine/document-host/contract-run';
import { BRIDGE, WASM, buildEngineBundle } from '../src/engine/document-host/test-host';

test.skip(!existsSync(WASM) || !existsSync(BRIDGE), 'pm-wasm and pm-bridge must be built');

test('the agent-tool contract leaves byte-identical documents on the desktop and in a document host worker', async ({ session }) => {
  test.setTimeout(240_000);
  const bundle = await buildEngineBundle();
  let worker: ContractTranscript;
  try {
    worker = await runContractOnWorker(bundle.script);
  } finally {
    bundle.dispose();
  }

  await session.openEditor();
  const page = session.page;
  await page.waitForFunction(() => Boolean((window as any).PM?.AgentHarness?.test?.handleLiveAgentTool));
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
      const answer = await PM.AgentHarness.test.handleLiveAgentTool({ runId: call.runId, callId: `contract-${index}`, tool: call.tool, arguments: call.arguments, baseRevision: revision });
      if (typeof answer?.revision === 'number') revision = answer.revision;
      answers.push(JSON.parse(JSON.stringify(answer)));
    }
    return { answers, document: JSON.stringify(PM.proj) };
  }, CONTRACT_SEQUENCE);

  const desktopAnswers = desktop.answers.map((a) => answerOf(a as never));
  for (const [index, call] of CONTRACT_SEQUENCE.entries()) {
    expect(desktopAnswers[index], `${index}: ${call.tool} on the desktop`).toMatchObject({ ok: true });
    expect(worker.answers[index], `${index}: ${call.tool} in the worker`).toEqual(desktopAnswers[index]);
  }
  expect(worker.document).toBe(canonicalDocument(JSON.parse(desktop.document)));
});
